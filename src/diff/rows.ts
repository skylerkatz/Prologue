import type {
  Comment,
  CommentSide,
  DiffLine,
  FileDiff,
  FileSummary,
  RepliesByRoot,
} from "../types";
import { guardReason, type GuardReason } from "./guards";

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
}

/** Reviewed files start collapsed (`expanded: false`); everything else open. */
export function initialFileState(expanded: boolean): FileViewState {
  return {
    expanded,
    forceLoad: false,
    diff: null,
    error: null,
    reveals: [],
    context: new Map(),
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
 * Review-level composing lives outside the virtualized list.
 */
export type ComposerLocation =
  | { level: "file"; fi: number }
  | {
      level: "line";
      fi: number;
      side: CommentSide;
      startLine: number;
      endLine: number;
    }
  | { level: "reply"; fi: number; rootId: number };

const ROW_HEIGHTS: Record<Exclude<Row["kind"], "skeleton">, number> = {
  file: 42,
  notice: 60,
  error: 48,
  empty: 40,
  hiddenComments: 40,
  hunk: 26,
  line: LINE_HEIGHT,
  comment: 96,
  composer: 150,
  expand: 26,
};

export function estimateRowHeight(row: Row): number {
  return row.kind === "skeleton" ? row.height : ROW_HEIGHTS[row.kind];
}

/** Stable identity so measurements survive rows shifting on load/expand. */
export function rowKey(row: Row): string {
  switch (row.kind) {
    case "file":
      return `f${row.fi}`;
    case "notice":
      return `n${row.fi}`;
    case "skeleton":
      return `s${row.fi}`;
    case "error":
      return `e${row.fi}`;
    case "empty":
      return `m${row.fi}`;
    case "hiddenComments":
      return `w${row.fi}`;
    case "hunk":
      return `h${row.fi}:${row.hi}`;
    case "comment":
      return `c${row.comment.id}`;
    // Only one composer exists at a time.
    case "composer":
      return "composer";
    case "expand":
      return `x${row.fi}:${row.gi}`;
    case "line":
      return `l${row.fi}:${row.line.oldLineno ?? ""}:${row.line.newLineno ?? ""}`;
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
      // Not part of the current diff (e.g. other working-tree mode); the
      // sidebar count still includes it.
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
  states: FileViewState[],
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
    const state = states[fi];
    if (!state.expanded) {
      return;
    }
    const fileComments = comments.get(fi);
    for (const comment of fileComments?.file ?? []) {
      pushThread(fi, comment);
    }
    if (composer?.level === "file" && composer.fi === fi) {
      rows.push({ kind: "composer", fi });
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
      composer?.level === "line" && composer.fi === fi ? composer : null;
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
