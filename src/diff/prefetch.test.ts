import { describe, expect, it } from "vitest";
import { prefetchedFileDiff, primeFileDiffs, type DiffParams } from "./prefetch";
import type { FileDiff, FileSummary } from "../types";

const params: DiffParams = {
  repoPath: "/repo",
  base: "main",
  head: "feature",
  mode: "committed",
  ignoreWhitespace: false,
};

const file = (path: string, fingerprint: string): FileSummary => ({
  path,
  oldPath: null,
  status: "modified",
  additions: 1,
  deletions: 1,
  binary: false,
  fingerprint,
});

const diff = (path: string): FileDiff => ({
  path,
  oldPath: null,
  status: "modified",
  binary: false,
  hunks: [],
  newTotalLines: 3,
});

describe("prefetched file diffs", () => {
  it("stand in only for the exact fetch they replace", () => {
    primeFileDiffs(params, [file("a.ts", "f1"), file("b.ts", "f2")], [diff("a.ts")]);
    expect(prefetchedFileDiff(params, file("a.ts", "f1"))?.path).toBe("a.ts");
    // Not sent, changed since, or computed for another view: fetch it.
    expect(prefetchedFileDiff(params, file("b.ts", "f2"))).toBeNull();
    expect(prefetchedFileDiff(params, file("a.ts", "f9"))).toBeNull();
    expect(
      prefetchedFileDiff({ ...params, ignoreWhitespace: true }, file("a.ts", "f1")),
    ).toBeNull();
    expect(prefetchedFileDiff({ ...params, base: "dev" }, file("a.ts", "f1"))).toBeNull();
  });

  it("are replaced by the next load", () => {
    primeFileDiffs(params, [file("a.ts", "f1")], [diff("a.ts")]);
    primeFileDiffs(params, [file("b.ts", "f2")], [diff("b.ts")]);
    expect(prefetchedFileDiff(params, file("a.ts", "f1"))).toBeNull();
    expect(prefetchedFileDiff(params, file("b.ts", "f2"))).not.toBeNull();
  });
});
