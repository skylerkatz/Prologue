import type { FileSummary } from "../types";

/** Files over this many changed lines collapse behind a "Load diff" click. */
export const MAX_AUTO_LINES = 5000;

export type GuardReason = "binary" | "oversize" | "generated";

/** Lockfiles and other generated files nobody reviews line by line. */
const GENERATED_NAMES = new Set([
  "package-lock.json",
  "npm-shrinkwrap.json",
  "yarn.lock",
  "pnpm-lock.yaml",
  "bun.lock",
  "bun.lockb",
  "deno.lock",
  "cargo.lock",
  "composer.lock",
  "gemfile.lock",
  "poetry.lock",
  "uv.lock",
  "pipfile.lock",
  "go.sum",
  "flake.lock",
  "packages.lock.json",
  "podfile.lock",
]);

const GENERATED_SUFFIXES = [".min.js", ".min.css", ".map", ".snap"];

export function isGeneratedPath(path: string): boolean {
  const name = (path.split("/").pop() ?? path).toLowerCase();
  return (
    GENERATED_NAMES.has(name) ||
    GENERATED_SUFFIXES.some((suffix) => name.endsWith(suffix))
  );
}

/** Why a file's diff is collapsed by default, or null to render eagerly. */
export function guardReason(file: FileSummary): GuardReason | null {
  if (file.binary) {
    return "binary";
  }
  if (file.additions + file.deletions > MAX_AUTO_LINES) {
    return "oversize";
  }
  if (isGeneratedPath(file.path)) {
    return "generated";
  }
  return null;
}
