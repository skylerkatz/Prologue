import type { FileReviewState, FileSummary } from "../types";
import type { Row } from "./rows";

/** An open thread root the n/p motions can land on: its comment id (the
 * position memory) and its current row index (the scroll target). */
export interface CommentTarget {
  id: number;
  rowIndex: number;
}

/**
 * The open thread roots in display order — the cycle n/p walk. Only rows the
 * current row model contains qualify, so collapsed files' threads (and, with
 * Hide Resolved on, resolved ones — already filtered upstream) are skipped,
 * matching what is actually visible. Replies never appear: they follow their
 * root, and closed roots render as a single collapsed card.
 */
export function commentTargets(rows: readonly Row[]): CommentTarget[] {
  const targets: CommentTarget[] = [];
  rows.forEach((row, rowIndex) => {
    if (
      row.kind === "comment" &&
      row.comment.parentId === null &&
      row.comment.state === "open"
    ) {
      targets.push({ id: row.comment.id, rowIndex });
    }
  });
  return targets;
}

/**
 * The target one n/p step from `lastId`, wrapping at both ends. From nowhere
 * (or a target that vanished — resolved, hidden, its file collapsed), n
 * lands on the first target and p on the last, mirroring how j/k enter the
 * file list. Null when no open threads are visible.
 */
export function nextCommentTarget(
  targets: readonly CommentTarget[],
  lastId: number | null,
  delta: 1 | -1,
): CommentTarget | null {
  if (targets.length === 0) {
    return null;
  }
  const prev =
    lastId === null ? -1 : targets.findIndex((t) => t.id === lastId);
  if (prev === -1) {
    return targets[delta > 0 ? 0 : targets.length - 1];
  }
  return targets[(prev + delta + targets.length) % targets.length];
}

/**
 * The next file J/K should land on: the first file not marked reviewed
 * ("changed since review" counts — it needs re-reviewing), scanning
 * `delta`-ward from `cursorPath` with wraparound. From nowhere, J scans from
 * the first file and K from the last. The scan ends back on the cursor
 * itself, so a lone unviewed file under the cursor is a no-op rather than
 * null. Null when every file is viewed.
 */
export function nextUnviewedPath(
  files: readonly FileSummary[],
  reviewStates: ReadonlyMap<string, FileReviewState>,
  cursorPath: string | null,
  delta: 1 | -1,
): string | null {
  if (files.length === 0) {
    return null;
  }
  const cursor =
    cursorPath === null ? -1 : files.findIndex((f) => f.path === cursorPath);
  const start = cursor === -1 ? (delta > 0 ? -1 : files.length) : cursor;
  for (let step = 1; step <= files.length; step++) {
    const i =
      (((start + delta * step) % files.length) + files.length) % files.length;
    const path = files[i].path;
    if (reviewStates.get(path) !== "reviewed") {
      return path;
    }
  }
  return null;
}
