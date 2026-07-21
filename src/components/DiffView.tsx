import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { getContextLines, getFileDiff } from "../ipc";
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
import type {
  AnchorStatus,
  Comment,
  CommentSide,
  CommentState,
  DiffLine,
  DiffSummary,
  FileStatus,
  FileSummary,
  NewCommentInput,
  RepliesByRoot,
  WorkingTreeMode,
} from "../types";
import {
  CommentCard,
  CommentComposer,
  replyDraftKey,
  type DraftStore,
} from "./Comments";
import { useCopyPath } from "./useCopyPath";

/** Parallel `get_file_diff` calls; each recomputes the repo diff in Rust. */
const MAX_CONCURRENT_LOADS = 3;

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
  mode: WorkingTreeMode;
  /** Fetch file hunks with whitespace changes ignored (git `-w`); must match
   * the setting the summary was computed with. */
  ignoreWhitespace: boolean;
  summary: DiffSummary;
  /** Bumping `nonce` scrolls the file's header into view. */
  scrollTarget: { path: string; nonce: number } | null;
  /** File- and line-level comments (review-level ones are ignored here). */
  comments: Comment[];
  /** Replies grouped under their thread root, for every thread. */
  replies: RepliesByRoot;
  /** Re-anchor outcome per line comment; "changed" flags the comment card. */
  anchorStatuses: ReadonlyMap<number, AnchorStatus>;
  onCreateComment: (input: NewCommentInput) => Promise<void>;
  onUpdateComment: (id: number, body: string) => Promise<void>;
  onDeleteComment: (id: number) => Promise<void>;
  onSetCommentState: (id: number, state: CommentState) => Promise<void>;
}

type ExpandDirection = "top" | "bottom" | "all";

/** A gutter-drag / shift-click line selection, confined to one hunk + side. */
interface LineSelection {
  fi: number;
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
  scrollTarget,
  comments,
  replies,
  anchorStatuses,
  onCreateComment,
  onUpdateComment,
  onDeleteComment,
  onSetCommentState,
}: DiffViewProps) {
  const [states, setStates] = useState<FileViewState[]>(() =>
    summary.files.map(initialFileState),
  );
  // Syntax tokens per visible hunk, keyed `${fi}:${hi}`; rows render plain
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
  const [cursorFi, setCursorFi] = useState<number | null>(null);
  const cursorFiRef = useRef<number | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  // Dedupe set, kept on success: a scroll-render can re-fire the load effect
  // before React applies the loaded state, and the `diff === null` guard
  // alone would re-fetch. Entries are only removed to allow error retries.
  const requested = useRef(new Set<number>());
  const activeLoads = useRef(0);
  // In-progress comment text; survives virtualized rows unmounting.
  const drafts = useRef<DraftStore>(new Map());
  // Live drag state + the fixed end of the selection; refs so the gutter
  // handlers stay referentially stable for RowContent's memo.
  const dragRef = useRef<{ fi: number; hi: number; side: CommentSide } | null>(
    null,
  );
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

  const updateState = useCallback(
    (fi: number, update: (state: FileViewState) => FileViewState) => {
      setStates((prev) => prev.map((s, i) => (i === fi ? update(s) : s)));
    },
    [],
  );

  const loadFile = useCallback(
    (fi: number) => {
      if (requested.current.has(fi)) {
        return;
      }
      requested.current.add(fi);
      activeLoads.current += 1;
      getFileDiff(
        repoPath,
        base,
        head,
        mode,
        ignoreWhitespace,
        summary.files[fi].path,
      )
        .then((diff) => {
          updateState(fi, (s) => ({
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
          requested.current.delete(fi);
          updateState(fi, (s) => ({ ...s, error: errorText(e) }));
        })
        .finally(() => {
          activeLoads.current -= 1;
        });
    },
    [repoPath, base, head, mode, ignoreWhitespace, summary, updateState],
  );

  const toggleFile = useCallback(
    (fi: number) => {
      updateState(fi, (s) => ({ ...s, expanded: !s.expanded }));
    },
    [updateState],
  );

  const forceLoadFile = useCallback(
    (fi: number) => {
      updateState(fi, (s) => ({ ...s, forceLoad: true }));
      loadFile(fi);
    },
    [updateState, loadFile],
  );

  const expandGap = useCallback(
    (fi: number, gi: number, direction: ExpandDirection) => {
      const state = states[fi];
      if (state.diff === null) {
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
      getContextLines(repoPath, head, mode, summary.files[fi].path, from, to)
        .then((ctx) => {
          updateState(fi, (s) => {
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
          updateState(fi, (s) => ({ ...s, error: errorText(e) }));
        });
    },
    [states, repoPath, head, mode, summary, updateState],
  );

  // Gutter mousedown: start (or shift-extend) a selection.
  const gutterDown = useCallback((fi: number, hi: number, line: DiffLine, shiftKey: boolean) => {
    const side = lineSide(line);
    const n = lineNumber(line);
    const prev = selectionRef.current;
    if (
      shiftKey &&
      prev !== null &&
      prev.fi === fi &&
      prev.hi === hi &&
      prev.side === side
    ) {
      const anchor = anchorRef.current ?? prev.start;
      const next = {
        fi,
        hi,
        side,
        start: Math.min(anchor, n),
        end: Math.max(anchor, n),
      };
      applySelection(next);
      setComposer({
        level: "line",
        fi,
        side,
        startLine: next.start,
        endLine: next.end,
      });
      return;
    }
    anchorRef.current = n;
    dragRef.current = { fi, hi, side };
    applySelection({ fi, hi, side, start: n, end: n });
    setComposer(null);
  }, []);

  // Extend an in-progress drag; ignores rows outside the anchor's hunk/side.
  const rowEnter = useCallback((fi: number, hi: number, line: DiffLine) => {
    const drag = dragRef.current;
    if (drag === null || drag.fi !== fi || drag.hi !== hi) {
      return;
    }
    const side = lineSide(line);
    if (side !== drag.side) {
      return;
    }
    const n = lineNumber(line);
    const anchor = anchorRef.current ?? n;
    applySelection({
      fi,
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
          fi: sel.fi,
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
    (fi: number) => {
      updateState(fi, (s) => (s.expanded ? s : { ...s, expanded: true }));
      applySelection(null);
      setComposer({ level: "file", fi });
    },
    [updateState],
  );

  // Open the reply composer under a thread root's last reply.
  const startReply = useCallback(
    (fi: number, rootId: number) => {
      applySelection(null);
      setComposer({ level: "reply", fi, rootId });
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
        const filePath = summary.files[target.fi].path;
        await onCreateComment({ level: "file", filePath, body });
      } else {
        await onCreateComment({
          level: "line",
          filePath: summary.files[target.fi].path,
          side: target.side,
          startLine: target.startLine,
          endLine: target.endLine,
          body,
        });
      }
      setComposer(null);
      applySelection(null);
    },
    [onCreateComment, summary, comments],
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

  // Double-click on a file card's name copies its absolute path.
  const { copied, copyPath } = useCopyPath();
  const copyFilePath = useCallback(
    (fi: number) => copyPath(`${repoPath}/${summary.files[fi].path}`),
    [copyPath, repoPath, summary.files],
  );

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

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateRowHeight(rows[index]),
    getItemKey: (index) => rowKey(rows[index]),
    overscan: 12,
  });
  const items = virtualizer.getVirtualItems();

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
      const state = states[row.fi];
      if (!state.expanded || state.diff !== null || state.error !== null) {
        continue;
      }
      if (guardReason(summary.files[row.fi]) !== null && !state.forceLoad) {
        continue;
      }
      loadFile(row.fi);
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
      const key = `${row.fi}:${row.hi}`;
      if (highlightRequested.current.has(key)) {
        continue;
      }
      const diff = states[row.fi].diff;
      if (diff === null) {
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

  const scrollToFile = useCallback(
    (fi: number) => {
      const index = rows.findIndex(
        (row) => row.kind === "file" && row.fi === fi,
      );
      if (index >= 0) {
        virtualizer.scrollToIndex(index, { align: "start" });
      }
    },
    [rows, virtualizer],
  );

  const moveCursor = useCallback(
    (delta: number) => {
      const count = summary.files.length;
      if (count === 0) {
        return;
      }
      const prev = cursorFiRef.current;
      // From nowhere, j lands on the first file and k on the last.
      const next =
        prev === null
          ? delta > 0
            ? 0
            : count - 1
          : Math.min(Math.max(prev + delta, 0), count - 1);
      cursorFiRef.current = next;
      setCursorFi(next);
      scrollToFile(next);
    },
    [summary.files.length, scrollToFile],
  );

  // `c` comments on the current line selection if there is one, else on the
  // file the keyboard cursor sits on.
  const composeAtCursor = useCallback(() => {
    const sel = selectionRef.current;
    if (sel !== null) {
      setComposer({
        level: "line",
        fi: sel.fi,
        side: sel.side,
        startLine: sel.start,
        endLine: sel.end,
      });
      return;
    }
    const fi = cursorFiRef.current;
    if (fi !== null) {
      addFileComment(fi);
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
      } else if (e.key === "c") {
        e.preventDefault();
        composeAtCursor();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [moveCursor, composeAtCursor]);

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
      <div
        className="diff-virtual"
        style={{ height: virtualizer.getTotalSize() }}
      >
        {items.map((item) => (
          <div
            key={item.key}
            data-index={item.index}
            ref={virtualizer.measureElement}
            className="diff-row"
            style={{ transform: `translateY(${item.start}px)` }}
          >
            <RowContent
              row={rows[item.index]}
              files={summary.files}
              states={states}
              highlights={highlights}
              cursorFi={cursorFi}
              selection={selection}
              composer={composer}
              editingId={editingId}
              drafts={drafts.current}
              replies={replies}
              anchorStatuses={anchorStatuses}
              onToggle={toggleFile}
              onLoad={forceLoadFile}
              onCopyPath={copyFilePath}
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
        ))}
      </div>
      {copied && (
        <div className="copy-toast" role="status">
          Copied file path to clipboard
        </div>
      )}
    </div>
  );
}

interface RowContentProps {
  row: Row;
  files: FileSummary[];
  states: FileViewState[];
  highlights: ReadonlyMap<string, LineTokens[]>;
  cursorFi: number | null;
  selection: LineSelection | null;
  composer: ComposerLocation | null;
  editingId: number | null;
  drafts: DraftStore;
  replies: RepliesByRoot;
  anchorStatuses: ReadonlyMap<number, AnchorStatus>;
  onToggle: (fi: number) => void;
  onLoad: (fi: number) => void;
  onCopyPath: (fi: number) => void;
  onExpand: (fi: number, gi: number, direction: ExpandDirection) => void;
  onGutterDown: (fi: number, hi: number, line: DiffLine, shiftKey: boolean) => void;
  onRowEnter: (fi: number, hi: number, line: DiffLine) => void;
  onAddFileComment: (fi: number) => void;
  onStartReply: (fi: number, rootId: number) => void;
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
  cursorFi,
  selection,
  composer,
  editingId,
  drafts,
  replies,
  anchorStatuses,
  onToggle,
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
  switch (row.kind) {
    case "file":
      return (
        <FileHeaderRow
          file={files[row.fi]}
          expanded={states[row.fi].expanded}
          focused={cursorFi === row.fi}
          onToggle={() => onToggle(row.fi)}
          onAddComment={() => onAddFileComment(row.fi)}
          onCopyPath={() => onCopyPath(row.fi)}
        />
      );
    case "notice":
      return (
        <GuardNoticeRow
          file={files[row.fi]}
          reason={row.reason}
          onLoad={() => onLoad(row.fi)}
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
        selection.fi === row.fi &&
        selection.hi === row.hi &&
        lineSide(row.line) === selection.side &&
        lineNumber(row.line) >= selection.start &&
        lineNumber(row.line) <= selection.end;
      const tokens =
        row.hi !== undefined && row.li !== undefined
          ? highlights.get(`${row.fi}:${row.hi}`)?.[row.li]
          : undefined;
      return (
        <LineRow
          line={row.line}
          fi={row.fi}
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
                ? () => onStartReply(row.fi, row.comment.id)
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
          onExpand={(direction) => onExpand(row.fi, row.gi, direction)}
        />
      );
  }
});

function FileHeaderRow({
  file,
  expanded,
  focused,
  onToggle,
  onAddComment,
  onCopyPath,
}: {
  file: FileSummary;
  expanded: boolean;
  /** The keyboard cursor (j/k) sits on this file. */
  focused: boolean;
  onToggle: () => void;
  onAddComment: () => void;
  /** Double-click on the file name copies its absolute path. */
  onCopyPath: () => void;
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
        {expanded ? "▾" : "▸"}
      </button>
      <span
        className={`status-badge status-${file.status}`}
        title={file.status}
      >
        {STATUS_LABELS[file.status]}
      </span>
      <span
        className="file-path"
        title={`${file.path}\nDouble-click to copy the full path`}
        onDoubleClick={onCopyPath}
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
  fi,
  hi,
  tokens,
  selected,
  onGutterDown,
  onRowEnter,
}: {
  line: DiffLine;
  fi: number;
  /** Absent for expanded gap-context lines, which are not commentable. */
  hi: number | undefined;
  /** Syntax tokens for this line; absent until its hunk is tokenized. */
  tokens: LineTokens | undefined;
  selected: boolean;
  onGutterDown: (fi: number, hi: number, line: DiffLine, shiftKey: boolean) => void;
  onRowEnter: (fi: number, hi: number, line: DiffLine) => void;
}) {
  const commentable = hi !== undefined;
  const gutterProps = commentable
    ? {
        className: "lineno lineno-commentable",
        onMouseDown: (e: React.MouseEvent) => {
          if (e.button === 0) {
            // Keep the drag from starting a text selection.
            e.preventDefault();
            onGutterDown(fi, hi, line, e.shiftKey);
          }
        },
      }
    : { className: "lineno" };
  return (
    <div
      className={`diff-line diff-line-${line.kind}${selected ? " diff-line-selected" : ""}`}
      onMouseEnter={commentable ? () => onRowEnter(fi, hi, line) : undefined}
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

function errorText(e: unknown): string {
  return typeof e === "string" ? e : String(e);
}
