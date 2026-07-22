import { describe, expect, it } from "vitest";
import type {
  Comment,
  DiffLine,
  FileDiff,
  FileReviewState,
  FileSummary,
  Hunk,
  LineKind,
} from "../types";
import {
  buildRows,
  computeGaps,
  estimateRowHeight,
  indexComments,
  initialFileState,
  lineNumber,
  lineSide,
  reviewedFlips,
  rowKey,
  type FileViewState,
  type Row,
} from "./rows";

function file(over: Partial<FileSummary> = {}): FileSummary {
  return {
    path: "src/a.ts",
    oldPath: null,
    status: "modified",
    additions: 3,
    deletions: 1,
    binary: false,
    fingerprint: "fp-a",
    ...over,
  };
}

function line(
  kind: LineKind,
  oldLineno: number | null,
  newLineno: number | null,
  content = "x",
): DiffLine {
  return { kind, oldLineno, newLineno, content };
}

/** One hunk: context 10/10, deletion old 11, additions new 11–12. */
function standardHunk(): Hunk {
  return {
    header: "@@ -10,2 +10,3 @@",
    oldStart: 10,
    oldLines: 2,
    newStart: 10,
    newLines: 3,
    lines: [
      line("context", 10, 10, "ctx"),
      line("deletion", 11, null, "removed"),
      line("addition", null, 11, "added1"),
      line("addition", null, 12, "added2"),
    ],
  };
}

function standardDiff(over: Partial<FileDiff> = {}): FileDiff {
  return {
    path: "src/a.ts",
    oldPath: null,
    status: "modified",
    binary: false,
    hunks: [standardHunk()],
    newTotalLines: 40,
    ...over,
  };
}

function loadedState(diff: FileDiff): FileViewState {
  return {
    ...initialFileState(true),
    diff,
    reveals: Array.from({ length: diff.hunks.length + 1 }, () => ({
      top: 0,
      bottom: 0,
    })),
  };
}

function comment(over: Partial<Comment> = {}): Comment {
  return {
    id: 1,
    reviewId: 1,
    level: "line",
    filePath: "src/a.ts",
    side: "new",
    startLine: 11,
    endLine: 11,
    codeAnchor: null,
    commitSha: "abc",
    state: "open",
    body: "note",
    parentId: null,
    author: "reviewer",
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...over,
  };
}

function statesFor(
  files: FileSummary[],
  state: FileViewState,
): Map<string, FileViewState> {
  return new Map(files.map((f) => [f.path, state]));
}

const noComments = new Map<number, never>();
const noReplies = new Map<number, Comment[]>();

function kinds(rows: Row[]): string[] {
  return rows.map((r) => r.kind);
}

describe("computeGaps", () => {
  it("returns no gaps without hunks or without a new-side total", () => {
    expect(computeGaps(standardDiff({ hunks: [] }))).toEqual([]);
    expect(computeGaps(standardDiff({ newTotalLines: null }))).toEqual([]);
  });

  it("brackets every hunk with gaps carrying the old-side offset", () => {
    const second: Hunk = {
      header: "@@ -19,1 +20,1 @@",
      oldStart: 19,
      oldLines: 1,
      newStart: 20,
      newLines: 1,
      lines: [line("addition", null, 20)],
    };
    const gaps = computeGaps(standardDiff({ hunks: [standardHunk(), second] }));
    expect(gaps).toEqual([
      // Above the first hunk: new 1..9, old lineno = new lineno.
      { start: 1, end: 9, oldOffset: 0 },
      // Between: new 13..19; the first hunk added a net line.
      { start: 13, end: 19, oldOffset: 1 },
      // Below the last hunk to the end of the file.
      { start: 21, end: 40, oldOffset: 1 },
    ]);
  });
});

describe("buildRows: per-file gating", () => {
  it("renders only the header for collapsed or unknown files", () => {
    const files = [file()];
    const collapsed = buildRows(
      files,
      statesFor(files, initialFileState(false)),
      noComments,
      noReplies,
      null,
      false,
    );
    expect(kinds(collapsed)).toEqual(["file"]);
    // Post-474 the state map is path-keyed; a file with no entry yet (e.g.
    // mid-reconcile) must degrade to a plain header, not crash.
    const missing = buildRows(files, new Map(), noComments, noReplies, null, false);
    expect(kinds(missing)).toEqual(["file"]);
  });

  it("shows a sized skeleton while an expanded file's hunks load", () => {
    const files = [file({ additions: 5, deletions: 2 })];
    const rows = buildRows(
      files,
      statesFor(files, initialFileState(true)),
      noComments,
      noReplies,
      null,
      false,
    );
    expect(rows[1]).toEqual({ kind: "skeleton", fi: 0, height: (5 + 2 + 4) * 21 });
    expect(estimateRowHeight(rows[1])).toBe((5 + 2 + 4) * 21);
  });

  it("labels an empty loaded diff, attributing it to hide-whitespace when on", () => {
    const files = [file()];
    const states = statesFor(files, loadedState(standardDiff({ hunks: [] })));
    const plain = buildRows(files, states, noComments, noReplies, null, false);
    expect(plain[1]).toEqual({ kind: "empty", fi: 0, whitespaceHidden: false });
    const hidden = buildRows(files, states, noComments, noReplies, null, true);
    expect(hidden[1]).toEqual({ kind: "empty", fi: 0, whitespaceHidden: true });
  });

  it("notices binary files even when force-loaded", () => {
    const files = [file({ binary: true })];
    const forced = { ...initialFileState(true), forceLoad: true };
    const rows = buildRows(files, statesFor(files, forced), noComments, noReplies, null, false);
    expect(rows[1]).toEqual({ kind: "notice", fi: 0, reason: "binary" });
  });

  it("guards oversize and generated files until force-loaded", () => {
    const big = [file({ additions: 5001, deletions: 0 })];
    const bigRows = buildRows(
      big,
      statesFor(big, initialFileState(true)),
      noComments,
      noReplies,
      null,
      false,
    );
    expect(bigRows[1]).toEqual({ kind: "notice", fi: 0, reason: "oversize" });

    const lock = [file({ path: "package-lock.json" })];
    const lockRows = buildRows(
      lock,
      statesFor(lock, initialFileState(true)),
      noComments,
      noReplies,
      null,
      false,
    );
    expect(lockRows[1]).toEqual({ kind: "notice", fi: 0, reason: "generated" });

    // Force-load falls through to the loading skeleton.
    const forced = { ...initialFileState(true), forceLoad: true };
    const forcedRows = buildRows(big, statesFor(big, forced), noComments, noReplies, null, false);
    expect(kinds(forcedRows)).toEqual(["file", "skeleton"]);
  });

  it("surfaces a file's load error as its only body row", () => {
    const files = [file()];
    const errored = { ...initialFileState(true), error: "boom" };
    const rows = buildRows(files, statesFor(files, errored), noComments, noReplies, null, false);
    expect(rows[1]).toEqual({ kind: "error", fi: 0, message: "boom" });
  });
});

describe("buildRows: hunks, gaps, and expansion", () => {
  it("collapses unrevealed gaps into expand rows with edge-growth flags", () => {
    const files = [file()];
    const rows = buildRows(
      files,
      statesFor(files, loadedState(standardDiff())),
      noComments,
      noReplies,
      null,
      false,
    );
    expect(kinds(rows)).toEqual([
      "file",
      "expand", // new 1..9 above the hunk
      "hunk",
      "line",
      "line",
      "line",
      "line",
      "expand", // new 13..40 below it
    ]);
    // A gap above the first hunk can only grow downward from the hunk edge.
    expect(rows[1]).toMatchObject({ gi: 0, hidden: 9, growTop: false, growBottom: true });
    // A gap below the last hunk can only grow upward.
    expect(rows[7]).toMatchObject({ gi: 1, hidden: 28, growTop: true, growBottom: false });
  });

  it("materializes revealed edges as context lines with derived old linenos", () => {
    const state = loadedState(standardDiff());
    state.reveals = [
      { top: 2, bottom: 1 },
      { top: 0, bottom: 0 },
    ];
    state.context = new Map([
      [1, "l1"],
      [2, "l2"],
      [9, "l9"],
    ]);
    const files = [file()];
    const rows = buildRows(files, statesFor(files, state), noComments, noReplies, null, false);
    // Gap 0 now renders: revealed 1..2, expand(6 hidden), revealed 9.
    expect(rows[1]).toMatchObject({
      kind: "line",
      line: { kind: "context", oldLineno: 1, newLineno: 1, content: "l1" },
    });
    expect(rows[2]).toMatchObject({
      kind: "line",
      line: { kind: "context", oldLineno: 2, newLineno: 2, content: "l2" },
    });
    expect(rows[3]).toMatchObject({ kind: "expand", gi: 0, hidden: 6 });
    expect(rows[4]).toMatchObject({
      kind: "line",
      line: { kind: "context", oldLineno: 9, newLineno: 9, content: "l9" },
    });
    // Un-fetched revealed lines fall back to empty content, never undefined.
    const bare = loadedState(standardDiff());
    bare.reveals = [{ top: 1, bottom: 0 }, { top: 0, bottom: 0 }];
    const bareRows = buildRows(files, statesFor(files, bare), noComments, noReplies, null, false);
    expect(bareRows[1]).toMatchObject({ kind: "line", line: { content: "" } });
  });

  it("merges a fully-revealed gap instead of leaving an empty expand row", () => {
    // Hunk starts at new 4, so the gap above is 1..3; revealing top 2 and
    // bottom 1 covers it entirely.
    const hunk: Hunk = { ...standardHunk(), oldStart: 4, newStart: 4 };
    const state = loadedState(standardDiff({ hunks: [hunk] }));
    state.reveals = [
      { top: 2, bottom: 1 },
      { top: 0, bottom: 0 },
    ];
    const files = [file()];
    const rows = buildRows(files, statesFor(files, state), noComments, noReplies, null, false);
    const gapRows = rows.slice(1, 4);
    expect(kinds(gapRows)).toEqual(["line", "line", "line"]);
    expect(rows.filter((r) => r.kind === "expand" && r.gi === 0)).toEqual([]);
  });
});

describe("indexComments", () => {
  it("maps comments to their file's index, keyed by side:endLine for lines", () => {
    const files = [file(), file({ path: "src/b.ts", fingerprint: "fp-b" })];
    const index = indexComments(files, [
      comment({ id: 1, filePath: "src/b.ts", side: "new", endLine: 11 }),
      comment({ id: 2, level: "file", filePath: "src/b.ts", side: null, startLine: null, endLine: null }),
      comment({ id: 3, filePath: "src/a.ts", side: "old", startLine: 11, endLine: 11 }),
    ]);
    expect(index.get(1)?.line.get("new:11")?.map((c) => c.id)).toEqual([1]);
    expect(index.get(1)?.file.map((c) => c.id)).toEqual([2]);
    expect(index.get(0)?.line.get("old:11")?.map((c) => c.id)).toEqual([3]);
  });

  it("skips review-level comments, replies, and files outside the diff", () => {
    const files = [file()];
    const index = indexComments(files, [
      comment({ id: 1, level: "review", filePath: null }),
      comment({ id: 2, parentId: 1 }),
      comment({ id: 3, filePath: "gone/elsewhere.ts" }),
    ]);
    expect(index.size).toBe(0);
  });
});

describe("buildRows: comments, threads, and composers", () => {
  const files = [file()];
  const states = () => statesFor(files, loadedState(standardDiff()));

  it("attaches line comments directly below their host line, per side", () => {
    const newSide = comment({ id: 7, side: "new", endLine: 11 });
    const oldSide = comment({ id: 8, side: "old", startLine: 11, endLine: 11 });
    const rows = buildRows(
      files,
      states(),
      indexComments(files, [newSide, oldSide]),
      noReplies,
      null,
      false,
    );
    const flat = rows.map((r) =>
      r.kind === "line"
        ? `${r.line.kind}:${lineSide(r.line)}:${lineNumber(r.line)}`
        : r.kind === "comment"
          ? `comment:${r.comment.id}`
          : r.kind,
    );
    expect(flat).toEqual([
      "file",
      "expand",
      "hunk",
      "context:new:10",
      "deletion:old:11",
      "comment:8", // old-side comment under the deletion
      "addition:new:11",
      "comment:7", // new-side comment under the addition
      "addition:new:12",
      "expand",
    ]);
  });

  it("contributes no row for a comment hosted inside a collapsed gap, but counts it under hide-whitespace", () => {
    // New line 5 sits in the 1..9 gap: no hunk line hosts it.
    const gapComment = comment({ id: 9, side: "new", startLine: 5, endLine: 5 });
    const index = indexComments(files, [gapComment]);
    const plain = buildRows(files, states(), index, noReplies, null, false);
    expect(plain.filter((r) => r.kind === "comment")).toEqual([]);
    expect(plain.filter((r) => r.kind === "hiddenComments")).toEqual([]);
    // With hide-whitespace on the same absence gets an indicator row instead
    // of silently vanishing.
    const flagged = buildRows(files, states(), index, noReplies, null, true);
    expect(flagged.filter((r) => r.kind === "hiddenComments")).toEqual([
      { kind: "hiddenComments", fi: 0, count: 1 },
    ]);
  });

  it("renders open threads whole and collapses closed ones to their root", () => {
    const root = comment({ id: 10, side: "new", endLine: 11 });
    const reply = comment({ id: 11, parentId: 10, filePath: null, side: null, startLine: null, endLine: null });
    const replies = new Map([[10, [reply]]]);
    const open = buildRows(
      files,
      states(),
      indexComments(files, [root]),
      replies,
      null,
      false,
    );
    expect(
      open.filter((r) => r.kind === "comment").map((r) => (r as Extract<Row, { kind: "comment" }>).comment.id),
    ).toEqual([10, 11]);

    const resolvedRoot = { ...root, state: "resolved" as const };
    const closed = buildRows(
      files,
      states(),
      indexComments(files, [resolvedRoot]),
      replies,
      null,
      false,
    );
    expect(
      closed.filter((r) => r.kind === "comment").map((r) => (r as Extract<Row, { kind: "comment" }>).comment.id),
    ).toEqual([10]);
  });

  it("places the single composer at its path-keyed location", () => {
    // File-level: after the file's comments, before any hunks.
    const fileRows = buildRows(files, states(), noComments, noReplies, { level: "file", path: "src/a.ts" }, false);
    expect(kinds(fileRows).slice(0, 3)).toEqual(["file", "composer", "expand"]);

    // Line-level: under the selected line.
    const lineRows = buildRows(
      files,
      states(),
      noComments,
      noReplies,
      { level: "line", path: "src/a.ts", side: "new", startLine: 11, endLine: 11 },
      false,
    );
    const lineIdx = lineRows.findIndex(
      (r) => r.kind === "line" && r.line.newLineno === 11,
    );
    expect(lineRows[lineIdx + 1]?.kind).toBe("composer");

    // Reply-level: after the thread's replies.
    const root = comment({ id: 12, side: "new", endLine: 11 });
    const reply = comment({ id: 13, parentId: 12, filePath: null });
    const replyRows = buildRows(
      files,
      states(),
      indexComments(files, [root]),
      new Map([[12, [reply]]]),
      { level: "reply", path: "src/a.ts", rootId: 12 },
      false,
    );
    const lastReplyIdx = replyRows.findIndex(
      (r) => r.kind === "comment" && r.comment.id === 13,
    );
    expect(replyRows[lastReplyIdx + 1]?.kind).toBe("composer");

    // A composer for another file's path places nothing here.
    const elsewhere = buildRows(files, states(), noComments, noReplies, { level: "file", path: "src/other.ts" }, false);
    expect(elsewhere.filter((r) => r.kind === "composer")).toEqual([]);
  });
});

describe("rowKey", () => {
  it("keys rows by path so identities survive files shifting position", () => {
    const a = file();
    const b = file({ path: "src/b.ts", fingerprint: "fp-b" });
    // Same file, different display index → same key.
    expect(rowKey({ kind: "file", fi: 0 }, [a])).toBe(rowKey({ kind: "file", fi: 1 }, [b, a]));
    expect(rowKey({ kind: "hunk", fi: 0, hi: 2, header: "@@" }, [a])).toBe("hsrc/a.ts:2");
    expect(
      rowKey({ kind: "line", fi: 0, line: line("addition", null, 11) }, [a]),
    ).toBe("lsrc/a.ts::11");
    // Comment rows key on the comment id, independent of file order.
    expect(rowKey({ kind: "comment", fi: 0, comment: comment({ id: 5 }) }, [a])).toBe("c5");
  });
});

describe("estimateRowHeight", () => {
  it("uses fixed heights per kind and the skeleton's own estimate", () => {
    expect(estimateRowHeight({ kind: "file", fi: 0 })).toBe(42);
    expect(estimateRowHeight({ kind: "line", fi: 0, line: line("context", 1, 1) })).toBe(21);
    expect(estimateRowHeight({ kind: "skeleton", fi: 0, height: 777 })).toBe(777);
  });
});

describe("reviewedFlips", () => {
  const files = [
    file({ path: "src/a.ts" }),
    file({ path: "src/b.ts", fingerprint: "fp-b" }),
    file({ path: "src/c.ts", fingerprint: "fp-c" }),
  ];
  const marks = (
    entries: [string, FileReviewState][],
  ): Map<string, FileReviewState> => new Map(entries);

  it("collapses newly reviewed files and expands unmarked ones", () => {
    expect(
      reviewedFlips(
        files,
        marks([["src/a.ts", "reviewed"]]),
        marks([
          ["src/b.ts", "reviewed"],
          ["src/c.ts", "reviewed"],
        ]),
      ),
    ).toEqual([
      { path: "src/a.ts", expanded: true },
      { path: "src/b.ts", expanded: false },
      { path: "src/c.ts", expanded: false },
    ]);
  });

  it("ignores files whose reviewed-ness did not change", () => {
    const same = marks([["src/a.ts", "reviewed"]]);
    expect(reviewedFlips(files, same, marks([...same]))).toEqual([]);
  });

  it("treats a changed-since-review transition as un-reviewing", () => {
    expect(
      reviewedFlips(
        files,
        marks([["src/a.ts", "reviewed"]]),
        marks([["src/a.ts", "changed"]]),
      ),
    ).toEqual([{ path: "src/a.ts", expanded: true }]);
    // Unmarked → changed never flips: the card was never collapsed.
    expect(
      reviewedFlips(files, marks([]), marks([["src/b.ts", "changed"]])),
    ).toEqual([]);
  });

  it("keeps display order so the first collapse is the scroll anchor", () => {
    const flips = reviewedFlips(
      files,
      marks([]),
      marks([
        ["src/c.ts", "reviewed"],
        ["src/b.ts", "reviewed"],
      ]),
    );
    expect(flips.map((f) => f.path)).toEqual(["src/b.ts", "src/c.ts"]);
  });
});
