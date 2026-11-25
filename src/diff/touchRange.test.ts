import { describe, expect, it } from "vitest";
import type { DiffLine, Hunk } from "../types";
import { nudgeRange, sideBounds, touchRange } from "./touchRange";

function line(
  kind: DiffLine["kind"],
  oldLineno: number | null,
  newLineno: number | null,
): DiffLine {
  return { kind, oldLineno, newLineno, content: `${kind} ${oldLineno}/${newLineno}`, intraline: [] } as DiffLine;
}

// old 10..13, new 10..14: two deletions replaced by three additions.
const lines = [
  line("context", 10, 10), // 0
  line("deletion", 11, null), // 1
  line("deletion", 12, null), // 2
  line("addition", null, 11), // 3
  line("addition", null, 12), // 4
  line("addition", null, 13), // 5
  line("context", 13, 14), // 6
];
const hunk: Hunk = {
  header: "@@ -10,4 +10,5 @@",
  oldStart: 10,
  oldLines: 4,
  newStart: 10,
  newLines: 5,
  lines,
};
const later: Hunk = {
  header: "@@ -40,2 +41,3 @@",
  oldStart: 40,
  oldLines: 2,
  newStart: 41,
  newLines: 3,
  lines: [line("context", 40, 41), line("addition", null, 42), line("context", 41, 43)],
};

describe("sideBounds", () => {
  it("spans each side's own numbers", () => {
    expect(sideBounds(hunk, "new")).toEqual({ start: 10, end: 14 });
    expect(sideBounds(hunk, "old")).toEqual({ start: 10, end: 13 });
  });
});

describe("touchRange", () => {
  it("orders the range whichever way the second tap goes", () => {
    expect(touchRange([hunk], { hi: 0, side: "new", line: 10 }, { hi: 0, line: lines[5] }))
      .toEqual({ start: 10, end: 13, clamped: false });
    expect(touchRange([hunk], { hi: 0, side: "new", line: 13 }, { hi: 0, line: lines[0] }))
      .toEqual({ start: 10, end: 13, clamped: false });
  });

  it("maps an other-side tap to the nearest same-side line toward the start", () => {
    // Deletion old 12 sits above the additions: from new 10 it walks up to
    // 10, from new 14 down to the first addition, new 11 — never old 12.
    expect(touchRange([hunk], { hi: 0, side: "new", line: 10 }, { hi: 0, line: lines[2] }))
      .toEqual({ start: 10, end: 10, clamped: false });
    expect(touchRange([hunk], { hi: 0, side: "new", line: 14 }, { hi: 0, line: lines[2] }))
      .toEqual({ start: 11, end: 14, clamped: false });
    // Old side: an addition below old 13's context line resolves to it.
    expect(touchRange([hunk], { hi: 0, side: "old", line: 13 }, { hi: 0, line: lines[4] }))
      .toEqual({ start: 13, end: 13, clamped: false });
    expect(touchRange([hunk], { hi: 0, side: "old", line: 10 }, { hi: 0, line: lines[4] }))
      .toEqual({ start: 10, end: 12, clamped: false });
  });

  it("clamps a tap in another hunk to the start hunk's edge", () => {
    const hunks = [hunk, later];
    expect(touchRange(hunks, { hi: 1, side: "new", line: 43 }, { hi: 0, line: lines[3] }))
      .toEqual({ start: 41, end: 43, clamped: true });
    expect(touchRange(hunks, { hi: 0, side: "old", line: 11 }, { hi: 1, line: later.lines[1] }))
      .toEqual({ start: 11, end: 13, clamped: true });
  });
});

describe("nudgeRange", () => {
  it("moves one end inside the hunk without crossing the other", () => {
    expect(nudgeRange(hunk, "new", { start: 11, end: 12 }, "start", -1)).toEqual({ start: 10, end: 12 });
    expect(nudgeRange(hunk, "new", { start: 10, end: 12 }, "start", -1)).toEqual({ start: 10, end: 12 });
    expect(nudgeRange(hunk, "new", { start: 12, end: 12 }, "start", 1)).toEqual({ start: 12, end: 12 });
    expect(nudgeRange(hunk, "new", { start: 11, end: 12 }, "end", -1)).toEqual({ start: 11, end: 11 });
    // Each side has its own last line: new reaches 14, old stops at 13.
    expect(nudgeRange(hunk, "new", { start: 11, end: 13 }, "end", 1)).toEqual({ start: 11, end: 14 });
    expect(nudgeRange(hunk, "old", { start: 11, end: 13 }, "end", 1)).toEqual({ start: 11, end: 13 });
  });
});
