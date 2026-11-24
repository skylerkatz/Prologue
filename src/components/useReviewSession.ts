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
  loadReview,
  openReview,
  reloadComments,
  reanchorComments,
  type GuideCliStatus,
} from "../ipc";
import { isTauri } from "../platform";
import { takeBootReview } from "../web";
import { primeFileDiffs } from "../diff/prefetch";
import { warmHighlighter } from "../highlight/shiki";
import { useTauriEvent } from "../useTauriEvent";
import { COMMENTS_CHANGED_EVENT } from "../generated/events";
import type {
  AnchorStatus,
  Comment,
  DiffSummary,
  Guide,
  Review,
  DiffMode,
} from "../types";

/**
 * A summary pinned to the parameters it was computed with, so the diff view
 * never fetches file hunks against refs the summary doesn't describe.
 */
export interface DiffViewState {
  summary: DiffSummary;
  base: string;
  head: string;
  mode: DiffMode;
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
  /** Web mode: the guide and engine status read with the review. */
  guideSeed: GuideSeed | null;
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
  mode: DiffMode,
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

const NO_REVIEW_DATA = {
  anchors: new Map<number, AnchorStatus>(),
  comments: [] as Comment[],
  reviewedFiles: new Map<string, string>(),
};

/** The app's load: local IPC calls are cheap, so it makes them one by one. */
async function loadInApp(
  repoPath: string,
  branch: string,
  base: string,
  mode: DiffMode,
  ignoreWhitespace: boolean,
) {
  // The summary is independent of review bookkeeping, so it runs in
  // parallel with the auto-archive pass (reviews whose branch merged or
  // vanished) — which must itself land before resuming (or starting)
  // this branch's review.
  const [summary] = await Promise.all([
    getDiffSummary(repoPath, base, branch, mode, ignoreWhitespace),
    archiveStaleReviews(repoPath),
  ]);
  const opened = await openReview(repoPath, branch, base, mode);
  const data =
    opened.review !== null && opened.review.status === "active"
      ? await fetchReviewData(repoPath, base, branch, mode, opened.review.id)
      : NO_REVIEW_DATA;
  return { summary, opened, data, seed: null };
}

/** Web mode's load: each call is a round trip through the portal, so the
 * server does the whole load in one (`load_review`) — or already did, and
 * inlined it into the page for the first render. */
async function loadOnServer(
  repoPath: string,
  branch: string,
  base: string,
  mode: DiffMode,
  ignoreWhitespace: boolean,
) {
  const loaded =
    takeBootReview(repoPath, branch, base, mode, ignoreWhitespace) ??
    (await loadReview(repoPath, branch, base, mode, ignoreWhitespace));
  const params = { repoPath, base, head: branch, mode, ignoreWhitespace };
  primeFileDiffs(params, loaded.summary.files, loaded.fileDiffs);
  warmHighlighter(loaded.summary.files.map((f) => f.path));
  return {
    summary: loaded.summary,
    opened: { review: loaded.review, branchMerged: loaded.branchMerged },
    data: {
      anchors: new Map<number, AnchorStatus>(
        loaded.anchors.map((r) => [r.commentId, r.status]),
      ),
      comments: loaded.comments,
      reviewedFiles: new Map<string, string>(
        loaded.reviewedFiles.map((r) => [r.filePath, r.fingerprint]),
      ),
    },
    seed: { guide: loaded.guide, guideCli: loaded.guideCli },
  };
}

/** The guide state `load_review` already read for a view, so useGuide need
 * not ask again for that generation. */
export interface GuideSeed {
  generation: number;
  guide: Guide | null;
  guideCli: GuideCliStatus;
}

/**
 * Owns the review session for one (repo, branch, base, mode, whitespace)
 * combination: computes the diff summary, resumes (or starts) the branch's
 * review, and keeps its comments fresh — including the `comments-changed`
 * re-read when other writers (the prologue CLI, other web viewers) commit
 * to reviews.db.
 */
export function useReviewSession(
  repoPath: string,
  branch: string,
  baseBranch: string,
  mode: DiffMode,
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
  const refetchSeq = useRef(0);
  const [guideSeed, setGuideSeed] = useState<GuideSeed | null>(null);

  useEffect(() => {
    if (!branch || !baseBranch) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    (async () => {
      const { summary, opened, data, seed } = isTauri()
        ? await loadInApp(repoPath, branch, baseBranch, mode, hideWhitespace)
        : await loadOnServer(repoPath, branch, baseBranch, mode, hideWhitespace);
      const active =
        opened.review !== null && opened.review.status === "active"
          ? opened.review
          : null;
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
      setGuideSeed(seed && { ...seed, generation: generation.current });
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
  // prologue-server also emits it after every web comment write, so each
  // viewer sees the others' comments; the writer's own refetch is harmless.
  // Re-read comments — with a re-anchor pass — without recomputing the diff.
  useTauriEvent(COMMENTS_CHANGED_EVENT, () => {
    const pinned = current.current;
    if (pinned === null) {
      return;
    }
    // Viewers' writes arrive in bursts; only the newest refetch may land,
    // or a slow older one could put back a comment just deleted.
    const seq = ++refetchSeq.current;
    void (async () => {
      // Comments only: reviewed marks are this viewer's own progress, and
      // re-reading them here would flip another web viewer's Viewed boxes
      // (and collapse files) mid-review, or undo a toggle still in flight.
      const args = [
        pinned.review.repoPath,
        pinned.view.base,
        pinned.view.head,
        pinned.view.mode,
        pinned.review.id,
      ] as const;
      const { anchors: results, comments } = isTauri()
        ? await reanchorComments(...args).then(async (anchors) => ({
            anchors,
            comments: await listComments(pinned.review.id),
          }))
        : await reloadComments(...args);
      // The view may have moved on (branch switch, refresh) meanwhile.
      if (
        current.current?.review.id !== pinned.review.id ||
        seq !== refetchSeq.current
      ) {
        return;
      }
      setAnchorStatuses(new Map(results.map((r) => [r.commentId, r.status])));
      setComments(comments);
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
    guideSeed,
  };
}
