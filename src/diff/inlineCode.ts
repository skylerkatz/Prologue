/** Splitting guide prose on backtick pairs so `identifiers` can render as
 * <code> spans. Pure string → segments; the React mapping lives with the
 * consumer so this stays trivially unit-testable. */

export interface InlineSegment {
  /** True when the segment renders as a <code> span (backticks stripped). */
  code: boolean;
  text: string;
}

/** Split text on backtick pairs into plain/code segments.
 *
 * Only complete, non-empty pairs become code: an unpaired trailing backtick
 * and empty pairs (``) stay literal text, so the rendered characters are
 * lossless except for the backticks around real code spans. Adjacent plain
 * runs are coalesced; empty input yields no segments.
 */
export function splitInlineCode(text: string): InlineSegment[] {
  const parts = text.split("`");
  const segments: InlineSegment[] = [];
  const pushText = (t: string) => {
    if (t === "") return;
    const last = segments[segments.length - 1];
    if (last !== undefined && !last.code) {
      last.text += t;
    } else {
      segments.push({ code: false, text: t });
    }
  };
  for (let i = 0; i < parts.length; i++) {
    if (i % 2 === 0) {
      pushText(parts[i]);
    } else if (i === parts.length - 1) {
      // Opening backtick with no closer: keep it literal.
      pushText("`" + parts[i]);
    } else if (parts[i] === "") {
      pushText("``");
    } else {
      segments.push({ code: true, text: parts[i] });
    }
  }
  return segments;
}
