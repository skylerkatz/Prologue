import type { FileSummary, Guide } from "../types";

/**
 * Whether the displayed diff has drifted from the one the guide was
 * generated over: a changed fingerprint, a file the guide never saw, or a
 * file it knew that is gone (a rename reads as one of each). Fingerprints
 * are content identities (blob OIDs + mode), so review-side presentation
 * toggles like hide-whitespace can never flip this.
 */
export function guideIsStale(
  fingerprints: Guide["fingerprints"],
  files: readonly FileSummary[],
): boolean {
  if (Object.keys(fingerprints).length !== files.length) {
    return true;
  }
  return files.some((f) => fingerprints[f.path] !== f.fingerprint);
}
