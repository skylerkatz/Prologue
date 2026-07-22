import type { FileSummary, Guide } from "../types";

/** A guide section resolved against the displayed summary's files. */
export interface ResolvedGuideSection {
  title: string;
  summary: string | null;
  /** `01/05`-style position among the guide's sections; null for the
   * catch-all bucket of files the guide doesn't know. */
  ordinal: string | null;
  files: FileSummary[];
}

/**
 * Resolve a guide's sections against the displayed files. This is the single
 * ordering source of truth for every guide surface: the grouped sidebar
 * renders these sections and the diff pane renders their flattening
 * ([`guideOrderedFiles`]), so the two can never disagree.
 *
 * The guide was validated exact-once against ITS diff, but the summary may
 * have drifted since (staleness UX is a later phase), so: paths the diff no
 * longer has drop out, empty sections vanish, and files the guide doesn't
 * know land in a trailing unnumbered bucket — every displayed file appears
 * in exactly one section. Null when there is no guide.
 */
export function resolveGuideSections(
  guide: Guide | null,
  files: readonly FileSummary[],
): ResolvedGuideSection[] | null {
  if (guide === null) {
    return null;
  }
  const byPath = new Map(files.map((f) => [f.path, f]));
  const claimed = new Set<string>();
  const resolved: Omit<ResolvedGuideSection, "ordinal">[] = [];
  for (const section of guide.sections) {
    const sectionFiles: FileSummary[] = [];
    for (const path of section.files) {
      const file = byPath.get(path);
      if (file !== undefined && !claimed.has(path)) {
        claimed.add(path);
        sectionFiles.push(file);
      }
    }
    if (sectionFiles.length > 0) {
      resolved.push({
        title: section.title,
        summary: section.summary,
        files: sectionFiles,
      });
    }
  }
  const total = String(resolved.length).padStart(2, "0");
  const out: ResolvedGuideSection[] = resolved.map((section, i) => ({
    ...section,
    ordinal: `${String(i + 1).padStart(2, "0")}/${total}`,
  }));
  const leftover = files.filter((f) => !claimed.has(f.path));
  if (leftover.length > 0) {
    out.push({
      title: "Not in guide",
      summary: null,
      ordinal: null,
      files: leftover,
    });
  }
  return out;
}

/** The diff pane's file order while the guide view is active: section order,
 * flattened. Contains every displayed file exactly once, by construction of
 * [`resolveGuideSections`]. */
export function guideOrderedFiles(
  sections: readonly ResolvedGuideSection[],
): FileSummary[] {
  return sections.flatMap((section) => section.files);
}
