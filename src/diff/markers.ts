import type { FileDiff } from "../types";

/** Inclusive run of new-side line numbers added by the diff. */
export interface LineRange {
  start: number;
  end: number;
}

/** One deleted source line, for the click-to-peek reveal. */
export interface DeletedLine {
  oldLineno: number | null;
  content: string;
}

/**
 * A run of deleted lines, anchored to the new side: the deletion happened
 * just after new-side line `afterLine` (0 = before the first line).
 */
export interface DeletionAnchor {
  afterLine: number;
  lines: DeletedLine[];
}

/** Change geometry of one file, projected onto new-side line numbers. */
export interface FileMarkers {
  added: LineRange[];
  deletions: DeletionAnchor[];
}

/**
 * Project a file's hunks onto the new side: contiguous addition runs (a
 * deletion between additions doesn't split a run — the new-side numbers
 * stay contiguous) and deletion runs anchored after the last line that
 * exists on the new side. Both come out sorted by position.
 */
export function computeMarkers(diff: FileDiff): FileMarkers {
  const added: LineRange[] = [];
  const deletions: DeletionAnchor[] = [];
  for (const hunk of diff.hunks) {
    // For a hunk with no new-side lines, git's header already names the
    // line before the deletion (0 when the whole file start was removed).
    let lastNew = hunk.newLines === 0 ? hunk.newStart : hunk.newStart - 1;
    let run: LineRange | null = null;
    let del: DeletionAnchor | null = null;
    for (const line of hunk.lines) {
      if (line.kind === "deletion") {
        if (del === null) {
          del = { afterLine: lastNew, lines: [] };
          deletions.push(del);
        }
        del.lines.push({ oldLineno: line.oldLineno, content: line.content });
        continue;
      }
      del = null;
      const n = line.newLineno;
      if (n === null) {
        continue;
      }
      lastNew = n;
      if (line.kind === "addition") {
        if (run !== null && run.end === n - 1) {
          run.end = n;
        } else {
          run = { start: n, end: n };
          added.push(run);
        }
      } else {
        run = null;
      }
    }
  }
  return { added, deletions };
}

export type BlockMark = "added" | "modified";

/**
 * Classify a rendered block spanning new-side lines [start, end]: fully
 * inside one added run → "added"; touching an added run, or with a deletion
 * anchored strictly inside it → "modified"; otherwise untouched. Plain
 * interval intersection — the runs are what the hunks already said.
 */
export function classifyBlock(
  markers: FileMarkers,
  start: number,
  end: number,
): BlockMark | null {
  let intersects = false;
  for (const range of markers.added) {
    if (range.start <= start && range.end >= end) {
      return "added";
    }
    if (range.start <= end && range.end >= start) {
      intersects = true;
    }
  }
  if (intersects) {
    return "modified";
  }
  for (const anchor of markers.deletions) {
    if (anchor.afterLine >= start && anchor.afterLine < end) {
      return "modified";
    }
  }
  return null;
}

/**
 * The new-side line a comment on this block should target: the first added
 * line inside it, else the line a deletion inside it hangs off (clamped to
 * 1 for a deletion before the first line). Null for untouched blocks.
 */
export function blockTargetLine(
  markers: FileMarkers,
  start: number,
  end: number,
): number | null {
  let best: number | null = null;
  for (const range of markers.added) {
    if (range.start <= end && range.end >= start) {
      const line = Math.max(range.start, start);
      if (best === null || line < best) {
        best = line;
      }
    }
  }
  if (best !== null) {
    return best;
  }
  for (const anchor of markers.deletions) {
    if (anchor.afterLine >= start && anchor.afterLine < end) {
      const line = Math.max(anchor.afterLine, 1);
      if (best === null || line < best) {
        best = line;
      }
    }
  }
  return best;
}

/** The file's first changed new-side line; the `c`-key auto-flip target. */
export function firstChangedLine(markers: FileMarkers): number | null {
  let best: number | null = null;
  for (const range of markers.added) {
    if (best === null || range.start < best) {
      best = range.start;
    }
  }
  for (const anchor of markers.deletions) {
    const line = Math.max(anchor.afterLine, 1);
    if (best === null || line < best) {
      best = line;
    }
  }
  return best;
}

// Keyed on the FileDiff object itself: a refresh that changes the file
// replaces the diff, so entries expire with the objects they describe, and
// the stable value keeps MarkdownPreview's memo effective.
const cache = new WeakMap<FileDiff, FileMarkers>();

export function markersFor(diff: FileDiff): FileMarkers {
  let markers = cache.get(diff);
  if (markers === undefined) {
    markers = computeMarkers(diff);
    cache.set(diff, markers);
  }
  return markers;
}
