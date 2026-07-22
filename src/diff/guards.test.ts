import { describe, expect, it } from "vitest";
import type { FileSummary } from "../types";
import { guardReason } from "./guards";

function file(over: Partial<FileSummary> = {}): FileSummary {
  return {
    path: "src/app.ts",
    oldPath: null,
    status: "modified",
    additions: 10,
    deletions: 2,
    binary: false,
    fingerprint: "fp",
    ...over,
  };
}

describe("guardReason", () => {
  it("passes ordinary files through", () => {
    expect(guardReason(file())).toBeNull();
  });

  it("flags binary files first, before any size check", () => {
    expect(guardReason(file({ binary: true, additions: 9999999 }))).toBe("binary");
  });

  it("collapses files over the auto-load line budget, inclusive boundary excluded", () => {
    expect(guardReason(file({ additions: 5000, deletions: 0 }))).toBeNull();
    expect(guardReason(file({ additions: 5000, deletions: 1 }))).toBe("oversize");
  });

  it("recognizes lockfiles by basename, case-insensitively and in subdirectories", () => {
    expect(guardReason(file({ path: "package-lock.json" }))).toBe("generated");
    expect(guardReason(file({ path: "frontend/deep/yarn.lock" }))).toBe("generated");
    expect(guardReason(file({ path: "src-tauri/Cargo.lock" }))).toBe("generated");
    // Not a lockfile basename — only an unlucky directory name.
    expect(guardReason(file({ path: "yarn.lock/readme.md" }))).toBeNull();
  });

  it("recognizes generated suffixes", () => {
    expect(guardReason(file({ path: "dist/app.min.js" }))).toBe("generated");
    expect(guardReason(file({ path: "styles/site.min.css" }))).toBe("generated");
    expect(guardReason(file({ path: "dist/bundle.js.map" }))).toBe("generated");
    expect(guardReason(file({ path: "tests/__snapshots__/ui.snap" }))).toBe("generated");
    expect(guardReason(file({ path: "src/minify.ts" }))).toBeNull();
  });
});
