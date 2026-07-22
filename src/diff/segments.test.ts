import { describe, expect, it } from "vitest";
import type { DiffLine } from "../types";
import { segmentLine } from "./segments";

function line(content: string, intraline?: { start: number; end: number }[]): DiffLine {
  return { kind: "addition", oldLineno: null, newLineno: 1, content, intraline };
}

describe("segmentLine", () => {
  it("returns the whole line as one unchanged segment without tokens or ranges", () => {
    expect(segmentLine(line("plain text"), undefined)).toEqual([
      { text: "plain text", changed: false, style: undefined },
    ]);
  });

  it("maps tokens straight through when there are no intraline ranges", () => {
    const tokens = [
      { content: "const ", htmlStyle: { color: "#c00" } },
      { content: "x", htmlStyle: { color: "#0c0" } },
    ];
    expect(segmentLine(line("const x"), tokens)).toEqual([
      { text: "const ", changed: false, style: { color: "#c00" } },
      { text: "x", changed: false, style: { color: "#0c0" } },
    ]);
  });

  it("splits an untokenized line at range boundaries", () => {
    expect(segmentLine(line("hello world", [{ start: 6, end: 11 }]), undefined)).toEqual([
      { text: "hello ", changed: false, style: undefined },
      { text: "world", changed: true, style: undefined },
    ]);
  });

  it("splits tokens at range boundaries so both layers compose", () => {
    const tokens = [
      { content: "const ", htmlStyle: { color: "#c00" } },
      { content: "foo", htmlStyle: { color: "#0c0" } },
    ];
    // Range 3..8 covers the tail of the first token and most of the second.
    expect(segmentLine(line("const foo", [{ start: 3, end: 8 }]), tokens)).toEqual([
      { text: "con", changed: false, style: { color: "#c00" } },
      { text: "st ", changed: true, style: { color: "#c00" } },
      { text: "fo", changed: true, style: { color: "#0c0" } },
      { text: "o", changed: false, style: { color: "#0c0" } },
    ]);
  });

  it("handles multiple disjoint ranges within one token", () => {
    expect(
      segmentLine(line("abcdefgh", [
        { start: 1, end: 3 },
        { start: 5, end: 6 },
      ]), undefined),
    ).toEqual([
      { text: "a", changed: false, style: undefined },
      { text: "bc", changed: true, style: undefined },
      { text: "de", changed: false, style: undefined },
      { text: "f", changed: true, style: undefined },
      { text: "gh", changed: false, style: undefined },
    ]);
  });

  it("marks a fully-changed line as one changed segment", () => {
    expect(segmentLine(line("new!", [{ start: 0, end: 4 }]), undefined)).toEqual([
      { text: "new!", changed: true, style: undefined },
    ]);
  });
});
