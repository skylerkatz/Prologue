import type { HighlighterCore, ThemedToken } from "shiki/core";
import { GRAMMAR_LOADERS } from "./grammars";

/**
 * Lazy Shiki wrapper. Everything — the engine, its wasm, themes, and each
 * grammar — loads through dynamic imports on first use, so none of it is in
 * the initial bundle and first paint never waits on it. Callers render plain
 * text until `tokenizeLines` resolves.
 */

/** Hunks beyond this many lines render plain; tokenizing them isn't worth
 * the main-thread time (files that big are behind the oversize guard). */
export const MAX_HIGHLIGHT_LINES = 2000;

/** Lines tokenized per synchronous chunk before yielding to the main
 * thread, so one huge hunk can't hold a frame hostage. */
const TOKENIZE_SLICE = 200;

export type LineTokens = ThemedToken[];

let highlighterPromise: Promise<HighlighterCore> | null = null;
const langPromises = new Map<string, Promise<void>>();

function getHighlighter(): Promise<HighlighterCore> {
  highlighterPromise ??= (async () => {
    const [core, engine, light, dark] = await Promise.all([
      import("shiki/core"),
      import("shiki/engine/oniguruma"),
      import("@shikijs/themes/github-light"),
      import("@shikijs/themes/github-dark"),
    ]);
    return core.createHighlighterCore({
      themes: [light.default, dark.default],
      langs: [],
      engine: engine.createOnigurumaEngine(import("shiki/wasm")),
    });
  })();
  return highlighterPromise;
}

function ensureLang(highlighter: HighlighterCore, lang: string): Promise<void> {
  let promise = langPromises.get(lang);
  if (promise === undefined) {
    promise = GRAMMAR_LOADERS[lang]().then((grammar) =>
      highlighter.loadLanguage(...grammar.default),
    );
    // A failed grammar load is retryable on the next request.
    promise.catch(() => langPromises.delete(lang));
    langPromises.set(lang, promise);
  }
  return promise;
}

/**
 * Tokenize `lines` as one block (so multi-line constructs keep grammar
 * state across lines) and return per-line tokens, parallel to the input.
 * Null means "render plain": unknown grammar or an oversized block.
 *
 * Tokens carry `light-dark()` colors for both app color schemes.
 */
export async function tokenizeLines(
  lines: string[],
  lang: string,
): Promise<LineTokens[] | null> {
  if (!(lang in GRAMMAR_LOADERS) || lines.length > MAX_HIGHLIGHT_LINES) {
    return null;
  }
  const highlighter = await getHighlighter();
  await ensureLang(highlighter, lang);

  const out: LineTokens[] = [];
  for (let start = 0; start < lines.length; start += TOKENIZE_SLICE) {
    if (start > 0) {
      // Grammar state resets at slice boundaries — a small correctness
      // trade for never blocking the main thread on one long tokenize.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    const slice = lines.slice(start, start + TOKENIZE_SLICE);
    const result = highlighter.codeToTokens(slice.join("\n"), {
      lang,
      themes: { light: "github-light", dark: "github-dark" },
      defaultColor: "light-dark()",
    });
    for (const line of result.tokens) {
      out.push(line);
    }
  }
  // Joining and re-splitting must round-trip the line count; guard against
  // a tokenizer quirk ever desyncing token rows from diff rows.
  while (out.length < lines.length) {
    out.push([]);
  }
  out.length = lines.length;
  return out;
}
