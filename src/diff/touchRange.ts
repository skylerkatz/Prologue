import type { CommentSide, DiffLine, Hunk } from "../types";
import { lineNumber, lineSide } from "./rows";

/** A touch range in progress: the start line, by hunk and side. */
export interface TouchAnchor {
  hi: number;
  side: CommentSide;
  line: number;
}

export interface LineRange {
  start: number;
  end: number;
}

function numberOn(line: DiffLine, side: CommentSide): number | null {
  return side === "old" ? line.oldLineno : line.newLineno;
}

/** The first and last line numbers `side` has in `hunk`; the numbers in
 * between are contiguous (context plus additions, or plus deletions). */
export function sideBounds(hunk: Hunk, side: CommentSide): LineRange | null {
  let start: number | null = null;
  let end: number | null = null;
  for (const line of hunk.lines) {
    const n = numberOn(line, side);
    if (n !== null) {
      start ??= n;
      end = n;
    }
  }
  return start === null || end === null ? null : { start, end };
}

function indexOf(hunk: Hunk, line: DiffLine): number {
  return hunk.lines.findIndex(
    (l) =>
      l === line ||
      (l.kind === line.kind &&
        l.oldLineno === line.oldLineno &&
        l.newLineno === line.newLineno),
  );
}

/**
 * The range a range-mode tap sets. Ranges obey the desktop drag's rules —
 * one hunk, one side — because the server anchors nothing else. A tap in
 * another hunk clamps to the start hunk's edge on that side (`clamped`);
 * a tap on the other side within the hunk (a deletion when the range is on
 * the new side, or the reverse) takes the nearest same-side line toward
 * the start, so the range never grows past the line tapped.
 */
export function touchRange(
  hunks: Hunk[],
  anchor: TouchAnchor,
  tap: { hi: number; line: DiffLine },
): (LineRange & { clamped: boolean }) | null {
  const hunk = hunks[anchor.hi];
  const bounds = hunk === undefined ? null : sideBounds(hunk, anchor.side);
  if (hunk === undefined || bounds === null) {
    return null;
  }
  let n: number;
  let clamped = false;
  if (tap.hi !== anchor.hi) {
    n = tap.hi < anchor.hi ? bounds.start : bounds.end;
    clamped = true;
  } else if (lineSide(tap.line) === anchor.side) {
    n = lineNumber(tap.line);
  } else {
    n = anchor.line;
    const from = indexOf(hunk, tap.line);
    const to = hunk.lines.findIndex(
      (l) => numberOn(l, anchor.side) === anchor.line,
    );
    if (from !== -1 && to !== -1) {
      const step = to > from ? 1 : -1;
      for (let i = from; i !== to + step; i += step) {
        const found = numberOn(hunk.lines[i], anchor.side);
        if (found !== null) {
          n = found;
          break;
        }
      }
    }
  }
  return {
    start: Math.min(anchor.line, n),
    end: Math.max(anchor.line, n),
    clamped,
  };
}

/** Move one end of a range by `delta` lines, staying inside the hunk's
 * lines on `side` and never crossing the other end. */
export function nudgeRange(
  hunk: Hunk,
  side: CommentSide,
  range: LineRange,
  edge: "start" | "end",
  delta: number,
): LineRange {
  const bounds = sideBounds(hunk, side) ?? range;
  const clamp = (n: number, lo: number, hi: number) =>
    Math.max(lo, Math.min(hi, n));
  return edge === "start"
    ? { start: clamp(range.start + delta, bounds.start, range.end), end: range.end }
    : { start: range.start, end: clamp(range.end + delta, range.start, bounds.end) };
}
