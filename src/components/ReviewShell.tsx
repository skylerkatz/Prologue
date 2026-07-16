import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  archiveStaleReviews,
  createComment,
  deleteComment,
  getDiffSummary,
  listComments,
  openReview,
  reanchorComments,
  updateComment,
  updateCommentState,
} from "../ipc";
import type {
  AnchorStatus,
  BranchList,
  Comment,
  CommentState,
  DiffSummary,
  NewCommentInput,
  RepoInfo,
  Review,
  WorkingTreeMode,
} from "../types";
import { ArchivedReviews } from "./ArchivedReviews";
import { DiffView } from "./DiffView";
import { FileList } from "./FileList";
import { ModeToggle } from "./ModeToggle";
import { OrphanedComments } from "./OrphanedComments";
import { ReviewCommentsPanel } from "./ReviewCommentsPanel";

interface ReviewShellProps {
  repo: RepoInfo;
  branchList: BranchList;
  branch: string;
  baseBranch: string;
  mode: WorkingTreeMode;
  refreshKey: number;
  onBranchChange: (branch: string) => void;
  onBaseBranchChange: (base: string) => void;
  onModeChange: (mode: WorkingTreeMode) => void;
  onRefresh: () => void;
  onSwitchRepo: () => void;
}

/**
 * A summary pinned to the parameters it was computed with, so the diff view
 * never fetches file hunks against refs the summary doesn't describe.
 */
interface DiffViewState {
  summary: DiffSummary;
  base: string;
  head: string;
  mode: WorkingTreeMode;
  /** Bumped per fetch; remounts DiffView so lazy-loaded hunks reset too. */
  generation: number;
}

export function ReviewShell({
  repo,
  branchList,
  branch,
  baseBranch,
  mode,
  refreshKey,
  onBranchChange,
  onBaseBranchChange,
  onModeChange,
  onRefresh,
  onSwitchRepo,
}: ReviewShellProps) {
  const [view, setView] = useState<DiffViewState | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [branchMerged, setBranchMerged] = useState(false);
  const [comments, setComments] = useState<Comment[]>([]);
  const [anchorStatuses, setAnchorStatuses] = useState<
    ReadonlyMap<number, AnchorStatus>
  >(new Map());
  const [showArchive, setShowArchive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [scrollTarget, setScrollTarget] = useState<{
    path: string;
    nonce: number;
  } | null>(null);
  const generation = useRef(0);
  // Pinned view params for comment creation; a ref so the DiffView callbacks
  // stay stable across re-renders.
  const current = useRef<{ review: Review; view: DiffViewState } | null>(null);

  useEffect(() => {
    if (!branch || !baseBranch) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    (async () => {
      const summary = await getDiffSummary(repo.path, baseBranch, branch, mode);
      // Auto-archive reviews whose branch merged or vanished, then resume
      // (or start) this branch's review.
      await archiveStaleReviews(repo.path);
      const opened = await openReview(repo.path, branch, baseBranch, mode);
      let reviewComments: Comment[] = [];
      let anchors: ReadonlyMap<number, AnchorStatus> = new Map();
      if (opened.review !== null && opened.review.status === "active") {
        // Re-locate line comments in the recomputed diff before reading
        // them back (moves are persisted server-side).
        const results = await reanchorComments(
          repo.path,
          baseBranch,
          branch,
          mode,
          opened.review.id,
        );
        anchors = new Map(results.map((r) => [r.commentId, r.status]));
        reviewComments = await listComments(opened.review.id);
      }
      if (cancelled) {
        return;
      }
      generation.current += 1;
      const nextView = {
        summary,
        base: baseBranch,
        head: branch,
        mode,
        generation: generation.current,
      };
      // Comment creation stays possible only on an active review.
      current.current =
        opened.review !== null && opened.review.status === "active"
          ? { review: opened.review, view: nextView }
          : null;
      setView(nextView);
      setReview(opened.review);
      setBranchMerged(opened.branchMerged);
      setComments(reviewComments);
      setAnchorStatuses(anchors);
      setScrollTarget(null);
    })()
      .catch((e: unknown) => {
        if (!cancelled) {
          current.current = null;
          setView(null);
          setReview(null);
          setBranchMerged(false);
          setComments([]);
          setAnchorStatuses(new Map());
          setError(typeof e === "string" ? e : String(e));
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
  }, [repo.path, branch, baseBranch, mode, refreshKey]);

  const handleCreate = useCallback(async (input: NewCommentInput) => {
    const pinned = current.current;
    if (pinned === null) {
      throw "No active review";
    }
    const comment = await createComment(
      pinned.review.repoPath,
      pinned.view.base,
      pinned.view.head,
      pinned.view.mode,
      { reviewId: pinned.review.id, ...input },
    );
    setComments((prev) => [...prev, comment]);
  }, []);

  const handleUpdate = useCallback(async (id: number, body: string) => {
    const updated = await updateComment(id, body);
    setComments((prev) => prev.map((c) => (c.id === id ? updated : c)));
  }, []);

  const handleDelete = useCallback(async (id: number) => {
    await deleteComment(id);
    setComments((prev) => prev.filter((c) => c.id !== id));
  }, []);

  const handleSetState = useCallback(
    async (id: number, state: CommentState) => {
      const updated = await updateCommentState(id, state);
      setComments((prev) => prev.map((c) => (c.id === id ? updated : c)));
    },
    [],
  );

  const handleCreateReviewComment = useCallback(
    (body: string) => handleCreate({ level: "review", body }),
    [handleCreate],
  );

  const reviewComments = useMemo(
    () => comments.filter((c) => c.level === "review"),
    [comments],
  );

  /**
   * Comments whose place in the diff is gone: line comments the re-anchor
   * pass orphaned, plus file/line comments on files that left the diff.
   * They render in the orphaned bucket, never inline at a stale position.
   */
  const orphanedComments = useMemo(() => {
    const paths = new Set(view?.summary.files.map((f) => f.path) ?? []);
    return comments.filter(
      (c) =>
        c.level !== "review" &&
        (anchorStatuses.get(c.id) === "orphaned" ||
          (c.filePath !== null && !paths.has(c.filePath))),
    );
  }, [comments, anchorStatuses, view]);

  const diffComments = useMemo(() => {
    const orphaned = new Set(orphanedComments.map((c) => c.id));
    return comments.filter((c) => !orphaned.has(c.id));
  }, [comments, orphanedComments]);

  const openCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const comment of comments) {
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

  return (
    <div className="review-shell">
      <header className="toolbar">
        <button
          type="button"
          className="repo-button"
          title={repo.path}
          onClick={onSwitchRepo}
        >
          {repo.name}
        </button>
        <label className="control">
          <span>Base</span>
          <select
            value={baseBranch}
            onChange={(e) => onBaseBranchChange(e.currentTarget.value)}
          >
            {branchList.branches.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
        </label>
        <span className="arrow" aria-hidden="true">
          ←
        </span>
        <label className="control">
          <span>Branch</span>
          <select
            value={branch}
            onChange={(e) => onBranchChange(e.currentTarget.value)}
          >
            {branchList.branches.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
        </label>
        <div className="toolbar-spacer" />
        <ModeToggle mode={mode} onChange={onModeChange} />
        <button
          type="button"
          className="refresh-button"
          title="Browse archived reviews (read-only)"
          onClick={() => setShowArchive(true)}
        >
          Archived
        </button>
        <button
          type="button"
          className="refresh-button"
          title="Refresh branches and diff"
          onClick={onRefresh}
          disabled={loading}
        >
          ↻ Refresh
        </button>
      </header>
      <main className="diff-main">
        {error !== null ? (
          <div className="diff-empty">
            <p className="error">{error}</p>
          </div>
        ) : view === null ? (
          <div className="diff-empty">
            <p>{loading ? "Computing diff…" : "Select branches to diff."}</p>
          </div>
        ) : branchMerged ? (
          <div className="diff-empty">
            <p>
              <code>{branch}</code> is merged into <code>{baseBranch}</code>
              {review !== null
                ? " — its review is archived and read-only."
                : "."}
            </p>
            {review !== null && (
              <button
                type="button"
                className="refresh-button"
                onClick={() => setShowArchive(true)}
              >
                View archived reviews
              </button>
            )}
          </div>
        ) : review === null ? (
          <div className="diff-empty">
            <p>No review for this branch.</p>
          </div>
        ) : view.summary.files.length === 0 && comments.length === 0 ? (
          <div className="diff-empty">
            <p>
              No changes between <code>{view.summary.baseRef}</code> and{" "}
              <code>{view.summary.headRef}</code>.
            </p>
          </div>
        ) : (
          <div className="review-body">
            <aside className="file-sidebar">
              <FileList
                summary={view.summary}
                openCounts={openCounts}
                onSelect={(path) =>
                  setScrollTarget((prev) => ({
                    path,
                    nonce: (prev?.nonce ?? 0) + 1,
                  }))
                }
              />
            </aside>
            <div className="diff-pane">
              <ReviewCommentsPanel
                comments={reviewComments}
                onCreate={handleCreateReviewComment}
                onUpdate={handleUpdate}
                onDelete={handleDelete}
                onSetState={handleSetState}
              />
              <OrphanedComments
                comments={orphanedComments}
                onUpdate={handleUpdate}
                onDelete={handleDelete}
                onSetState={handleSetState}
              />
              <DiffView
                key={view.generation}
                repoPath={repo.path}
                base={view.base}
                head={view.head}
                mode={view.mode}
                summary={view.summary}
                scrollTarget={scrollTarget}
                comments={diffComments}
                anchorStatuses={anchorStatuses}
                onCreateComment={handleCreate}
                onUpdateComment={handleUpdate}
                onDeleteComment={handleDelete}
                onSetCommentState={handleSetState}
              />
            </div>
          </div>
        )}
      </main>
      {showArchive && (
        <ArchivedReviews
          repoPath={repo.path}
          onClose={() => setShowArchive(false)}
        />
      )}
    </div>
  );
}
