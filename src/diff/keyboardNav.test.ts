import { describe, expect, it } from "vitest";
import type { Comment, FileReviewState, FileSummary } from "../types";
import {
  commentTargets,
  nextCommentTarget,
  nextUnviewedPath,
  type CommentTarget,
} from "./keyboardNav";
import type { Row } from "./rows";

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

function commentRow(over: Partial<Comment>): Row {
  return { kind: "comment", fi: 0, comment: comment(over) };
}

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

describe("commentTargets", () => {
  it("keeps only open thread roots, in row order", () => {
    const rows: Row[] = [
      { kind: "file", fi: 0 },
      commentRow({ id: 1 }),
      commentRow({ id: 2, parentId: 1 }),
      commentRow({ id: 3, state: "resolved" }),
      { kind: "line", fi: 0, line: { kind: "context", oldLineno: 1, newLineno: 1, content: "x" } },
      commentRow({ id: 4 }),
    ];
    expect(commentTargets(rows)).toEqual([
      { id: 1, rowIndex: 1 },
      { id: 4, rowIndex: 5 },
    ]);
  });

  it("is empty when no comment rows exist", () => {
    expect(commentTargets([{ kind: "file", fi: 0 }])).toEqual([]);
  });
});

describe("nextCommentTarget", () => {
  const targets: CommentTarget[] = [
    { id: 1, rowIndex: 2 },
    { id: 4, rowIndex: 9 },
    { id: 7, rowIndex: 20 },
  ];

  it("returns null when there are no targets", () => {
    expect(nextCommentTarget([], null, 1)).toBeNull();
  });

  it("enters at the first target going forward, the last going back", () => {
    expect(nextCommentTarget(targets, null, 1)).toEqual(targets[0]);
    expect(nextCommentTarget(targets, null, -1)).toEqual(targets[2]);
  });

  it("re-enters the same way when the remembered id vanished", () => {
    expect(nextCommentTarget(targets, 999, 1)).toEqual(targets[0]);
    expect(nextCommentTarget(targets, 999, -1)).toEqual(targets[2]);
  });

  it("steps and wraps in both directions", () => {
    expect(nextCommentTarget(targets, 1, 1)).toEqual(targets[1]);
    expect(nextCommentTarget(targets, 7, 1)).toEqual(targets[0]);
    expect(nextCommentTarget(targets, 4, -1)).toEqual(targets[0]);
    expect(nextCommentTarget(targets, 1, -1)).toEqual(targets[2]);
  });
});

describe("nextUnviewedPath", () => {
  const files = [
    file({ path: "a.ts" }),
    file({ path: "b.ts" }),
    file({ path: "c.ts" }),
    file({ path: "d.ts" }),
  ];
  const states = (entries: [string, FileReviewState][]) =>
    new Map<string, FileReviewState>(entries);

  it("returns null with no files or with every file reviewed", () => {
    expect(nextUnviewedPath([], new Map(), null, 1)).toBeNull();
    const all = states(files.map((f) => [f.path, "reviewed"]));
    expect(nextUnviewedPath(files, all, "a.ts", 1)).toBeNull();
  });

  it("skips reviewed files in both directions", () => {
    const s = states([
      ["b.ts", "reviewed"],
      ["c.ts", "reviewed"],
    ]);
    expect(nextUnviewedPath(files, s, "a.ts", 1)).toBe("d.ts");
    expect(nextUnviewedPath(files, s, "d.ts", -1)).toBe("a.ts");
  });

  it("treats changed-since-review as unviewed", () => {
    const s = states([
      ["b.ts", "changed"],
      ["c.ts", "reviewed"],
      ["d.ts", "reviewed"],
    ]);
    expect(nextUnviewedPath(files, s, "a.ts", 1)).toBe("b.ts");
  });

  it("wraps past the ends", () => {
    const s = states([
      ["b.ts", "reviewed"],
      ["c.ts", "reviewed"],
      ["d.ts", "reviewed"],
    ]);
    expect(nextUnviewedPath(files, s, "d.ts", 1)).toBe("a.ts");
    expect(nextUnviewedPath(files, s, "a.ts", -1)).toBe("a.ts");
  });

  it("enters from the first file going forward and the last going back", () => {
    const s = states([["a.ts", "reviewed"]]);
    expect(nextUnviewedPath(files, s, null, 1)).toBe("b.ts");
    expect(nextUnviewedPath(files, s, null, -1)).toBe("d.ts");
    // A vanished cursor path behaves like no cursor.
    expect(nextUnviewedPath(files, s, "gone.ts", 1)).toBe("b.ts");
  });

  it("lands back on a lone unviewed cursor file instead of null", () => {
    const s = states([
      ["a.ts", "reviewed"],
      ["c.ts", "reviewed"],
      ["d.ts", "reviewed"],
    ]);
    expect(nextUnviewedPath(files, s, "b.ts", 1)).toBe("b.ts");
  });
});
