import { describe, expect, it } from "vitest";
import { groupReplies, type Comment } from "./types";

function comment(over: Partial<Comment> = {}): Comment {
  return {
    id: 1,
    reviewId: 1,
    level: "line",
    filePath: "src/a.ts",
    side: "new",
    startLine: 1,
    endLine: 1,
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

describe("groupReplies", () => {
  it("groups replies under their root, preserving input (id) order", () => {
    const grouped = groupReplies([
      comment({ id: 1 }),
      comment({ id: 2, parentId: 1 }),
      comment({ id: 3 }),
      comment({ id: 4, parentId: 3 }),
      comment({ id: 5, parentId: 1 }),
    ]);
    expect(grouped.get(1)?.map((c) => c.id)).toEqual([2, 5]);
    expect(grouped.get(3)?.map((c) => c.id)).toEqual([4]);
  });

  it("never keys roots, even when they have no replies", () => {
    const grouped = groupReplies([comment({ id: 1 }), comment({ id: 2 })]);
    expect(grouped.size).toBe(0);
  });
});
