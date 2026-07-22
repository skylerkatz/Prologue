import type { FileSummary } from "../types";

/**
 * Rank the changed files against a filter query for the ⌘P jump overlay.
 * Case-insensitive; a substring hit in the basename beats one anywhere in
 * the path, which beats a scattered subsequence match. Ties keep diff
 * order. An empty (or whitespace) query returns every file unranked.
 */
export function matchFiles(
  files: readonly FileSummary[],
  query: string,
): FileSummary[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") {
    return [...files];
  }
  const ranked: Array<{ file: FileSummary; tier: number; index: number }> = [];
  files.forEach((file, index) => {
    const tier = matchTier(file.path, needle);
    if (tier !== null) {
      ranked.push({ file, tier, index });
    }
  });
  ranked.sort((a, b) => a.tier - b.tier || a.index - b.index);
  return ranked.map((r) => r.file);
}

function matchTier(path: string, needle: string): number | null {
  const haystack = path.toLowerCase();
  const basename = haystack.slice(haystack.lastIndexOf("/") + 1);
  if (basename.includes(needle)) {
    return 0;
  }
  if (haystack.includes(needle)) {
    return 1;
  }
  if (isSubsequence(needle, haystack)) {
    return 2;
  }
  return null;
}

/** True when every needle character appears in order in the haystack. */
function isSubsequence(needle: string, haystack: string): boolean {
  let matched = 0;
  for (let i = 0; i < haystack.length && matched < needle.length; i++) {
    if (haystack[i] === needle[matched]) {
      matched++;
    }
  }
  return matched === needle.length;
}
