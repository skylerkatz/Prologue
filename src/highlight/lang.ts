/**
 * File path → Shiki grammar id. Every entry here must have a matching loader
 * in `grammars.ts`; `detectLang` returns null for anything unmapped, which
 * renders plain (never a guess at a wrong grammar).
 */

/**
 * Compound extensions checked against the full filename before the plain
 * extension map — `.blade.php` must resolve to Blade, not PHP.
 */
const COMPOUND_EXTENSIONS: ReadonlyArray<readonly [string, string]> = [
  [".blade.php", "blade"],
  [".d.ts", "typescript"],
  [".d.mts", "typescript"],
  [".d.cts", "typescript"],
];

const EXTENSIONS: Readonly<Record<string, string>> = {
  php: "php",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  jsx: "jsx",
  tsx: "tsx",
  json: "json",
  jsonc: "jsonc",
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  less: "less",
  vue: "vue",
  md: "markdown",
  markdown: "markdown",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  sql: "sql",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  rs: "rust",
  py: "python",
  go: "go",
  rb: "ruby",
  java: "java",
  kt: "kotlin",
  swift: "swift",
  xml: "xml",
  svg: "xml",
  ini: "ini",
  env: "dotenv",
  graphql: "graphql",
  gql: "graphql",
  twig: "twig",
  diff: "diff",
  patch: "diff",
};

/** Extension-less filenames with a known grammar. */
const FILENAMES: Readonly<Record<string, string>> = {
  dockerfile: "dockerfile",
  makefile: "make",
  ".env": "dotenv",
};

/** The Shiki grammar id for a repo path, or null to stay plain. */
export function detectLang(path: string): string | null {
  const name = (path.split("/").pop() ?? path).toLowerCase();
  for (const [suffix, lang] of COMPOUND_EXTENSIONS) {
    if (name.endsWith(suffix)) {
      return lang;
    }
  }
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    const byExtension = EXTENSIONS[name.slice(dot + 1)];
    if (byExtension !== undefined) {
      return byExtension;
    }
  }
  // `.env.local`, `.env.production`, … are still dotenv files.
  if (name === ".env" || name.startsWith(".env.")) {
    return "dotenv";
  }
  return FILENAMES[name] ?? null;
}
