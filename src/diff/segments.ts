import type { DiffLine, IntralineRange } from "../types";

/** What a segment needs from a Shiki token (structurally compatible). */
interface StyledToken {
  content: string;
  htmlStyle?: Record<string, string>;
}

/** One renderable slice of a line: syntax style + intraline emphasis. */
export interface LineSegment {
  text: string;
  /** Inside a word-level changed span (stronger diff background). */
  changed: boolean;
  style?: Record<string, string>;
}

/**
 * Merge a line's syntax tokens with its intraline changed ranges into flat
 * segments, splitting tokens at range boundaries so both layers compose.
 * Everything is UTF-16 indexed — token contents tile the line, and ranges
 * come from Rust already in UTF-16 units — so plain `slice` works.
 *
 * Pure and per-visible-line, so it stays virtualization-friendly: cost is
 * proportional to what's on screen, nothing is precomputed per file.
 */
export function segmentLine(
  line: DiffLine,
  tokens: readonly StyledToken[] | undefined,
): LineSegment[] {
  const ranges: IntralineRange[] = line.intraline ?? [];
  const styled: readonly StyledToken[] = tokens ?? [{ content: line.content }];
  if (ranges.length === 0) {
    return styled.map((t) => ({ text: t.content, changed: false, style: t.htmlStyle }));
  }
  const segments: LineSegment[] = [];
  let tokenStart = 0;
  for (const token of styled) {
    const tokenEnd = tokenStart + token.content.length;
    let cursor = tokenStart;
    // Ranges are sorted and disjoint (built left-to-right in Rust).
    for (const range of ranges) {
      if (range.end <= cursor || range.start >= tokenEnd) {
        continue;
      }
      const from = Math.max(range.start, cursor);
      const to = Math.min(range.end, tokenEnd);
      if (from > cursor) {
        segments.push({
          text: token.content.slice(cursor - tokenStart, from - tokenStart),
          changed: false,
          style: token.htmlStyle,
        });
      }
      segments.push({
        text: token.content.slice(from - tokenStart, to - tokenStart),
        changed: true,
        style: token.htmlStyle,
      });
      cursor = to;
    }
    if (cursor < tokenEnd) {
      segments.push({
        text: token.content.slice(cursor - tokenStart),
        changed: false,
        style: token.htmlStyle,
      });
    }
    tokenStart = tokenEnd;
  }
  return segments;
}
