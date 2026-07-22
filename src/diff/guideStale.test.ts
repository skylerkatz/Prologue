import { describe, expect, it } from "vitest";
import type { FileSummary } from "../types";
import { guideIsStale } from "./guideStale";

function file(path: string, over: Partial<FileSummary> = {}): FileSummary {
  return {
    path,
    oldPath: null,
    status: "modified",
    additions: 10,
    deletions: 2,
    binary: false,
    fingerprint: `fp-${path}`,
    ...over,
  };
}

describe("guideIsStale", () => {
  it("is fresh when every fingerprint matches", () => {
    expect(
      guideIsStale({ "a.ts": "fp-a.ts", "b.ts": "fp-b.ts" }, [
        file("a.ts"),
        file("b.ts"),
      ]),
    ).toBe(false);
  });

  it("is fresh on an empty diff with an empty guide", () => {
    expect(guideIsStale({}, [])).toBe(false);
  });

  it("is stale when a file's fingerprint changed", () => {
    expect(
      guideIsStale({ "a.ts": "fp-a.ts", "b.ts": "old" }, [
        file("a.ts"),
        file("b.ts"),
      ]),
    ).toBe(true);
  });

  it("is stale when the diff gained a file the guide never saw", () => {
    expect(
      guideIsStale({ "a.ts": "fp-a.ts" }, [file("a.ts"), file("new.ts")]),
    ).toBe(true);
  });

  it("is stale when a file the guide knew left the diff", () => {
    expect(
      guideIsStale({ "a.ts": "fp-a.ts", "gone.ts": "fp-gone.ts" }, [
        file("a.ts"),
      ]),
    ).toBe(true);
  });

  it("is stale when a file was renamed (one add + one remove)", () => {
    expect(
      guideIsStale({ "old.ts": "fp-x" }, [
        file("new.ts", { oldPath: "old.ts", fingerprint: "fp-x" }),
      ]),
    ).toBe(true);
  });

  it("ignores presentation-only summary drift (whitespace toggle)", () => {
    // Toggling hide-whitespace changes +/− counts but not fingerprints
    // (content identities); the guide must not read as stale.
    expect(
      guideIsStale({ "a.ts": "fp-a.ts" }, [
        file("a.ts", { additions: 0, deletions: 0 }),
      ]),
    ).toBe(false);
  });
});
