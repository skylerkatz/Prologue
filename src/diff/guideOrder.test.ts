import { describe, expect, it } from "vitest";
import type { FileSummary, Guide, GuideSection } from "../types";
import { guideOrderedFiles, resolveGuideSections } from "./guideOrder";

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

function section(title: string, files: string[]): GuideSection {
  return { title, summary: `${title} summary`, files };
}

function guide(sections: GuideSection[]): Guide {
  return {
    id: 1,
    reviewId: 1,
    baseRef: "main",
    headRef: "feature",
    mode: "committed",
    fingerprints: {},
    model: "sonnet",
    costUsd: null,
    createdAt: "2026-07-22T00:00:00Z",
    sections,
  };
}

const paths = (files: FileSummary[]) => files.map((f) => f.path);

describe("resolveGuideSections", () => {
  it("is null without a guide", () => {
    expect(resolveGuideSections(null, [file("a.ts")])).toBeNull();
  });

  it("keeps section order and assigns ordinals", () => {
    const sections = resolveGuideSections(
      guide([section("Core", ["z.ts", "m.ts"]), section("Tests", ["a.ts"])]),
      [file("a.ts"), file("m.ts"), file("z.ts")],
    );
    expect(sections).not.toBeNull();
    expect(sections?.map((s) => s.title)).toEqual(["Core", "Tests"]);
    expect(sections?.map((s) => s.ordinal)).toEqual(["01/02", "02/02"]);
    expect(sections?.map((s) => paths(s.files))).toEqual([
      ["z.ts", "m.ts"],
      ["a.ts"],
    ]);
    expect(sections?.map((s) => s.summary)).toEqual([
      "Core summary",
      "Tests summary",
    ]);
  });

  it("drops paths the diff no longer has and vanishes emptied sections", () => {
    const sections = resolveGuideSections(
      guide([section("Gone", ["deleted.ts"]), section("Core", ["a.ts"])]),
      [file("a.ts")],
    );
    expect(sections?.map((s) => s.title)).toEqual(["Core"]);
    // Ordinals count surviving sections only.
    expect(sections?.map((s) => s.ordinal)).toEqual(["01/01"]);
  });

  it("claims a path duplicated across sections for the first one only", () => {
    const sections = resolveGuideSections(
      guide([section("First", ["a.ts"]), section("Second", ["a.ts", "b.ts"])]),
      [file("a.ts"), file("b.ts")],
    );
    expect(sections?.map((s) => paths(s.files))).toEqual([["a.ts"], ["b.ts"]]);
  });

  it("buckets files the guide doesn't know last, unnumbered", () => {
    const sections = resolveGuideSections(
      guide([section("Core", ["b.ts"])]),
      [file("a.ts"), file("b.ts"), file("c.ts")],
    );
    expect(sections?.map((s) => s.title)).toEqual(["Core", "Not in guide"]);
    const bucket = sections?.[1];
    expect(bucket?.ordinal).toBeNull();
    expect(bucket?.summary).toBeNull();
    // Leftovers keep the summary's own (A–Z) order.
    expect(paths(bucket?.files ?? [])).toEqual(["a.ts", "c.ts"]);
  });

  it("covers every displayed file exactly once, whatever the guide claims", () => {
    const files = [file("a.ts"), file("b.ts"), file("c.ts"), file("d.ts")];
    const sections = resolveGuideSections(
      guide([
        section("One", ["c.ts", "ghost.ts", "a.ts"]),
        section("Dupes", ["a.ts", "c.ts"]),
      ]),
      files,
    );
    const flattened = paths(guideOrderedFiles(sections ?? []));
    expect(flattened).toEqual(["c.ts", "a.ts", "b.ts", "d.ts"]);
    expect(new Set(flattened).size).toEqual(files.length);
  });
});

describe("guideOrderedFiles", () => {
  it("flattens sections into the diff pane's file order", () => {
    const sections = resolveGuideSections(
      guide([section("Core", ["z.ts"]), section("Tests", ["a.ts", "m.ts"])]),
      [file("a.ts"), file("m.ts"), file("z.ts")],
    );
    expect(paths(guideOrderedFiles(sections ?? []))).toEqual([
      "z.ts",
      "a.ts",
      "m.ts",
    ]);
  });

  it("is empty for no sections", () => {
    expect(guideOrderedFiles([])).toEqual([]);
  });
});
