import type { DiffMode, FileDiff, FileSummary } from "../types";

/** The coordinates a file's hunks were computed for. */
export interface DiffParams {
  repoPath: string;
  base: string;
  head: string;
  mode: DiffMode;
  ignoreWhitespace: boolean;
}

/**
 * Hunks the web server sent along with the review (`load_review`, or the
 * state inlined in index.html), so files that open expanded render without
 * a `get_file_diff` round trip each. Keyed by the diff's coordinates and
 * the file's content fingerprint, so an entry can only ever stand in for
 * the exact fetch it replaces. Reads don't consume entries (StrictMode
 * runs state initializers twice); each new load replaces the whole set.
 */
let prefetched = new Map<string, FileDiff>();

const key = (p: DiffParams, file: FileSummary): string =>
  [p.repoPath, p.base, p.head, p.mode, p.ignoreWhitespace, file.path, file.fingerprint].join(
    "\u0000",
  );

export function primeFileDiffs(
  params: DiffParams,
  files: readonly FileSummary[],
  diffs: readonly FileDiff[],
): void {
  const byPath = new Map(files.map((f) => [f.path, f]));
  prefetched = new Map();
  for (const diff of diffs) {
    const file = byPath.get(diff.path);
    if (file !== undefined) {
      prefetched.set(key(params, file), diff);
    }
  }
}

export function prefetchedFileDiff(
  params: DiffParams,
  file: FileSummary,
): FileDiff | null {
  return prefetched.get(key(params, file)) ?? null;
}
