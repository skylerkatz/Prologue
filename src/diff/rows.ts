import type {
  Comment,
  CommentSide,
  DiffLine,
  FileDiff,
  FileReviewState,
  FileSummary,
  RepliesByRoot,
} from "../types";
import { guardReason, type GuardReason } from "./guards";
import { detectLang } from "../highlight/lang";

/** Fixed row height for diff lines; keeps scroll estimates honest. */
const LINE_HEIGHT = 21;

/** How many unchanged lines one expand-context click reveals. */
export const CONTEXT_CHUNK = 20;

/** Lines revealed so far at each edge of a gap between hunks. */
export interface GapReveal {
  /** Revealed at the top of the gap (just below the hunk above). */
  top: number;
  /** Revealed at the bottom of the gap (just above the hunk below). */
  bottom: number;
}

/** Per-file UI state driving which rows the file contributes. */
export interface FileViewState {
  expanded: boolean;
  /** Guarded file whose "Load diff" was clicked. */
  forceLoad: boolean;
  diff: FileDiff | null;
  error: string | null;
  /** Parallel to `computeGaps(diff)` once the diff is loaded. */
  reveals: GapReveal[];
  /** Fetched unchanged new-side lines, keyed by new line number. */
  context: Map<number, string>;
  /** Rendered markdown preview instead of the source diff. Only ever true
   * for [`previewEligible`] files. */
  previewMode: boolean;
  /** Full new-side text backing the rendered preview; null until fetched. */
  fileContent: string | null;
  /** Preview content fetch failure, shown inside the preview row. */
  previewError: string | null;
}

/** Whether the file gets the source/rich toggle and rich-by-default: a
 * markdown document with a new side to render. Deleted files have no new
 * side; binary files never render. */
export function previewEligible(file: FileSummary): boolean {
  return (
    !file.binary &&
    file.status !== "deleted" &&
    detectLang(file.path) === "markdown"
  );
}

/**
 * Files whose reviewed-ness flipped between two review-state maps, with the
 * `expanded` value each card should take: marking collapses, unmarking
 * re-expands, and "changed since review" counts as unreviewed (the file
 * needs re-reviewing). In `files` order, so the first collapsed entry is
 * the scroll anchor for a batch mark.
 */
export function reviewedFlips(
  files: readonly FileSummary[],
  prev: ReadonlyMap<string, FileReviewState>,
  next: ReadonlyMap<string, FileReviewState>,
): { path: string; expanded: boolean }[] {
  const flips: { path: string; expanded: boolean }[] = [];
  for (const file of files) {
    const was = prev.get(file.path) === "reviewed";
    const is = next.get(file.path) === "reviewed";
    if (was !== is) {
      flips.push({ path: file.path, expanded: !is });
    }
  }
  return flips;
}

/** Reviewed files start collapsed (`expanded: false`); everything else open.
 * Eligible markdown files start in the rendered preview (`previewMode`). */
export function initialFileState(
  expanded: boolean,
  previewMode = false,
): FileViewState {
  return {
    expanded,
    forceLoad: false,
    diff: null,
    error: null,
    reveals: [],
    context: new Map(),
    previewMode,
    fileContent: null,
    previewError: null,
  };
}

/**
 * A run of unchanged lines outside any hunk, in new-side line numbers.
 * Gap `i` sits above hunk `i`; the last gap follows the last hunk.
 */
export interface Gap {
  /** First new-side line of the gap (1-based). */
  start: number;
  /** Last new-side line, inclusive; `end < start` means an empty gap. */
  end: number;
  /** Old lineno = new lineno − oldOffset, constant within a gap. */
  oldOffset: number;
}

export function computeGaps(diff: FileDiff): Gap[] {
  if (diff.newTotalLines === null || diff.hunks.length === 0) {
    return [];
  }
  const gaps: Gap[] = [];
  const first = diff.hunks[0];
  gaps.push({
    start: 1,
    end: first.newStart - 1,
    oldOffset: first.newStart - first.oldStart,
  });
  for (let i = 1; i < diff.hunks.length; i++) {
    const prev = diff.hunks[i - 1];
    const next = diff.hunks[i];
    gaps.push({
      start: prev.newStart + prev.newLines,
      end: next.newStart - 1,
      oldOffset: next.newStart - next.oldStart,
    });
  }
  const last = diff.hunks[diff.hunks.length - 1];
  gaps.push({
    start: last.newStart + last.newLines,
    end: diff.newTotalLines,
    oldOffset: last.newStart + last.newLines - (last.oldStart + last.oldLines),
  });
  return gaps;
}

export type Row =
  | { kind: "file"; fi: number }
  | { kind: "notice"; fi: number; reason: GuardReason }
  | { kind: "skeleton"; fi: number; height: number }
  | { kind: "error"; fi: number; message: string }
  /** `whitespaceHidden` relabels the body: the file has changes, they are
   * all whitespace and the hide-whitespace toggle dropped them. */
  | { kind: "empty"; fi: number; whitespaceHidden: boolean }
  /** Line comments whose host line disappeared under hide-whitespace. */
  | { kind: "hiddenComments"; fi: number; count: number }
  /** Rendered markdown preview of the whole new side, replacing the file's
   * hunk/line rows while `previewMode` is on. One measured row. */
  | { kind: "preview"; fi: number }
  | { kind: "hunk"; fi: number; hi: number; header: string }
  /** `hi`/`li` (hunk index, line index within the hunk) are set for hunk
   * lines; expanded gap-context lines carry neither. */
  | { kind: "line"; fi: number; line: DiffLine; hi?: number; li?: number }
  | { kind: "comment"; fi: number; comment: Comment }
  | { kind: "composer"; fi: number }
  | {
      kind: "expand";
      fi: number;
      gi: number;
      hidden: number;
      /** A hunk sits above the gap, so it can grow downward from it. */
      growTop: boolean;
      /** A hunk sits below the gap, so it can grow upward from it. */
      growBottom: boolean;
    };

/**
 * Where the single open comment composer sits: after a file's header, below
 * the selection's last line, or under a thread root's last reply.
 * Review-level composing lives outside the virtualized list. Keyed by file
 * path (not display index) so it survives refreshes that reorder files.
 */
export type ComposerLocation =
  | { level: "file"; path: string }
  | {
      level: "line";
      path: string;
      side: CommentSide;
      startLine: number;
      endLine: number;
    }
  | { level: "reply"; path: string; rootId: number };

const ROW_HEIGHTS: Record<Exclude<Row["kind"], "skeleton">, number> = {
  file: 42,
  notice: 60,
  error: 48,
  empty: 40,
  hiddenComments: 40,
  // A guess; the virtualizer measures the rendered document immediately.
  preview: 400,
  hunk: 26,
  line: LINE_HEIGHT,
  comment: 96,
  composer: 150,
  expand: 26,
};

export function estimateRowHeight(row: Row): number {
  return row.kind === "skeleton" ? row.height : ROW_HEIGHTS[row.kind];
}

/** Stable identity so measurements survive rows shifting on load/expand —
 * and across refreshes: keys are path-based, so files added or removed by a
 * new diff never shift another file's row identities. */
export function rowKey(row: Row, files: FileSummary[]): string {
  const path = files[row.fi].path;
  switch (row.kind) {
    case "file":
      return `f${path}`;
    case "notice":
      return `n${path}`;
    case "skeleton":
      return `s${path}`;
    case "error":
      return `e${path}`;
    case "empty":
      return `m${path}`;
    case "hiddenComments":
      return `w${path}`;
    case "preview":
      return `p${path}`;
    case "hunk":
      return `h${path}:${row.hi}`;
    case "comment":
      return `c${row.comment.id}`;
    // Only one composer exists at a time.
    case "composer":
      return "composer";
    case "expand":
      return `x${path}:${row.gi}`;
    case "line":
      return `l${path}:${row.line.oldLineno ?? ""}:${row.line.newLineno ?? ""}`;
  }
}

/** The side a diff line's comments belong to (context lines count as new). */
export function lineSide(line: DiffLine): CommentSide {
  return line.kind === "deletion" ? "old" : "new";
}

/** The line number on the side given by [`lineSide`]. */
export function lineNumber(line: DiffLine): number {
  return (line.kind === "deletion" ? line.oldLineno : line.newLineno) ?? 0;
}

/** Line-level comments of one file keyed by `side:endLine`, in id order. */
export type LineCommentIndex = Map<string, Comment[]>;

export interface FileComments {
  /** File-level comments in id order. */
  file: Comment[];
  line: LineCommentIndex;
}

const lineCommentKey = (side: CommentSide, endLine: number): string =>
  `${side}:${endLine}`;

/**
 * Group file- and line-level comments by file index so `buildRows` can place
 * them without scanning the comment list per row.
 */
export function indexComments(
  files: FileSummary[],
  comments: Comment[],
): Map<number, FileComments> {
  const byPath = new Map(files.map((file, fi) => [file.path, fi]));
  const index = new Map<number, FileComments>();
  for (const comment of comments) {
    // Replies never place themselves: they render under their thread root
    // (buildRows appends them from the replies map).
    if (
      comment.parentId !== null ||
      comment.level === "review" ||
      comment.filePath === null
    ) {
      continue;
    }
    const fi = byPath.get(comment.filePath);
    if (fi === undefined) {
      // Not part of the current diff (e.g. other working-tree mode); it
      // surfaces in the orphaned-comments bucket above the diff instead.
      continue;
    }
    let entry = index.get(fi);
    if (entry === undefined) {
      entry = { file: [], line: new Map() };
      index.set(fi, entry);
    }
    if (
      comment.level === "line" &&
      comment.side !== null &&
      comment.endLine !== null
    ) {
      const key = lineCommentKey(comment.side, comment.endLine);
      const bucket = entry.line.get(key);
      if (bucket === undefined) {
        entry.line.set(key, [comment]);
      } else {
        bucket.push(comment);
      }
    } else {
      entry.file.push(comment);
    }
  }
  return index;
}

/** Approximate body height for a file whose hunks are still loading. */
function skeletonHeight(file: FileSummary): number {
  return (file.additions + file.deletions + 4) * LINE_HEIGHT;
}

/**
 * Flatten the whole diff — every file — into one list of virtualizable rows.
 * Only rows the virtualizer asks for are ever rendered. Comment, reply, and
 * composer rows join the same list, so the DOM stays bounded however many
 * exist.
 */
export function buildRows(
  files: FileSummary[],
  states: ReadonlyMap<string, FileViewState>,
  comments: Map<number, FileComments>,
  replies: RepliesByRoot,
  composer: ComposerLocation | null,
  /** The hide-whitespace toggle the diffs were fetched with; drives the
   * empty-state relabel and the hidden-comments indicator. */
  ignoreWhitespace: boolean,
): Row[] {
  const rows: Row[] = [];
  // A thread root followed by its replies and (if open here) the reply
  // composer. Resolved/dismissed roots collapse the whole thread: replies
  // contribute no rows until the root is reopened.
  const pushThread = (fi: number, comment: Comment) => {
    rows.push({ kind: "comment", fi, comment });
    if (comment.state !== "open") {
      return;
    }
    for (const reply of replies.get(comment.id) ?? []) {
      rows.push({ kind: "comment", fi, comment: reply });
    }
    if (composer?.level === "reply" && composer.rootId === comment.id) {
      rows.push({ kind: "composer", fi });
    }
  };
  files.forEach((file, fi) => {
    rows.push({ kind: "file", fi });
    const state = states.get(file.path);
    if (state === undefined || !state.expanded) {
      return;
    }
    const fileComments = comments.get(fi);
    for (const comment of fileComments?.file ?? []) {
      pushThread(fi, comment);
    }
    if (composer?.level === "file" && composer.path === file.path) {
      rows.push({ kind: "composer", fi });
    }
    // Rich view is a read mode over the new-side document: it needs no
    // diff, so it bypasses the guard/skeleton/hunk rows entirely. Line
    // comments have no host row here — they live in the sidebar and in
    // source view, one toggle away.
    if (state.previewMode) {
      rows.push({ kind: "preview", fi });
      return;
    }
    const guard = guardReason(file);
    if (guard === "binary") {
      rows.push({ kind: "notice", fi, reason: guard });
      return;
    }
    if (guard !== null && !state.forceLoad && state.diff === null) {
      rows.push({ kind: "notice", fi, reason: guard });
      return;
    }
    if (state.error !== null) {
      rows.push({ kind: "error", fi, message: state.error });
      return;
    }
    const diff = state.diff;
    if (diff === null) {
      rows.push({ kind: "skeleton", fi, height: skeletonHeight(file) });
      return;
    }
    // With hide-whitespace on, comments on lines inside dropped hunks have
    // no host row below and would silently vanish from the inline view;
    // count them so an indicator row can point at the toggle. They are not
    // orphans — the sidebar still lists them.
    let hiddenComments = 0;
    if (
      ignoreWhitespace &&
      fileComments !== undefined &&
      fileComments.line.size > 0
    ) {
      const hosts = new Set<string>();
      for (const hunk of diff.hunks) {
        for (const line of hunk.lines) {
          hosts.add(lineCommentKey(lineSide(line), lineNumber(line)));
        }
      }
      for (const [key, bucket] of fileComments.line) {
        if (!hosts.has(key)) {
          hiddenComments += bucket.length;
        }
      }
    }
    if (diff.hunks.length === 0) {
      rows.push({ kind: "empty", fi, whitespaceHidden: ignoreWhitespace });
      if (hiddenComments > 0) {
        rows.push({ kind: "hiddenComments", fi, count: hiddenComments });
      }
      return;
    }
    if (hiddenComments > 0) {
      rows.push({ kind: "hiddenComments", fi, count: hiddenComments });
    }
    const lineComposer =
      composer?.level === "line" && composer.path === file.path
        ? composer
        : null;
    const gaps = computeGaps(diff);
    diff.hunks.forEach((hunk, hi) => {
      if (gaps.length > 0) {
        pushGapRows(rows, fi, hi, gaps[hi], state, diff.hunks.length);
      }
      rows.push({ kind: "hunk", fi, hi, header: hunk.header });
      hunk.lines.forEach((line, li) => {
        rows.push({ kind: "line", fi, line, hi, li });
        const key = lineCommentKey(lineSide(line), lineNumber(line));
        for (const comment of fileComments?.line.get(key) ?? []) {
          pushThread(fi, comment);
        }
        if (
          lineComposer !== null &&
          lineCommentKey(lineComposer.side, lineComposer.endLine) === key
        ) {
          rows.push({ kind: "composer", fi });
        }
      });
    });
    if (gaps.length > 0) {
      const gi = diff.hunks.length;
      pushGapRows(rows, fi, gi, gaps[gi], state, diff.hunks.length);
    }
  });
  return rows;
}

function pushGapRows(
  rows: Row[],
  fi: number,
  gi: number,
  gap: Gap,
  state: FileViewState,
  hunkCount: number,
): void {
  if (gap.end < gap.start) {
    return;
  }
  const reveal = state.reveals[gi] ?? { top: 0, bottom: 0 };
  const topEnd = gap.start + reveal.top - 1;
  const bottomStart = gap.end - reveal.bottom + 1;

  const pushContext = (from: number, to: number) => {
    for (let n = from; n <= to; n++) {
      rows.push({
        kind: "line",
        fi,
        line: {
          kind: "context",
          oldLineno: n - gap.oldOffset,
          newLineno: n,
          content: state.context.get(n) ?? "",
        },
      });
    }
  };

  if (topEnd + 1 >= bottomStart) {
    pushContext(gap.start, gap.end);
    return;
  }
  pushContext(gap.start, topEnd);
  rows.push({
    kind: "expand",
    fi,
    gi,
    hidden: bottomStart - topEnd - 1,
    growTop: gi > 0,
    growBottom: gi < hunkCount,
  });
  pushContext(bottomStart, gap.end);
}
