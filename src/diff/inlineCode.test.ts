import { describe, expect, it } from "vitest";
import { splitInlineCode } from "./inlineCode";

const text = (t: string) => ({ code: false, text: t });
const code = (t: string) => ({ code: true, text: t });

describe("splitInlineCode", () => {
  it("passes plain text through as one segment", () => {
    expect(splitInlineCode("no backticks here")).toEqual([
      text("no backticks here"),
    ]);
  });

  it("returns no segments for empty input", () => {
    expect(splitInlineCode("")).toEqual([]);
  });

  it("splits a single pair into text/code/text", () => {
    expect(splitInlineCode("call `foo()` next")).toEqual([
      text("call "),
      code("foo()"),
      text(" next"),
    ]);
  });

  it("handles a code span at each end of the string", () => {
    expect(splitInlineCode("`start` and `end`")).toEqual([
      code("start"),
      text(" and "),
      code("end"),
    ]);
  });

  it("keeps an unpaired backtick literal", () => {
    expect(splitInlineCode("odd `tick stays put")).toEqual([
      text("odd `tick stays put"),
    ]);
  });

  it("pairs greedily and keeps a trailing odd backtick literal", () => {
    expect(splitInlineCode("`a` then `rest")).toEqual([
      code("a"),
      text(" then `rest"),
    ]);
  });

  it("handles adjacent code spans", () => {
    expect(splitInlineCode("`a``b`")).toEqual([code("a"), code("b")]);
  });

  it("keeps an empty pair literal", () => {
    expect(splitInlineCode("empty `` pair")).toEqual([text("empty `` pair")]);
  });

  it("keeps backtick-only strings literal", () => {
    expect(splitInlineCode("`")).toEqual([text("`")]);
    expect(splitInlineCode("``")).toEqual([text("``")]);
    expect(splitInlineCode("```")).toEqual([text("```")]);
  });

  it("coalesces adjacent literal runs into one text segment", () => {
    // "``x" = empty pair (literal) + plain "x" — one merged segment.
    expect(splitInlineCode("``x")).toEqual([text("``x")]);
  });
});
