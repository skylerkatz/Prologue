import { useMemo } from "react";
import { groupReplies, type Comment, type FileReviewState } from "../types";
import type { ReviewSession } from "./useReviewSession";

/**
 * Thread roots stranded by their file leaving the diff entirely. Anchor-
 * orphaned comments whose file is still present are NOT included — they
 * render inside that file's section (tagged as orphaned there), so only
 * file-gone threads fall through to the global bucket. Replies always
 * follow their root, so they are never orphaned on their own.
 */
export function fileGoneRoots(
  comments: Comment[],
  presentPaths: ReadonlySet<string>,
): Comment[] {
  return comments.filter(
    (c) =>
      c.parentId === null &&
      c.level !== "review" &&
      c.filePath !== null &&
      !presentPaths.has(c.filePath),
  );
}

/**
 * Derivations the review surfaces render, computed from the session state:
 * per-file review status, the Hide Resolved filter, and the comment slices
 * for each surface (review panel, inline diff, orphaned bucket).
 */
export function useReviewDerived(session: ReviewSession, hideResolved: boolean) {
  const { view, comments, reviewedFiles } = session;

  /** Per-file review state derived from the displayed summary: a stored
   * fingerprint that still matches is "reviewed"; one that no longer matches
   * is "changed" (changed since review); unmarked files are absent. */
  const reviewStates = useMemo(() => {
    const map = new Map<string, FileReviewState>();
    for (const f of view?.summary.files ?? []) {
      const stored = reviewedFiles.get(f.path);
      if (stored !== undefined) {
        map.set(f.path, stored === f.fingerprint ? "reviewed" : "changed");
      }
    }
    return map;
  }, [view, reviewedFiles]);

  /** What the live surfaces render: with Hide Resolved on, only open thread
   * roots and their replies. Raw `comments` stays the source for mutations
   * and the open-only counts. Resolving a thread while hiding is on makes
   * it vanish immediately; uncheck View > Hide Resolved Comments to see it. */
  const visibleComments = useMemo(() => {
    if (!hideResolved) {
      return comments;
    }
    const openRoots = new Set(
      comments
        .filter((c) => c.parentId === null && c.state === "open")
        .map((c) => c.id),
    );
    return comments.filter((c) =>
      c.parentId === null ? c.state === "open" : openRoots.has(c.parentId),
    );
  }, [comments, hideResolved]);

  /** Replies grouped under their thread root; threads render as a unit
   * wherever the root lands (inline, review panel, or orphaned bucket). */
  const repliesByRoot = useMemo(
    () => groupReplies(visibleComments),
    [visibleComments],
  );

  /** Review-level thread roots, for the pinned panel above the diff. */
  const reviewComments = useMemo(
    () =>
      visibleComments.filter(
        (c) => c.level === "review" && c.parentId === null,
      ),
    [visibleComments],
  );

  /**
   * Thread roots whose file left the diff entirely. Only these render in
   * the global orphaned bucket; anchor-orphaned comments on files still
   * present render inside that file's section instead (see `diffComments`).
   */
  const orphanedComments = useMemo(() => {
    const paths = new Set(view?.summary.files.map((f) => f.path) ?? []);
    return fileGoneRoots(visibleComments, paths);
  }, [visibleComments, view]);

  /** Everything the diff surfaces render, including anchor-orphaned
   * comments whose file is still in the diff — those land in their file's
   * section (tagged as orphaned) rather than at a stale line. */
  const diffComments = useMemo(() => {
    const orphaned = new Set(orphanedComments.map((c) => c.id));
    return visibleComments.filter((c) => !orphaned.has(c.id));
  }, [visibleComments, orphanedComments]);

  /** Open THREADS: lifecycle lives on roots, so replies never count. */
  const openCount = useMemo(
    () =>
      comments.filter((c) => c.state === "open" && c.parentId === null).length,
    [comments],
  );

  /** Open thread roots per file, for the sidebar badges. */
  const openCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const comment of comments) {
      // Replies carry no file path (parentId set ⇒ filePath null), so this
      // already counts thread roots only.
      if (
        comment.level === "review" ||
        comment.filePath === null ||
        comment.state !== "open"
      ) {
        continue;
      }
      counts.set(comment.filePath, (counts.get(comment.filePath) ?? 0) + 1);
    }
    return counts;
  }, [comments]);

  return {
    reviewStates,
    repliesByRoot,
    reviewComments,
    orphanedComments,
    diffComments,
    openCount,
    openCounts,
  };
}
