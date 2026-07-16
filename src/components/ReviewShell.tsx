import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  createComment,
  deleteComment,
  getDiffSummary,
  listComments,
  openReview,
  updateComment,
} from "../ipc";
import type {
  BranchList,
  Comment,
  DiffSummary,
  NewCommentInput,
  RepoInfo,
  Review,
  WorkingTreeMode,
} from "../types";
import { DiffView } from "./DiffView";
import { FileList } from "./FileList";
import { ModeToggle } from "./ModeToggle";
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
  const [comments, setComments] = useState<Comment[]>([]);
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
      // Resume (or start) the branch's active review and pull its comments.
      const activeReview = await openReview(repo.path, branch, baseBranch, mode);
      const reviewComments = await listComments(activeReview.id);
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
      current.current = { review: activeReview, view: nextView };
      setView(nextView);
      setReview(activeReview);
      setComments(reviewComments);
      setScrollTarget(null);
    })()
      .catch((e: unknown) => {
        if (!cancelled) {
          current.current = null;
          setView(null);
          setReview(null);
          setComments([]);
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

  const handleCreateReviewComment = useCallback(
    (body: string) => handleCreate({ level: "review", body }),
    [handleCreate],
  );

  const reviewComments = useMemo(
    () => comments.filter((c) => c.level === "review"),
    [comments],
  );

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
        ) : view === null || review === null ? (
          <div className="diff-empty">
            <p>{loading ? "Computing diff…" : "Select branches to diff."}</p>
          </div>
        ) : view.summary.files.length === 0 ? (
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
              />
              <DiffView
                key={view.generation}
                repoPath={repo.path}
                base={view.base}
                head={view.head}
                mode={view.mode}
                summary={view.summary}
                scrollTarget={scrollTarget}
                comments={comments}
                onCreateComment={handleCreate}
                onUpdateComment={handleUpdate}
                onDeleteComment={handleDelete}
              />
            </div>
          </div>
        )}
      </main>
    </div>
  );
}
