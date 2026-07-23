import { describe, expect, it } from "vitest";
import type { DiffLine, FileDiff, Hunk, LineKind } from "../types";
import {
  blockTargetLine,
  classifyBlock,
  computeMarkers,
  firstChangedLine,
  markersFor,
  type FileMarkers,
} from "./markers";

function line(
  kind: LineKind,
  oldLineno: number | null,
  newLineno: number | null,
  content = "x",
): DiffLine {
  return { kind, oldLineno, newLineno, content };
}

function hunk(over: Partial<Hunk>): Hunk {
  return {
    header: "@@",
    oldStart: 1,
    oldLines: 1,
    newStart: 1,
    newLines: 1,
    lines: [],
    ...over,
  };
}

function diffWith(hunks: Hunk[]): FileDiff {
  return {
    path: "README.md",
    oldPath: null,
    status: "modified",
    binary: false,
    hunks,
    newTotalLines: 100,
  };
}

describe("computeMarkers", () => {
  it("collects a pure addition run", () => {
    const markers = computeMarkers(
      diffWith([
        hunk({
          oldStart: 4,
          oldLines: 2,
          newStart: 4,
          newLines: 5,
          lines: [
            line("context", 4, 4),
            line("addition", null, 5),
            line("addition", null, 6),
            line("addition", null, 7),
            line("context", 5, 8),
          ],
        }),
      ]),
    );
    expect(markers.added).toEqual([{ start: 5, end: 7 }]);
    expect(markers.deletions).toEqual([]);
  });

  it("anchors a mid-hunk deletion after the preceding new-side line", () => {
    const markers = computeMarkers(
      diffWith([
        hunk({
          oldStart: 9,
          oldLines: 4,
          newStart: 9,
          newLines: 2,
          lines: [
            line("context", 9, 9),
            line("deletion", 10, null, "gone-a"),
            line("deletion", 11, null, "gone-b"),
            line("context", 12, 10),
          ],
        }),
      ]),
    );
    expect(markers.added).toEqual([]);
    expect(markers.deletions).toEqual([
      {
        afterLine: 9,
        lines: [
          { oldLineno: 10, content: "gone-a" },
          { oldLineno: 11, content: "gone-b" },
        ],
      },
    ]);
  });

  it("keeps a replacement's deletion anchored before its additions", () => {
    const markers = computeMarkers(
      diffWith([
        hunk({
          oldStart: 5,
          oldLines: 3,
          newStart: 5,
          newLines: 3,
          lines: [
            line("context", 5, 5),
            line("deletion", 6, null, "old"),
            line("addition", null, 6, "new"),
            line("context", 7, 7),
          ],
        }),
      ]),
    );
    expect(markers.added).toEqual([{ start: 6, end: 6 }]);
    expect(markers.deletions).toEqual([
      { afterLine: 5, lines: [{ oldLineno: 6, content: "old" }] },
    ]);
  });

  it("does not split an addition run on an interleaved deletion", () => {
    const markers = computeMarkers(
      diffWith([
        hunk({
          oldStart: 1,
          oldLines: 3,
          newStart: 1,
          newLines: 3,
          lines: [
            line("context", 1, 1),
            line("addition", null, 2),
            line("deletion", 2, null),
            line("addition", null, 3),
          ],
        }),
      ]),
    );
    // New lines 2 and 3 are contiguous on the new side; one run.
    expect(markers.added).toEqual([{ start: 2, end: 3 }]);
    // The deletion after addition 2 anchors after line 2.
    expect(markers.deletions).toEqual([
      { afterLine: 2, lines: [{ oldLineno: 2, content: "x" }] },
    ]);
  });

  it("anchors a deletion at file start before line 1", () => {
    // git reports `@@ -1,2 +0,0 @@` for deleting the first lines with no
    // context surviving on the new side.
    const markers = computeMarkers(
      diffWith([
        hunk({
          oldStart: 1,
          oldLines: 2,
          newStart: 0,
          newLines: 0,
          lines: [line("deletion", 1, null), line("deletion", 2, null)],
        }),
      ]),
    );
    expect(markers.deletions).toEqual([
      {
        afterLine: 0,
        lines: [
          { oldLineno: 1, content: "x" },
          { oldLineno: 2, content: "x" },
        ],
      },
    ]);
  });

  it("splits runs across hunks and context breaks", () => {
    const markers = computeMarkers(
      diffWith([
        hunk({
          oldStart: 1,
          oldLines: 3,
          newStart: 1,
          newLines: 5,
          lines: [
            line("addition", null, 1),
            line("context", 1, 2),
            line("addition", null, 3),
            line("addition", null, 4),
            line("context", 2, 5),
            line("deletion", 3, null),
          ],
        }),
        hunk({
          oldStart: 20,
          oldLines: 1,
          newStart: 22,
          newLines: 2,
          lines: [line("context", 20, 22), line("addition", null, 23)],
        }),
      ]),
    );
    expect(markers.added).toEqual([
      { start: 1, end: 1 },
      { start: 3, end: 4 },
      { start: 23, end: 23 },
    ]);
    expect(markers.deletions).toEqual([
      { afterLine: 5, lines: [{ oldLineno: 3, content: "x" }] },
    ]);
  });
});

describe("classifyBlock", () => {
  const markers: FileMarkers = {
    added: [{ start: 10, end: 14 }],
    deletions: [{ afterLine: 30, lines: [{ oldLineno: 31, content: "x" }] }],
  };

  it("marks a block fully inside an added run as added", () => {
    expect(classifyBlock(markers, 10, 14)).toBe("added");
    expect(classifyBlock(markers, 11, 13)).toBe("added");
  });

  it("marks a partially overlapping block as modified", () => {
    expect(classifyBlock(markers, 8, 11)).toBe("modified");
    expect(classifyBlock(markers, 14, 20)).toBe("modified");
    expect(classifyBlock(markers, 5, 40)).toBe("modified");
  });

  it("marks a block with a deletion strictly inside as modified", () => {
    expect(classifyBlock(markers, 28, 34)).toBe("modified");
    expect(classifyBlock(markers, 30, 31)).toBe("modified");
  });

  it("leaves deletions at block edges to the tombstone between blocks", () => {
    // Deletion after the block's last line, or just before its first.
    expect(classifyBlock(markers, 25, 30)).toBe(null);
    expect(classifyBlock(markers, 31, 35)).toBe(null);
  });

  it("leaves untouched blocks unmarked", () => {
    expect(classifyBlock(markers, 1, 9)).toBe(null);
    expect(classifyBlock(markers, 15, 25)).toBe(null);
  });

  it("prefers added over modified when one run covers and another touches", () => {
    const two: FileMarkers = {
      added: [
        { start: 10, end: 14 },
        { start: 3, end: 4 },
      ],
      deletions: [],
    };
    expect(classifyBlock(two, 11, 12)).toBe("added");
  });
});

describe("blockTargetLine", () => {
  const markers: FileMarkers = {
    added: [
      { start: 10, end: 14 },
      { start: 20, end: 21 },
    ],
    deletions: [{ afterLine: 30, lines: [] }],
  };

  it("targets the first added line inside the block", () => {
    expect(blockTargetLine(markers, 8, 25)).toBe(10);
    expect(blockTargetLine(markers, 12, 25)).toBe(12);
    expect(blockTargetLine(markers, 16, 25)).toBe(20);
  });

  it("falls back to the deletion anchor line", () => {
    expect(blockTargetLine(markers, 28, 34)).toBe(30);
  });

  it("clamps a before-first-line anchor to line 1", () => {
    const atStart: FileMarkers = {
      added: [],
      deletions: [{ afterLine: 0, lines: [] }],
    };
    expect(blockTargetLine(atStart, 0, 3)).toBe(1);
  });

  it("returns null for untouched blocks", () => {
    expect(blockTargetLine(markers, 1, 9)).toBe(null);
  });
});

describe("firstChangedLine", () => {
  it("takes the earliest change across additions and deletions", () => {
    expect(
      firstChangedLine({
        added: [{ start: 12, end: 14 }],
        deletions: [{ afterLine: 5, lines: [] }],
      }),
    ).toBe(5);
    expect(
      firstChangedLine({
        added: [{ start: 3, end: 4 }],
        deletions: [{ afterLine: 8, lines: [] }],
      }),
    ).toBe(3);
  });

  it("clamps a start-of-file deletion to line 1 and handles no changes", () => {
    expect(
      firstChangedLine({
        added: [],
        deletions: [{ afterLine: 0, lines: [] }],
      }),
    ).toBe(1);
    expect(firstChangedLine({ added: [], deletions: [] })).toBe(null);
  });
});

describe("markersFor", () => {
  it("caches per diff object identity", () => {
    const diff = diffWith([
      hunk({
        newStart: 1,
        newLines: 1,
        lines: [line("addition", null, 1)],
      }),
    ]);
    const first = markersFor(diff);
    expect(markersFor(diff)).toBe(first);
    expect(first.added).toEqual([{ start: 1, end: 1 }]);
  });
});
