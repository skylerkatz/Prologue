import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import {
  defaultRangeExtractor,
  useVirtualizer,
  type Range,
} from "@tanstack/react-virtual";
import { errorText, getContextLines, getFileDiff } from "../ipc";
import { guardReason, type GuardReason } from "../diff/guards";
import { segmentLine } from "../diff/segments";
import { detectLang } from "../highlight/lang";
import { tokenizeLines, type LineTokens } from "../highlight/shiki";
import {
  buildRows,
  computeGaps,
  CONTEXT_CHUNK,
  estimateRowHeight,
  indexComments,
  initialFileState,
  lineNumber,
  lineSide,
  rowKey,
  type ComposerLocation,
  type FileViewState,
  type Row,
} from "../diff/rows";
import {
  commentTargets,
  nextCommentTarget,
  nextUnviewedPath,
} from "../diff/keyboardNav";
import type {
  AnchorStatus,
  Comment,
  CommentSide,
  CommentState,
  DiffLine,
  DiffSummary,
  FileReviewState,
  FileStatus,
  FileSummary,
  NewCommentInput,
  RepliesByRoot,
  DiffMode,
} from "../types";
import {
  CommentCard,
  CommentComposer,
  replyDraftKey,
  type DraftStore,
} from "./Comments";
import { Chevron } from "./Chevron";
import { useCopyPath } from "./useCopyPath";

/** Parallel `get_file_diff` calls; each recomputes the repo diff in Rust. */
const MAX_CONCURRENT_LOADS = 3;

/** Dedupe/cache keys tied to a file's content identity, so a refresh that
 * changes the file naturally invalidates its entries. `\u0000` cannot appear
 * in paths. */
const fileKey = (file: FileSummary): string =>
  `${file.path}\u0000${file.fingerprint}`;
const hunkKey = (file: FileSummary, hi: number): string =>
  `${fileKey(file)}\u0000${hi}`;
/** The `fileKey` prefix of a `hunkKey`. */
const hunkKeyFile = (key: string): string =>
  key.slice(0, key.lastIndexOf("\u0000"));

/** Reviewed files start collapsed; "changed since review" files start
 * expanded — they need re-reviewing. */
function initialStates(
  files: FileSummary[],
  reviewStates: ReadonlyMap<string, FileReviewState>,
): Map<string, FileViewState> {
  return new Map(
    files.map((f) => [
      f.path,
      initialFileState(reviewStates.get(f.path) !== "reviewed"),
    ]),
  );
}

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface DiffViewProps {
  repoPath: string;
  base: string;
  head: string;
  mode: DiffMode;
  /** Fetch file hunks with whitespace changes ignored (git `-w`); must match
   * the setting the summary was computed with. */
  ignoreWhitespace: boolean;
  summary: DiffSummary;
  /** Rendered inside the scroll pane above the file cards (the orphaned
   * bucket) so it scrolls out of frame with the diff. */
  topContent?: ReactNode;
  /** Bumping `nonce` scrolls the file's header into view. */
  scrollTarget: { path: string; nonce: number } | null;
  /** File- and line-level comments (review-level ones are ignored here). */
  comments: Comment[];
  /** Replies grouped under their thread root, for every thread. */
  replies: RepliesByRoot;
  /** Re-anchor outcome per line comment; "changed" flags the comment card. */
  anchorStatuses: ReadonlyMap<number, AnchorStatus>;
  /** Per-file reviewed state; reviewed files mount collapsed and "changed"
   * ones carry a badge. Absent = never marked. */
  reviewStates: ReadonlyMap<string, FileReviewState>;
  onToggleReviewed: (path: string) => void;
  onCreateComment: (input: NewCommentInput) => Promise<void>;
  onUpdateComment: (id: number, body: string) => Promise<void>;
  onDeleteComment: (id: number) => Promise<void>;
  onSetCommentState: (id: number, state: CommentState) => Promise<void>;
}

type ExpandDirection = "top" | "bottom" | "all";

/** A gutter-drag / shift-click line selection, confined to one hunk + side.
 * Keyed by file path so it can survive refreshes of other files. */
interface LineSelection {
  path: string;
  hi: number;
  side: CommentSide;
  start: number;
  end: number;
}

export function DiffView({
  repoPath,
  base,
  head,
  mode,
  ignoreWhitespace,
  summary,
  topContent,
  scrollTarget,
  comments,
  replies,
  anchorStatuses,
  reviewStates,
  onToggleReviewed,
  onCreateComment,
  onUpdateComment,
  onDeleteComment,
  onSetCommentState,
}: DiffViewProps) {
  // Keyed by file PATH, not display index, and reconciled across refreshes
  // (see the block below) instead of remounting — so scroll position,
  // expansions, and the keyboard cursor survive a watcher-driven refresh.
  const [states, setStates] = useState<ReadonlyMap<string, FileViewState>>(
    () => initialStates(summary.files, reviewStates),
  );
  // Syntax tokens per visible hunk, keyed by `hunkKey`; rows render plain
  // until their hunk's entry appears, so highlighting never gates paint.
  const [highlights, setHighlights] = useState<
    ReadonlyMap<string, LineTokens[]>
  >(new Map());
  const highlightRequested = useRef(new Set<string>());
  const [selection, setSelection] = useState<LineSelection | null>(null);
  const [composer, setComposer] = useState<ComposerLocation | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  // Keyboard cursor: the file whose header j/k landed on. A ref mirror so
  // the window keydown handler always sees the latest position.
  const [cursorPath, setCursorPath] = useState<string | null>(null);
  const cursorPathRef = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  // Dedupe set keyed by `fileKey`, kept on success: a scroll-render can
  // re-fire the load effect before React applies the loaded state, and the
  // `diff === null` guard alone would re-fetch. Entries are removed on error
  // to allow retries, and pruned when a refresh changes a file's content; a
  // resolved fetch whose key was pruned is stale and never applied.
  const requested = useRef(new Set<string>());
  const activeLoads = useRef(0);
  // In-progress comment text; survives virtualized rows unmounting.
  const drafts = useRef<DraftStore>(new Map());
  // Live drag state + the fixed end of the selection; refs so the gutter
  // handlers stay referentially stable for RowContent's memo.
  const dragRef = useRef<{
    path: string;
    hi: number;
    side: CommentSide;
  } | null>(null);
  const anchorRef = useRef<number | null>(null);
  // Written wherever the state is set (not at render) so the window mouseup
  // handler sees the latest selection even before React re-renders.
  const selectionRef = useRef(selection);
  const composerRef = useRef(composer);
  composerRef.current = composer;

  const applySelection = useCallback((sel: LineSelection | null) => {
    selectionRef.current = sel;
    setSelection(sel);
  }, []);

  // ── Refresh reconciliation ─────────────────────────────────────────────
  // A new summary adjusts state during render (React's derived-state
  // pattern) instead of remounting, so rows never see a mix of new files
  // and stale state. A params change (branch/mode/whitespace/repo switch)
  // resets everything, like the old remount-by-generation did; a same-params
  // refresh (the watcher, ⌘R) reconciles per file by content fingerprint.
  // Idempotent by construction — StrictMode may run it twice.
  const params = { repoPath, base, head, mode, ignoreWhitespace };
  const [synced, setSynced] = useState({ summary, params });
  if (synced.summary !== summary) {
    const paramsChanged =
      synced.params.repoPath !== repoPath ||
      synced.params.base !== base ||
      synced.params.head !== head ||
      synced.params.mode !== mode ||
      synced.params.ignoreWhitespace !== ignoreWhitespace;
    const prevFiles = synced.summary.files;
    setSynced({ summary, params });
    if (paramsChanged) {
      requested.current = new Set();
      highlightRequested.current = new Set();
      drafts.current = new Map();
      dragRef.current = null;
      anchorRef.current = null;
      cursorPathRef.current = null;
      setStates(initialStates(summary.files, reviewStates));
      setHighlights(new Map());
      applySelection(null);
      setComposer(null);
      setEditingId(null);
      setCursorPath(null);
    } else {
      // Keep state only for files whose content identity is unchanged;
      // changed files reset (their diff, reveals, and context describe old
      // content) and re-expand for re-review; vanished files drop out.
      const prevFingerprints = new Map(
        prevFiles.map((f) => [f.path, f.fingerprint]),
      );
      const kept = new Set<string>();
      const nextStates = new Map<string, FileViewState>();
      for (const file of summary.files) {
        const existing = states.get(file.path);
        if (
          existing !== undefined &&
          prevFingerprints.get(file.path) === file.fingerprint
        ) {
          nextStates.set(file.path, existing);
          kept.add(file.path);
        } else {
          nextStates.set(
            file.path,
            initialFileState(reviewStates.get(file.path) !== "reviewed"),
          );
        }
      }
      setStates(nextStates);
      const valid = new Set(summary.files.map(fileKey));
      requested.current = new Set(
        [...requested.current].filter((k) => valid.has(k)),
      );
      highlightRequested.current = new Set(
        [...highlightRequested.current].filter((k) =>
          valid.has(hunkKeyFile(k)),
        ),
      );
      setHighlights(
        (prev) =>
          new Map([...prev].filter(([k]) => valid.has(hunkKeyFile(k)))),
      );
      // Line selections/composers bind to hunk line numbers, so they only
      // survive on files whose diff is unchanged; file/reply composers just
      // need their file to still be in the diff. The cursor likewise.
      const paths = new Set(summary.files.map((f) => f.path));
      if (
        selectionRef.current !== null &&
        !kept.has(selectionRef.current.path)
      ) {
        dragRef.current = null;
        anchorRef.current = null;
        applySelection(null);
      }
      const pinned = composerRef.current;
      if (
        pinned !== null &&
        (pinned.level === "line"
          ? !kept.has(pinned.path)
          : !paths.has(pinned.path))
      ) {
        setComposer(null);
      }
      if (
        cursorPathRef.current !== null &&
        !paths.has(cursorPathRef.current)
      ) {
        cursorPathRef.current = null;
        setCursorPath(null);
      }
    }
  }

  const fileByPath = useMemo(
    () => new Map(summary.files.map((f) => [f.path, f])),
    [summary.files],
  );

  const updateState = useCallback(
    (path: string, update: (state: FileViewState) => FileViewState) => {
      setStates((prev) => {
        const state = prev.get(path);
        if (state === undefined) {
          return prev;
        }
        const next = new Map(prev);
        next.set(path, update(state));
        return next;
      });
    },
    [],
  );

  const loadFile = useCallback(
    (file: FileSummary) => {
      const key = fileKey(file);
      if (requested.current.has(key)) {
        return;
      }
      requested.current.add(key);
      activeLoads.current += 1;
      getFileDiff(repoPath, base, head, mode, ignoreWhitespace, file.path)
        .then((diff) => {
          // A pruned key means a refresh changed (or dropped) the file while
          // this fetch was in flight; the result describes old content.
          if (!requested.current.has(key)) {
            return;
          }
          updateState(file.path, (s) => ({
            ...s,
            diff,
            error: null,
            reveals: Array.from({ length: diff.hunks.length + 1 }, () => ({
              top: 0,
              bottom: 0,
            })),
          }));
        })
        .catch((e: unknown) => {
          requested.current.delete(key);
          updateState(file.path, (s) => ({ ...s, error: errorText(e) }));
        })
        .finally(() => {
          activeLoads.current -= 1;
        });
    },
    [repoPath, base, head, mode, ignoreWhitespace, updateState],
  );

  const toggleFile = useCallback(
    (path: string) => {
      updateState(path, (s) => ({ ...s, expanded: !s.expanded }));
    },
    [updateState],
  );

  // Marking collapses the card and unmarking re-expands it; the caret
  // (`toggleFile`) stays independent, so peeking at a reviewed file never
  // unmarks it.
  const pendingScrollPath = useRef<string | null>(null);
  const toggleReviewed = useCallback(
    (path: string) => {
      const isReviewed = reviewStates.get(path) === "reviewed";
      updateState(path, (s) => ({ ...s, expanded: isReviewed }));
      if (!isReviewed) {
        // Collapsing removes the file's rows; without a correction the
        // viewport would land mid-way through a later file. Scroll the
        // collapsed header to the top instead — the next file sits right
        // below it. (Runs from an effect once the row model has rebuilt.)
        pendingScrollPath.current = path;
      }
      onToggleReviewed(path);
    },
    [reviewStates, onToggleReviewed, updateState],
  );

  const forceLoadFile = useCallback(
    (path: string) => {
      updateState(path, (s) => ({ ...s, forceLoad: true }));
      const file = fileByPath.get(path);
      if (file !== undefined) {
        loadFile(file);
      }
    },
    [updateState, fileByPath, loadFile],
  );

  const expandGap = useCallback(
    (path: string, gi: number, direction: ExpandDirection) => {
      const state = states.get(path);
      if (state === undefined || state.diff === null) {
        return;
      }
      const gap = computeGaps(state.diff)[gi];
      const reveal = state.reveals[gi] ?? { top: 0, bottom: 0 };
      const firstHidden = gap.start + reveal.top;
      const lastHidden = gap.end - reveal.bottom;
      if (firstHidden > lastHidden) {
        return;
      }
      let from = firstHidden;
      let to = lastHidden;
      if (direction === "top") {
        to = Math.min(firstHidden + CONTEXT_CHUNK - 1, lastHidden);
      } else if (direction === "bottom") {
        from = Math.max(lastHidden - CONTEXT_CHUNK + 1, firstHidden);
      }
      const count = to - from + 1;
      getContextLines(repoPath, head, mode, path, from, to)
        .then((ctx) => {
          updateState(path, (s) => {
            const context = new Map(s.context);
            ctx.lines.forEach((content, i) => context.set(ctx.start + i, content));
            const reveals = s.reveals.map((r, i) => {
              if (i !== gi) {
                return r;
              }
              return direction === "bottom"
                ? { ...r, bottom: r.bottom + count }
                : { ...r, top: r.top + count };
            });
            return { ...s, context, reveals };
          });
        })
        .catch((e: unknown) => {
          updateState(path, (s) => ({ ...s, error: errorText(e) }));
        });
    },
    [states, repoPath, head, mode, updateState],
  );

  // Gutter mousedown: start (or shift-extend) a selection.
  const gutterDown = useCallback((path: string, hi: number, line: DiffLine, shiftKey: boolean) => {
    const side = lineSide(line);
    const n = lineNumber(line);
    const prev = selectionRef.current;
    if (
      shiftKey &&
      prev !== null &&
      prev.path === path &&
      prev.hi === hi &&
      prev.side === side
    ) {
      const anchor = anchorRef.current ?? prev.start;
      const next = {
        path,
        hi,
        side,
        start: Math.min(anchor, n),
        end: Math.max(anchor, n),
      };
      applySelection(next);
      setComposer({
        level: "line",
        path,
        side,
        startLine: next.start,
        endLine: next.end,
      });
      return;
    }
    anchorRef.current = n;
    dragRef.current = { path, hi, side };
    applySelection({ path, hi, side, start: n, end: n });
    setComposer(null);
  }, []);

  // Extend an in-progress drag; ignores rows outside the anchor's hunk/side.
  const rowEnter = useCallback((path: string, hi: number, line: DiffLine) => {
    const drag = dragRef.current;
    if (drag === null || drag.path !== path || drag.hi !== hi) {
      return;
    }
    const side = lineSide(line);
    if (side !== drag.side) {
      return;
    }
    const n = lineNumber(line);
    const anchor = anchorRef.current ?? n;
    applySelection({
      path,
      hi,
      side,
      start: Math.min(anchor, n),
      end: Math.max(anchor, n),
    });
  }, []);

  // Releasing a gutter drag anywhere opens the composer for the selection.
  useEffect(() => {
    const onMouseUp = () => {
      if (dragRef.current === null) {
        return;
      }
      dragRef.current = null;
      const sel = selectionRef.current;
      if (sel !== null) {
        setComposer({
          level: "line",
          path: sel.path,
          side: sel.side,
          startLine: sel.start,
          endLine: sel.end,
        });
      }
    };
    window.addEventListener("mouseup", onMouseUp);
    return () => window.removeEventListener("mouseup", onMouseUp);
  }, []);

  const addFileComment = useCallback(
    (path: string) => {
      updateState(path, (s) => (s.expanded ? s : { ...s, expanded: true }));
      applySelection(null);
      setComposer({ level: "file", path });
    },
    [updateState],
  );

  // Open the reply composer under a thread root's last reply.
  const startReply = useCallback(
    (path: string, rootId: number) => {
      applySelection(null);
      setComposer({ level: "reply", path, rootId });
    },
    [applySelection],
  );

  const submitComposer = useCallback(
    async (body: string) => {
      const target = composerRef.current;
      if (target === null) {
        return;
      }
      if (target.level === "reply") {
        const root = comments.find((c) => c.id === target.rootId);
        if (root === undefined) {
          throw `Comment not found: C${target.rootId}`;
        }
        // Rust resolves the parent to the thread root and ignores the
        // positional fields; the level is passed through for shape only.
        await onCreateComment({ level: root.level, parentId: root.id, body });
      } else if (target.level === "file") {
        await onCreateComment({ level: "file", filePath: target.path, body });
      } else {
        await onCreateComment({
          level: "line",
          filePath: target.path,
          side: target.side,
          startLine: target.startLine,
          endLine: target.endLine,
          body,
        });
      }
      setComposer(null);
      applySelection(null);
    },
    [onCreateComment, comments],
  );

  const cancelComposer = useCallback(() => {
    setComposer(null);
    applySelection(null);
  }, []);

  const saveComment = useCallback(
    (id: number, body: string) =>
      onUpdateComment(id, body).then(() => setEditingId(null)),
    [onUpdateComment],
  );

  const cancelEdit = useCallback(() => setEditingId(null), []);

  // Double-click on a file card's name copies its repo-relative path;
  // ⌥ double-click copies the absolute path.
  const { copied, copyPath } = useCopyPath(repoPath);

  const commentIndex = useMemo(
    () => indexComments(summary.files, comments),
    [summary.files, comments],
  );

  const rows = useMemo(
    () =>
      buildRows(
        summary.files,
        states,
        commentIndex,
        replies,
        composer,
        ignoreWhitespace,
      ),
    [summary.files, states, commentIndex, replies, composer, ignoreWhitespace],
  );

  // Height of the content rendered above the virtual list (the orphaned
  // bucket); fed to the virtualizer as scrollMargin so row offsets and
  // scrollToIndex stay correct. Item `start` values include the margin.
  const topRef = useRef<HTMLDivElement | null>(null);
  const [topHeight, setTopHeight] = useState(0);
  useLayoutEffect(() => {
    const el = topRef.current;
    if (el === null) {
      return;
    }
    const measure = () => setTopHeight(el.getBoundingClientRect().height);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Sticky file headers: while the top of the scrollport is inside a file's
  // BODY, that file's header stays mounted (rangeExtractor) and renders
  // in-flow with position: sticky instead of a translate, so it pins while
  // its rows scroll beneath it. A header row at the top pins nothing — so
  // collapsed (e.g. reviewed) files, which contribute no body rows, never
  // pin and scroll away like any other row.
  const headerIndexByFi = useMemo(() => {
    const map = new Map<number, number>();
    rows.forEach((row, i) => {
      if (row.kind === "file") {
        map.set(row.fi, i);
      }
    });
    return map;
  }, [rows]);
  const activeStickyIndex = useRef(-1);
  const rangeExtractor = useCallback(
    (range: Range) => {
      const top = rows[range.startIndex];
      const active =
        top !== undefined && top.kind !== "file"
          ? (headerIndexByFi.get(top.fi) ?? -1)
          : -1;
      activeStickyIndex.current = active;
      const next = new Set(defaultRangeExtractor(range));
      if (active !== -1) {
        next.add(active);
      }
      return [...next].sort((a, b) => a - b);
    },
    [rows, headerIndexByFi],
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateRowHeight(rows[index]),
    getItemKey: (index) => rowKey(rows[index], summary.files),
    overscan: 12,
    scrollMargin: topHeight,
    rangeExtractor,
  });
  const items = virtualizer.getVirtualItems();

  // Push-out: once the pinned file's last row nears the top, slide the
  // header up with it (GitHub-style) instead of hovering over the tail.
  // Driven imperatively from scroll events — the virtualizer doesn't
  // re-render per scrolled pixel, so React state would lag.
  const pinnedLastIndex = (() => {
    const active = activeStickyIndex.current;
    if (active === -1) {
      return -1;
    }
    for (let i = active + 1; i < rows.length; i++) {
      if (rows[i].kind === "file") {
        return i - 1;
      }
    }
    return rows.length - 1;
  })();
  const pinnedRef = useRef<{ el: HTMLDivElement; lastIndex: number } | null>(
    null,
  );
  const updatePinnedPush = useCallback(() => {
    const scroller = scrollRef.current;
    const pinned = pinnedRef.current;
    if (scroller === null || pinned === null) {
      return;
    }
    const measurement = virtualizer.measurementsCache[pinned.lastIndex];
    if (measurement === undefined) {
      return;
    }
    // The row wrapper carries a 16px transparent spacer above the 44px-ish
    // header; sticky top -16px keeps the header flush. Viewport y of the
    // file's bottom = its virtual end − scroll (the pane has no top padding).
    const spacer = 16;
    const headerHeight = pinned.el.offsetHeight - spacer;
    const fileBottom = measurement.end - scroller.scrollTop;
    const push = Math.min(0, fileBottom - headerHeight);
    pinned.el.style.top = `${-spacer + push}px`;
  }, [virtualizer]);
  useEffect(() => {
    const scroller = scrollRef.current;
    if (scroller === null) {
      return;
    }
    scroller.addEventListener("scroll", updatePinnedPush, { passive: true });
    return () => scroller.removeEventListener("scroll", updatePinnedPush);
  }, [updatePinnedPush]);
  // Re-apply after every render: the pinned row, its file's measured end,
  // or the row model may have changed without a scroll event.
  useEffect(() => {
    updatePinnedPush();
  });

  const langs = useMemo(
    () => summary.files.map((file) => detectLang(file.path)),
    [summary.files],
  );

  // Lazily fetch hunks for expanded files whose rows are in the viewport.
  // Runs after every render; the guards keep it a cheap no-op once loaded.
  useEffect(() => {
    for (const item of items) {
      if (activeLoads.current >= MAX_CONCURRENT_LOADS) {
        break;
      }
      const row = rows[item.index];
      if (row.kind !== "file" && row.kind !== "skeleton") {
        continue;
      }
      const file = summary.files[row.fi];
      const state = states.get(file.path);
      if (
        state === undefined ||
        !state.expanded ||
        state.diff !== null ||
        state.error !== null
      ) {
        continue;
      }
      if (guardReason(file) !== null && !state.forceLoad) {
        continue;
      }
      loadFile(file);
    }
  });

  // Lazily tokenize hunks whose lines are in the viewport. Same cadence as
  // the fetch effect above; the requested-set makes re-runs cheap no-ops.
  useEffect(() => {
    for (const item of items) {
      const row = rows[item.index];
      if ((row.kind !== "line" && row.kind !== "hunk") || row.hi === undefined) {
        continue;
      }
      const lang = langs[row.fi];
      if (lang === null) {
        continue;
      }
      const key = hunkKey(summary.files[row.fi], row.hi);
      if (highlightRequested.current.has(key)) {
        continue;
      }
      const diff = states.get(summary.files[row.fi].path)?.diff;
      if (diff === undefined || diff === null) {
        continue;
      }
      highlightRequested.current.add(key);
      const contents = diff.hunks[row.hi].lines.map((line) => line.content);
      tokenizeLines(contents, lang)
        .then((tokens) => {
          if (tokens !== null) {
            setHighlights((prev) => new Map(prev).set(key, tokens));
          }
        })
        .catch(() => {
          // Tokenization failures keep the plain-text rendering.
        });
    }
  });

  const scrollToPath = useCallback(
    (path: string) => {
      const index = rows.findIndex(
        (row) => row.kind === "file" && summary.files[row.fi].path === path,
      );
      if (index >= 0) {
        virtualizer.scrollToIndex(index, { align: "start" });
      }
    },
    [rows, summary.files, virtualizer],
  );

  // Deferred scroll after a mark-reviewed collapse: waits for `rows` to
  // rebuild without the collapsed file's body so the header's new index
  // (and the shrunken offsets) are what gets scrolled to.
  useEffect(() => {
    const path = pendingScrollPath.current;
    if (path !== null) {
      pendingScrollPath.current = null;
      scrollToPath(path);
    }
  }, [rows, scrollToPath]);

  const moveCursor = useCallback(
    (delta: number) => {
      const files = summary.files;
      if (files.length === 0) {
        return;
      }
      const prev =
        cursorPathRef.current === null
          ? -1
          : files.findIndex((f) => f.path === cursorPathRef.current);
      // From nowhere (or a vanished file), j lands on the first file and k
      // on the last.
      const next =
        prev === -1
          ? delta > 0
            ? 0
            : files.length - 1
          : Math.min(Math.max(prev + delta, 0), files.length - 1);
      const path = files[next].path;
      cursorPathRef.current = path;
      setCursorPath(path);
      scrollToPath(path);
    },
    [summary.files, scrollToPath],
  );

  // n/p position memory: the open thread the motion last landed on. A ref,
  // not state — nothing renders it, and the keydown handler needs the
  // latest value. A stale id (thread resolved, file collapsed) just makes
  // the next step re-enter at the first/last target.
  const lastCommentIdRef = useRef<number | null>(null);
  const visibleThreads = useMemo(() => commentTargets(rows), [rows]);

  const moveCommentCursor = useCallback(
    (delta: 1 | -1) => {
      const target = nextCommentTarget(
        visibleThreads,
        lastCommentIdRef.current,
        delta,
      );
      if (target !== null) {
        lastCommentIdRef.current = target.id;
        virtualizer.scrollToIndex(target.rowIndex, { align: "center" });
      }
    },
    [visibleThreads, virtualizer],
  );

  // J/K move the file cursor like j/k, but only between unviewed files.
  const moveCursorUnviewed = useCallback(
    (delta: 1 | -1) => {
      const path = nextUnviewedPath(
        summary.files,
        reviewStates,
        cursorPathRef.current,
        delta,
      );
      if (path !== null) {
        cursorPathRef.current = path;
        setCursorPath(path);
        scrollToPath(path);
      }
    },
    [summary.files, reviewStates, scrollToPath],
  );

  // `c` comments on the current line selection if there is one, else on the
  // file the keyboard cursor sits on.
  const composeAtCursor = useCallback(() => {
    const sel = selectionRef.current;
    if (sel !== null) {
      setComposer({
        level: "line",
        path: sel.path,
        side: sel.side,
        startLine: sel.start,
        endLine: sel.end,
      });
      return;
    }
    const path = cursorPathRef.current;
    if (path !== null) {
      addFileComment(path);
    }
  }, [addFileComment]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) {
        return;
      }
      // Never hijack typing: composers, branch selects, future inputs.
      const target = e.target;
      if (
        target instanceof HTMLElement &&
        (target.tagName === "TEXTAREA" ||
          target.tagName === "INPUT" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      // The archive browser overlays the diff; don't scroll behind it.
      if (document.querySelector(".archive-overlay") !== null) {
        return;
      }
      if (e.key === "j") {
        e.preventDefault();
        moveCursor(1);
      } else if (e.key === "k") {
        e.preventDefault();
        moveCursor(-1);
      } else if (e.key === "J") {
        e.preventDefault();
        moveCursorUnviewed(1);
      } else if (e.key === "K") {
        e.preventDefault();
        moveCursorUnviewed(-1);
      } else if (e.key === "n") {
        e.preventDefault();
        moveCommentCursor(1);
      } else if (e.key === "p") {
        e.preventDefault();
        moveCommentCursor(-1);
      } else if (e.key === "c") {
        e.preventDefault();
        composeAtCursor();
      } else if (e.key === "v") {
        e.preventDefault();
        const path = cursorPathRef.current;
        if (path !== null) {
          toggleReviewed(path);
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    moveCursor,
    moveCursorUnviewed,
    moveCommentCursor,
    composeAtCursor,
    toggleReviewed,
  ]);

  useEffect(() => {
    if (scrollTarget === null) {
      return;
    }
    const index = rows.findIndex(
      (row) =>
        row.kind === "file" &&
        summary.files[row.fi].path === scrollTarget.path,
    );
    if (index >= 0) {
      virtualizer.scrollToIndex(index, { align: "start" });
    }
    // Keyed to the nonce alone: only a click should scroll, not row-model
    // changes.
  }, [scrollTarget?.nonce]);

  return (
    <div className="diff-scroll" ref={scrollRef}>
      <div ref={topRef}>{topContent}</div>
      <div
        className="diff-virtual"
        style={{ height: virtualizer.getTotalSize() }}
      >
        {items.map((item) => {
          const isPinned = item.index === activeStickyIndex.current;
          return (
          <div
            key={item.key}
            data-index={item.index}
            ref={(el) => {
              virtualizer.measureElement(el);
              if (isPinned) {
                pinnedRef.current =
                  el === null ? null : { el, lastIndex: pinnedLastIndex };
              } else if (el !== null && el.style.top !== "") {
                // React doesn't know about the imperative push-out offset;
                // clear it when a row stops being the pinned header.
                el.style.top = "";
              }
            }}
            className={isPinned ? "diff-row diff-row-pinned" : "diff-row"}
            style={
              isPinned
                ? undefined
                : { transform: `translateY(${item.start - topHeight}px)` }
            }
          >
            <RowContent
              row={rows[item.index]}
              files={summary.files}
              states={states}
              highlights={highlights}
              cursorPath={cursorPath}
              selection={selection}
              composer={composer}
              editingId={editingId}
              drafts={drafts.current}
              replies={replies}
              anchorStatuses={anchorStatuses}
              reviewStates={reviewStates}
              onToggle={toggleFile}
              onToggleReviewed={toggleReviewed}
              onLoad={forceLoadFile}
              onCopyPath={copyPath}
              onExpand={expandGap}
              onGutterDown={gutterDown}
              onRowEnter={rowEnter}
              onAddFileComment={addFileComment}
              onStartReply={startReply}
              onComposerSubmit={submitComposer}
              onComposerCancel={cancelComposer}
              onEditStart={setEditingId}
              onEditCancel={cancelEdit}
              onSaveComment={saveComment}
              onDeleteComment={onDeleteComment}
              onSetCommentState={onSetCommentState}
            />
          </div>
          );
        })}
      </div>
      {copied && (
        <div className="copy-toast" role="status">
          {copied === "absolute"
            ? "Copied absolute path to clipboard"
            : "Copied file path to clipboard"}
        </div>
      )}
    </div>
  );
}

interface RowContentProps {
  row: Row;
  files: FileSummary[];
  states: ReadonlyMap<string, FileViewState>;
  highlights: ReadonlyMap<string, LineTokens[]>;
  cursorPath: string | null;
  selection: LineSelection | null;
  composer: ComposerLocation | null;
  editingId: number | null;
  drafts: DraftStore;
  replies: RepliesByRoot;
  anchorStatuses: ReadonlyMap<number, AnchorStatus>;
  reviewStates: ReadonlyMap<string, FileReviewState>;
  onToggle: (path: string) => void;
  onToggleReviewed: (path: string) => void;
  onLoad: (path: string) => void;
  onCopyPath: (path: string, absolute: boolean) => void;
  onExpand: (path: string, gi: number, direction: ExpandDirection) => void;
  onGutterDown: (path: string, hi: number, line: DiffLine, shiftKey: boolean) => void;
  onRowEnter: (path: string, hi: number, line: DiffLine) => void;
  onAddFileComment: (path: string) => void;
  onStartReply: (path: string, rootId: number) => void;
  onComposerSubmit: (body: string) => Promise<void>;
  onComposerCancel: () => void;
  onEditStart: (id: number) => void;
  onEditCancel: () => void;
  onSaveComment: (id: number, body: string) => Promise<void>;
  onDeleteComment: (id: number) => Promise<void>;
  onSetCommentState: (id: number, state: CommentState) => Promise<void>;
}

/**
 * Memoized so scrolling — which only moves wrapper transforms — never
 * re-renders row subtrees; row objects and every handler are stable across
 * scroll frames. Interaction state (selection, composer, editing) does
 * re-render the visible rows, which is fine — those are user-paced events.
 */
const RowContent = memo(function RowContent({
  row,
  files,
  states,
  highlights,
  cursorPath,
  selection,
  composer,
  editingId,
  drafts,
  replies,
  anchorStatuses,
  reviewStates,
  onToggle,
  onToggleReviewed,
  onLoad,
  onCopyPath,
  onExpand,
  onGutterDown,
  onRowEnter,
  onAddFileComment,
  onStartReply,
  onComposerSubmit,
  onComposerCancel,
  onEditStart,
  onEditCancel,
  onSaveComment,
  onDeleteComment,
  onSetCommentState,
}: RowContentProps) {
  const path = files[row.fi].path;
  switch (row.kind) {
    case "file":
      return (
        <FileHeaderRow
          file={files[row.fi]}
          expanded={states.get(path)?.expanded ?? true}
          focused={cursorPath === path}
          reviewState={reviewStates.get(path)}
          onToggle={() => onToggle(path)}
          onToggleReviewed={() => onToggleReviewed(path)}
          onAddComment={() => onAddFileComment(path)}
          onCopyPath={(absolute) => onCopyPath(path, absolute)}
        />
      );
    case "notice":
      return (
        <GuardNoticeRow
          file={files[row.fi]}
          reason={row.reason}
          onLoad={() => onLoad(path)}
        />
      );
    case "skeleton":
      return (
        <div className="diff-skeleton" style={{ height: row.height }}>
          Loading diff…
        </div>
      );
    case "error":
      return <div className="diff-file-error">{row.message}</div>;
    case "empty":
      return (
        <div className="diff-file-empty">
          {row.whitespaceHidden
            ? "Only whitespace changes hidden."
            : "No content changes."}
        </div>
      );
    case "hiddenComments":
      return (
        <div className="diff-hidden-comments">
          {row.count} {row.count === 1 ? "comment" : "comments"} on hidden
          lines — show whitespace to view.
        </div>
      );
    case "hunk":
      return <div className="hunk-header">{row.header}</div>;
    case "line": {
      const selected =
        selection !== null &&
        row.hi !== undefined &&
        selection.path === path &&
        selection.hi === row.hi &&
        lineSide(row.line) === selection.side &&
        lineNumber(row.line) >= selection.start &&
        lineNumber(row.line) <= selection.end;
      const tokens =
        row.hi !== undefined && row.li !== undefined
          ? highlights.get(hunkKey(files[row.fi], row.hi))?.[row.li]
          : undefined;
      return (
        <LineRow
          line={row.line}
          path={path}
          hi={row.hi}
          tokens={tokens}
          selected={selected}
          onGutterDown={onGutterDown}
          onRowEnter={onRowEnter}
        />
      );
    }
    case "comment": {
      const isReply = row.comment.parentId !== null;
      return (
        <div className="inline-comment">
          <CommentCard
            comment={row.comment}
            editing={editingId === row.comment.id}
            drafts={drafts}
            codeChanged={anchorStatuses.get(row.comment.id) === "changed"}
            replyCount={isReply ? 0 : (replies.get(row.comment.id)?.length ?? 0)}
            onEditStart={onEditStart}
            onEditCancel={onEditCancel}
            onSave={onSaveComment}
            onDelete={onDeleteComment}
            // Lifecycle lives on thread roots; replies expose none.
            onSetState={isReply ? undefined : onSetCommentState}
            onReply={
              !isReply && row.comment.state === "open"
                ? () => onStartReply(path, row.comment.id)
                : undefined
            }
          />
        </div>
      );
    }
    case "composer":
      return (
        <div
          className={`inline-comment${composer?.level === "reply" ? " reply-composer" : ""}`}
        >
          <CommentComposer
            drafts={drafts}
            draftKey={
              composer?.level === "reply"
                ? replyDraftKey(composer.rootId)
                : "new"
            }
            placeholder={
              composer?.level === "reply"
                ? `Reply to C${composer.rootId}…`
                : composer?.level === "file"
                  ? `Comment on ${files[row.fi].path}…`
                  : "Comment on the selected lines…"
            }
            submitLabel={composer?.level === "reply" ? "Reply" : undefined}
            onSubmit={onComposerSubmit}
            onCancel={onComposerCancel}
          />
        </div>
      );
    case "expand":
      return (
        <ExpandRow
          hidden={row.hidden}
          growTop={row.growTop}
          growBottom={row.growBottom}
          onExpand={(direction) => onExpand(path, row.gi, direction)}
        />
      );
  }
});

function FileHeaderRow({
  file,
  expanded,
  focused,
  reviewState,
  onToggle,
  onToggleReviewed,
  onAddComment,
  onCopyPath,
}: {
  file: FileSummary;
  expanded: boolean;
  /** The keyboard cursor (j/k) sits on this file. */
  focused: boolean;
  reviewState: FileReviewState | undefined;
  onToggle: () => void;
  onToggleReviewed: () => void;
  onAddComment: () => void;
  /** Double-click on the file name copies its path; ⌥ for absolute. */
  onCopyPath: (absolute: boolean) => void;
}) {
  return (
    <div
      className={`diff-file-header${focused ? " diff-file-header-focused" : ""}`}
    >
      <button
        type="button"
        className="file-toggle"
        aria-expanded={expanded}
        aria-label={expanded ? "Collapse file" : "Expand file"}
        onClick={onToggle}
      >
        <Chevron expanded={expanded} />
      </button>
      <span
        className={`status-badge status-${file.status}`}
        title={file.status}
      >
        {STATUS_LABELS[file.status]}
      </span>
      <span
        className="file-path"
        title={`${file.path}\nDouble-click to copy the path (⌥ for absolute)`}
        onDoubleClick={(event) => onCopyPath(event.altKey)}
      >
        {file.oldPath !== null && (
          <>
            <span className="file-old-path">{file.oldPath}</span>
            <span className="file-rename-arrow" aria-hidden="true">
              {" → "}
            </span>
          </>
        )}
        {file.path}
      </span>
      {reviewState === "changed" && (
        <span
          className="changed-badge"
          title="This file changed since you marked it reviewed"
        >
          Changed since review
        </span>
      )}
      <button
        type="button"
        className="add-comment-button"
        title={`Comment on ${file.path}`}
        onClick={onAddComment}
      >
        + Add comment
      </button>
      {file.binary ? (
        <span className="file-counts file-binary">BIN</span>
      ) : (
        <span className="file-counts">
          <span className="count-added">+{file.additions}</span>
          <span className="count-deleted">−{file.deletions}</span>
        </span>
      )}
      <label className="reviewed-toggle" title="Mark this file as reviewed (v)">
        <input
          type="checkbox"
          checked={reviewState === "reviewed"}
          onChange={(e) => {
            // Blur so the window keydown handler (which ignores INPUT
            // targets) keeps serving j/k/c/v after a mouse toggle.
            e.currentTarget.blur();
            onToggleReviewed();
          }}
        />
        Viewed
      </label>
    </div>
  );
}

function GuardNoticeRow({
  file,
  reason,
  onLoad,
}: {
  file: FileSummary;
  reason: GuardReason;
  onLoad: () => void;
}) {
  if (reason === "binary") {
    return <div className="diff-notice">Binary file — no text diff.</div>;
  }
  const label =
    reason === "oversize"
      ? `Large diff: ${(file.additions + file.deletions).toLocaleString()} changed lines.`
      : "Lockfile / generated file — hidden by default.";
  return (
    <div className="diff-notice">
      <span>{label}</span>
      <button type="button" className="load-diff-button" onClick={onLoad}>
        Load diff
      </button>
    </div>
  );
}

function LineRow({
  line,
  path,
  hi,
  tokens,
  selected,
  onGutterDown,
  onRowEnter,
}: {
  line: DiffLine;
  path: string;
  /** Absent for expanded gap-context lines, which are not commentable. */
  hi: number | undefined;
  /** Syntax tokens for this line; absent until its hunk is tokenized. */
  tokens: LineTokens | undefined;
  selected: boolean;
  onGutterDown: (path: string, hi: number, line: DiffLine, shiftKey: boolean) => void;
  onRowEnter: (path: string, hi: number, line: DiffLine) => void;
}) {
  const commentable = hi !== undefined;
  const gutterProps = commentable
    ? {
        className: "lineno lineno-commentable",
        onMouseDown: (e: React.MouseEvent) => {
          if (e.button === 0) {
            // Keep the drag from starting a text selection.
            e.preventDefault();
            onGutterDown(path, hi, line, e.shiftKey);
          }
        },
      }
    : { className: "lineno" };
  return (
    <div
      className={`diff-line diff-line-${line.kind}${selected ? " diff-line-selected" : ""}`}
      onMouseEnter={commentable ? () => onRowEnter(path, hi, line) : undefined}
    >
      <span {...gutterProps}>{line.oldLineno ?? ""}</span>
      <span {...gutterProps}>{line.newLineno ?? ""}</span>
      <span className="line-sign" aria-hidden="true">
        {line.kind === "addition" ? "+" : line.kind === "deletion" ? "−" : ""}
      </span>
      <span className="line-content">
        {segmentLine(line, tokens).map((seg, i) => (
          <span
            key={i}
            className={seg.changed ? "intraline-changed" : undefined}
            style={seg.style as CSSProperties}
          >
            {seg.text}
          </span>
        ))}
      </span>
    </div>
  );
}

function ExpandRow({
  hidden,
  growTop,
  growBottom,
  onExpand,
}: {
  hidden: number;
  growTop: boolean;
  growBottom: boolean;
  onExpand: (direction: ExpandDirection) => void;
}) {
  if (hidden <= CONTEXT_CHUNK) {
    return (
      <div className="expand-row">
        <button type="button" onClick={() => onExpand("all")}>
          ↕ Expand {hidden} hidden {hidden === 1 ? "line" : "lines"}
        </button>
      </div>
    );
  }
  return (
    <div className="expand-row">
      {growTop && (
        <button
          type="button"
          title="Show the next 20 lines below the hunk above"
          onClick={() => onExpand("top")}
        >
          ↓ Show {CONTEXT_CHUNK}
        </button>
      )}
      <span className="expand-hidden-count">
        {hidden.toLocaleString()} unchanged lines
      </span>
      {growBottom && (
        <button
          type="button"
          title="Show the previous 20 lines above the hunk below"
          onClick={() => onExpand("bottom")}
        >
          ↑ Show {CONTEXT_CHUNK}
        </button>
      )}
      <button type="button" onClick={() => onExpand("all")}>
        Expand all
      </button>
    </div>
  );
}
