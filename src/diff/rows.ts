import type { DiffLine, FileDiff, FileSummary } from "../types";
import { guardReason, type GuardReason } from "./guards";

/** Fixed row height for diff lines; keeps scroll estimates honest. */
export const LINE_HEIGHT = 21;

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

export function initialFileState(): FileViewState {
  return {
    expanded: true,
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
  | { kind: "empty"; fi: number }
  | { kind: "hunk"; fi: number; hi: number; header: string }
  | { kind: "line"; fi: number; line: DiffLine }
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

const ROW_HEIGHTS: Record<Exclude<Row["kind"], "skeleton">, number> = {
  file: 42,
  notice: 60,
  error: 48,
  empty: 40,
  hunk: 26,
  line: LINE_HEIGHT,
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
    case "hunk":
      return `h${row.fi}:${row.hi}`;
    case "expand":
      return `x${row.fi}:${row.gi}`;
    case "line":
      return `l${row.fi}:${row.line.oldLineno ?? ""}:${row.line.newLineno ?? ""}`;
  }
}

/** Approximate body height for a file whose hunks are still loading. */
function skeletonHeight(file: FileSummary): number {
  return (file.additions + file.deletions + 4) * LINE_HEIGHT;
}

/**
 * Flatten the whole diff — every file — into one list of virtualizable rows.
 * Only rows the virtualizer asks for are ever rendered.
 */
export function buildRows(
  files: FileSummary[],
  states: FileViewState[],
): Row[] {
  const rows: Row[] = [];
  files.forEach((file, fi) => {
    rows.push({ kind: "file", fi });
    const state = states[fi];
    if (!state.expanded) {
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
    if (diff.hunks.length === 0) {
      rows.push({ kind: "empty", fi });
      return;
    }
    const gaps = computeGaps(diff);
    diff.hunks.forEach((hunk, hi) => {
      if (gaps.length > 0) {
        pushGapRows(rows, fi, hi, gaps[hi], state, diff.hunks.length);
      }
      rows.push({ kind: "hunk", fi, hi, header: hunk.header });
      for (const line of hunk.lines) {
        rows.push({ kind: "line", fi, line });
      }
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
