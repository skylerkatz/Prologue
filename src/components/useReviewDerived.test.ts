import { describe, expect, it } from "vitest";
import type { Comment } from "../types";
import { fileGoneRoots } from "./useReviewDerived";

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

describe("fileGoneRoots", () => {
  const present = new Set(["src/a.ts"]);

  it("keeps anchor-orphaned comments on present files out of the bucket", () => {
    // Anchor status is not an input at all: presence of the file is the only
    // criterion, so a re-anchor-orphaned comment on src/a.ts stays in
    // diffComments and renders inside that file's section.
    const inFile = comment({ id: 1, filePath: "src/a.ts" });
    expect(fileGoneRoots([inFile], present)).toEqual([]);
  });

  it("buckets roots whose file left the diff, at any level", () => {
    const lineGone = comment({ id: 1, filePath: "gone/old.ts" });
    const fileGone = comment({
      id: 2,
      level: "file",
      filePath: "gone/old.ts",
      side: null,
      startLine: null,
      endLine: null,
    });
    expect(
      fileGoneRoots([lineGone, fileGone], present).map((c) => c.id),
    ).toEqual([1, 2]);
  });

  it("never buckets review-level roots or replies", () => {
    const review = comment({ id: 1, level: "review", filePath: null });
    const reply = comment({
      id: 2,
      parentId: 3,
      filePath: null,
      side: null,
      startLine: null,
      endLine: null,
    });
    expect(fileGoneRoots([review, reply], present)).toEqual([]);
  });
});
