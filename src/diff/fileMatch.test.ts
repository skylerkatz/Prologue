import { describe, expect, it } from "vitest";
import type { FileSummary } from "../types";
import { matchFiles } from "./fileMatch";

function file(path: string, over: Partial<FileSummary> = {}): FileSummary {
  return {
    path,
    oldPath: null,
    status: "modified",
    additions: 10,
    deletions: 2,
    binary: false,
    fingerprint: "fp",
    ...over,
  };
}

const paths = (files: FileSummary[]) => files.map((f) => f.path);

describe("matchFiles", () => {
  it("returns every file in diff order for an empty or whitespace query", () => {
    const files = [file("b.ts"), file("a.ts")];
    expect(paths(matchFiles(files, ""))).toEqual(["b.ts", "a.ts"]);
    expect(paths(matchFiles(files, "   "))).toEqual(["b.ts", "a.ts"]);
  });

  it("matches substrings case-insensitively", () => {
    const files = [file("src/components/FileList.tsx"), file("src/main.rs")];
    expect(paths(matchFiles(files, "filelist"))).toEqual([
      "src/components/FileList.tsx",
    ]);
    expect(paths(matchFiles(files, "MAIN"))).toEqual(["src/main.rs"]);
  });

  it("ranks basename hits above directory-only hits", () => {
    const files = [
      file("review/archive.rs"),
      file("src/review.rs"),
      file("review/mod.rs"),
    ];
    // "review" is in every path, but only src/review.rs has it in the basename.
    expect(paths(matchFiles(files, "review"))).toEqual([
      "src/review.rs",
      "review/archive.rs",
      "review/mod.rs",
    ]);
  });

  it("falls back to in-order subsequence matching", () => {
    const files = [file("src/components/BranchSelect.tsx"), file("src/db.rs")];
    expect(paths(matchFiles(files, "bsel"))).toEqual([
      "src/components/BranchSelect.tsx",
    ]);
    // Out of order — not a subsequence.
    expect(matchFiles(files, "selb")).toEqual([]);
  });

  it("ranks substring matches above subsequence matches", () => {
    const files = [
      file("src/apps.ts"), // "aps" only as a subsequence
      file("docs/aps.md"), // "aps" as the basename substring
    ];
    expect(paths(matchFiles(files, "aps"))).toEqual([
      "docs/aps.md",
      "src/apps.ts",
    ]);
  });

  it("drops files that do not match at all", () => {
    const files = [file("src/a.ts"), file("src/b.ts")];
    expect(matchFiles(files, "zzz")).toEqual([]);
  });

  it("keeps diff order within a tier", () => {
    const files = [file("src/z-thing.ts"), file("src/a-thing.ts")];
    expect(paths(matchFiles(files, "thing"))).toEqual([
      "src/z-thing.ts",
      "src/a-thing.ts",
    ]);
  });
});
