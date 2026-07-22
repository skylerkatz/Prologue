import {
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  archiveStaleReviews,
  errorText,
  getDiffSummary,
  listComments,
  listReviewedFiles,
  openReview,
  reanchorComments,
} from "../ipc";
import { useTauriEvent } from "../useTauriEvent";
import type {
  AnchorStatus,
  Comment,
  DiffSummary,
  Review,
  WorkingTreeMode,
} from "../types";

/**
 * A summary pinned to the parameters it was computed with, so the diff view
 * never fetches file hunks against refs the summary doesn't describe.
 */
export interface DiffViewState {
  summary: DiffSummary;
  base: string;
  head: string;
  mode: WorkingTreeMode;
  /** The summary was computed with whitespace changes ignored (git `-w`). */
  ignoreWhitespace: boolean;
  /** Bumped per fetch; stamps scroll targets so a click made against a
   * previous diff is never replayed. DiffView itself reconciles across
   * fetches instead of remounting. */
  generation: number;
}

/** The active review pinned to the view it was opened against. */
export interface PinnedReview {
  review: Review;
  view: DiffViewState;
}

/** Everything the review surfaces render, owned by useReviewSession. */
export interface ReviewSession {
  view: DiffViewState | null;
  review: Review | null;
  branchMerged: boolean;
  comments: Comment[];
  /** Path → fingerprint stored at mark time; compared against the current
   * summary's fingerprints to derive reviewed vs "changed since review". */
  reviewedFiles: ReadonlyMap<string, string>;
  anchorStatuses: ReadonlyMap<number, AnchorStatus>;
  error: string | null;
  loading: boolean;
  /** Pinned view params for mutations; a ref so useCommentMutations'
   * callbacks stay stable across re-renders. Null unless the review is
   * active — comment creation stays possible only on an active review. */
  current: { current: PinnedReview | null };
  setComments: Dispatch<SetStateAction<Comment[]>>;
  setReviewedFiles: Dispatch<SetStateAction<ReadonlyMap<string, string>>>;
}

/**
 * Read back everything the review stores about the current diff. The
 * re-anchor pass gates the comment read — moves are persisted server-side
 * before listComments sees them — while the reviewed-file rows are
 * independent and load in parallel.
 */
async function fetchReviewData(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  reviewId: number,
) {
  const [[results, comments], reviewedRows] = await Promise.all([
    reanchorComments(repoPath, base, head, mode, reviewId).then(
      async (r) => [r, await listComments(reviewId)] as const,
    ),
    listReviewedFiles(reviewId),
  ]);
  return {
    anchors: new Map<number, AnchorStatus>(
      results.map((r) => [r.commentId, r.status]),
    ),
    comments,
    reviewedFiles: new Map<string, string>(
      reviewedRows.map((r) => [r.filePath, r.fingerprint]),
    ),
  };
}

/**
 * Owns the review session for one (repo, branch, base, mode, whitespace)
 * combination: computes the diff summary, resumes (or starts) the branch's
 * review, and keeps its comments and reviewed marks fresh — including the
 * `comments-changed` re-read when external writers (the prologue CLI)
 * commit to reviews.db behind the app's back.
 */
export function useReviewSession(
  repoPath: string,
  branch: string,
  baseBranch: string,
  mode: WorkingTreeMode,
  hideWhitespace: boolean,
  refreshKey: number,
): ReviewSession {
  const [view, setView] = useState<DiffViewState | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [branchMerged, setBranchMerged] = useState(false);
  const [comments, setComments] = useState<Comment[]>([]);
  const [reviewedFiles, setReviewedFiles] = useState<
    ReadonlyMap<string, string>
  >(new Map());
  const [anchorStatuses, setAnchorStatuses] = useState<
    ReadonlyMap<number, AnchorStatus>
  >(new Map());
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);
  const current = useRef<PinnedReview | null>(null);

  useEffect(() => {
    if (!branch || !baseBranch) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    (async () => {
      // The summary is independent of review bookkeeping, so it runs in
      // parallel with the auto-archive pass (reviews whose branch merged or
      // vanished) — which must itself land before resuming (or starting)
      // this branch's review.
      const [summary] = await Promise.all([
        getDiffSummary(repoPath, baseBranch, branch, mode, hideWhitespace),
        archiveStaleReviews(repoPath),
      ]);
      const opened = await openReview(repoPath, branch, baseBranch, mode);
      const active =
        opened.review !== null && opened.review.status === "active"
          ? opened.review
          : null;
      const data =
        active !== null
          ? await fetchReviewData(repoPath, baseBranch, branch, mode, active.id)
          : {
              anchors: new Map<number, AnchorStatus>(),
              comments: [] as Comment[],
              reviewedFiles: new Map<string, string>(),
            };
      if (cancelled) {
        return;
      }
      generation.current += 1;
      const nextView: DiffViewState = {
        summary,
        base: baseBranch,
        head: branch,
        mode,
        ignoreWhitespace: hideWhitespace,
        generation: generation.current,
      };
      current.current = active !== null ? { review: active, view: nextView } : null;
      setView(nextView);
      setReview(opened.review);
      setBranchMerged(opened.branchMerged);
      setComments(data.comments);
      // Same commit as setView, so DiffView reconciles its collapse state
      // against the fresh reviewed map, never a stale one.
      setReviewedFiles(data.reviewedFiles);
      setAnchorStatuses(data.anchors);
    })()
      .catch((e: unknown) => {
        if (!cancelled) {
          current.current = null;
          setView(null);
          setReview(null);
          setBranchMerged(false);
          setComments([]);
          setReviewedFiles(new Map());
          setAnchorStatuses(new Map());
          setError(errorText(e));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [repoPath, branch, baseBranch, mode, hideWhitespace, refreshKey]);

  // External writers (the prologue CLI) commit to reviews.db behind the
  // app's back; the backend's database watcher emits `comments-changed`
  // only for those (its data_version guard filters the app's own writes).
  // Re-read comments — with a re-anchor pass — without recomputing the diff.
  useTauriEvent("comments-changed", () => {
    const pinned = current.current;
    if (pinned === null) {
      return;
    }
    void (async () => {
      const data = await fetchReviewData(
        pinned.review.repoPath,
        pinned.view.base,
        pinned.view.head,
        pinned.view.mode,
        pinned.review.id,
      );
      // The view may have moved on (branch switch, refresh) meanwhile.
      if (current.current?.review.id !== pinned.review.id) {
        return;
      }
      setAnchorStatuses(data.anchors);
      setComments(data.comments);
      setReviewedFiles(data.reviewedFiles);
    })().catch(() => {
      // Leave the current comments in place; View > Refresh still
      // covers everything.
    });
  });

  return {
    view,
    review,
    branchMerged,
    comments,
    reviewedFiles,
    anchorStatuses,
    error,
    loading,
    current,
    setComments,
    setReviewedFiles,
  };
}
