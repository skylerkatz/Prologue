import type { LanguageRegistration } from "shiki/core";

type GrammarModule = { default: LanguageRegistration[] };

/**
 * One loader per grammar id `detectLang` can return. Each import is a
 * separate lazy chunk — Vite code-splits them, so a grammar's bytes are
 * fetched only the first time a file of that language is highlighted.
 * (Embedded-grammar dependencies — e.g. Blade pulls in PHP and HTML — ship
 * inside the importing grammar's module.)
 */
export const GRAMMAR_LOADERS: Readonly<
  Record<string, () => Promise<GrammarModule>>
> = {
  blade: () => import("@shikijs/langs/blade"),
  php: () => import("@shikijs/langs/php"),
  javascript: () => import("@shikijs/langs/javascript"),
  typescript: () => import("@shikijs/langs/typescript"),
  jsx: () => import("@shikijs/langs/jsx"),
  tsx: () => import("@shikijs/langs/tsx"),
  json: () => import("@shikijs/langs/json"),
  jsonc: () => import("@shikijs/langs/jsonc"),
  html: () => import("@shikijs/langs/html"),
  css: () => import("@shikijs/langs/css"),
  scss: () => import("@shikijs/langs/scss"),
  less: () => import("@shikijs/langs/less"),
  vue: () => import("@shikijs/langs/vue"),
  markdown: () => import("@shikijs/langs/markdown"),
  yaml: () => import("@shikijs/langs/yaml"),
  toml: () => import("@shikijs/langs/toml"),
  sql: () => import("@shikijs/langs/sql"),
  shellscript: () => import("@shikijs/langs/shellscript"),
  rust: () => import("@shikijs/langs/rust"),
  python: () => import("@shikijs/langs/python"),
  go: () => import("@shikijs/langs/go"),
  ruby: () => import("@shikijs/langs/ruby"),
  java: () => import("@shikijs/langs/java"),
  kotlin: () => import("@shikijs/langs/kotlin"),
  swift: () => import("@shikijs/langs/swift"),
  xml: () => import("@shikijs/langs/xml"),
  ini: () => import("@shikijs/langs/ini"),
  dotenv: () => import("@shikijs/langs/dotenv"),
  graphql: () => import("@shikijs/langs/graphql"),
  twig: () => import("@shikijs/langs/twig"),
  diff: () => import("@shikijs/langs/diff"),
  dockerfile: () => import("@shikijs/langs/dockerfile"),
  make: () => import("@shikijs/langs/make"),
};
