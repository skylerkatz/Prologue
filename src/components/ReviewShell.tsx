import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  archiveStaleReviews,
  createComment,
  deleteComment,
  getDiffSummary,
  listComments,
  listReviewedFiles,
  markFileReviewed,
  openReview,
  reanchorComments,
  unmarkFileReviewed,
  updateComment,
  updateCommentState,
} from "../ipc";
import {
  groupReplies,
  type AnchorStatus,
  type BranchList,
  type Comment,
  type CommentState,
  type DiffSummary,
  type FileReviewState,
  type NewCommentInput,
  type RepoInfo,
  type Review,
  type ReviewedFile,
  type WorkingTreeMode,
} from "../types";
import { ArchivedReviews } from "./ArchivedReviews";
import { DiffView } from "./DiffView";
import { ExportMenu, type ExportTarget } from "./ExportMenu";
import { FileList } from "./FileList";
import { ModeToggle } from "./ModeToggle";
import { OrphanedComments } from "./OrphanedComments";
import { ReviewCommentsPanel } from "./ReviewCommentsPanel";
import { WhitespaceToggle } from "./WhitespaceToggle";

interface ReviewShellProps {
  repo: RepoInfo;
  branchList: BranchList;
  branch: string;
  baseBranch: string;
  mode: WorkingTreeMode;
  hideWhitespace: boolean;
  refreshKey: number;
  onBranchChange: (branch: string) => void;
  onBaseBranchChange: (base: string) => void;
  onModeChange: (mode: WorkingTreeMode) => void;
  onHideWhitespaceChange: (hide: boolean) => void;
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
  /** The summary was computed with whitespace changes ignored (git `-w`). */
  ignoreWhitespace: boolean;
  /** Bumped per fetch; remounts DiffView so lazy-loaded hunks reset too. */
  generation: number;
}

export function ReviewShell({
  repo,
  branchList,
  branch,
  baseBranch,
  mode,
  hideWhitespace,
  refreshKey,
  onBranchChange,
  onBaseBranchChange,
  onModeChange,
  onHideWhitespaceChange,
  onRefresh,
  onSwitchRepo,
}: ReviewShellProps) {
  const [view, setView] = useState<DiffViewState | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [branchMerged, setBranchMerged] = useState(false);
  const [comments, setComments] = useState<Comment[]>([]);
  // Path → fingerprint stored at mark time; compared against the current
  // summary's fingerprints to derive reviewed vs "changed since review".
  const [reviewedFiles, setReviewedFiles] = useState<
    ReadonlyMap<string, string>
  >(new Map());
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
      const summary = await getDiffSummary(
        repo.path,
        baseBranch,
        branch,
        mode,
        hideWhitespace,
      );
      // Auto-archive reviews whose branch merged or vanished, then resume
      // (or start) this branch's review.
      await archiveStaleReviews(repo.path);
      const opened = await openReview(repo.path, branch, baseBranch, mode);
      let reviewComments: Comment[] = [];
      let anchors: ReadonlyMap<number, AnchorStatus> = new Map();
      let reviewedRows: ReviewedFile[] = [];
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
        reviewedRows = await listReviewedFiles(opened.review.id);
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
        ignoreWhitespace: hideWhitespace,
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
      // Same commit as setView, so the remounted DiffView initializes its
      // collapse state against the fresh reviewed map.
      setReviewedFiles(
        new Map(reviewedRows.map((r) => [r.filePath, r.fingerprint])),
      );
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
          setReviewedFiles(new Map());
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
  }, [repo.path, branch, baseBranch, mode, hideWhitespace, refreshKey]);

  // External writers (the prologue CLI) commit to reviews.db behind the
  // app's back; the backend's database watcher emits `comments-changed`
  // only for those (its data_version guard filters the app's own writes).
  // Re-read comments — with a re-anchor pass — without recomputing the diff.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen("comments-changed", () => {
      const pinned = current.current;
      if (pinned === null) {
        return;
      }
      void (async () => {
        const results = await reanchorComments(
          pinned.review.repoPath,
          pinned.view.base,
          pinned.view.head,
          pinned.view.mode,
          pinned.review.id,
        );
        const reviewComments = await listComments(pinned.review.id);
        const reviewedRows = await listReviewedFiles(pinned.review.id);
        // The view may have moved on (branch switch, refresh) meanwhile.
        if (disposed || current.current?.review.id !== pinned.review.id) {
          return;
        }
        setAnchorStatuses(new Map(results.map((r) => [r.commentId, r.status])));
        setComments(reviewComments);
        setReviewedFiles(
          new Map(reviewedRows.map((r) => [r.filePath, r.fingerprint])),
        );
      })().catch(() => {
        // Leave the current comments in place; the manual Refresh button
        // still covers everything.
      });
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

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

  // Ref mirror so the stable toggle callback reads the latest map without
  // depending on it (state updaters run at render time, too late to pick
  // the IPC call from inside one).
  const reviewedRef = useRef(reviewedFiles);
  reviewedRef.current = reviewedFiles;

  /** Toggle a file's reviewed mark, optimistically: marking stores the
   * fingerprint the user is looking at; unmarking deletes the row. */
  const handleToggleReviewed = useCallback((path: string) => {
    const pinned = current.current;
    if (pinned === null) {
      return;
    }
    const file = pinned.view.summary.files.find((f) => f.path === path);
    if (file === undefined) {
      return;
    }
    const previous = reviewedRef.current;
    const wasReviewed = previous.get(path) === file.fingerprint;
    const next = new Map(previous);
    if (wasReviewed) {
      next.delete(path);
    } else {
      // Marking an unreviewed or "changed since review" file both store the
      // fingerprint currently on screen — the user reviewed what they saw.
      next.set(path, file.fingerprint);
    }
    setReviewedFiles(next);
    const op = wasReviewed
      ? unmarkFileReviewed(pinned.review.id, path)
      : markFileReviewed(pinned.review.id, path, file.fingerprint);
    op.catch(() => {
      if (current.current?.review.id === pinned.review.id) {
        setReviewedFiles(previous);
      }
    });
  }, []);

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
    // Deleting a thread root cascades its replies server-side; drop them
    // from local state the same way.
    setComments((prev) => prev.filter((c) => c.id !== id && c.parentId !== id));
  }, []);

  const handleCreateReply = useCallback(
    (rootId: number, body: string) =>
      handleCreate({ level: "review", parentId: rootId, body }),
    [handleCreate],
  );

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

  /** Replies grouped under their thread root; threads render as a unit
   * wherever the root lands (inline, review panel, or orphaned bucket). */
  const repliesByRoot = useMemo(() => groupReplies(comments), [comments]);

  const reviewComments = useMemo(
    () => comments.filter((c) => c.level === "review" && c.parentId === null),
    [comments],
  );

  /**
   * Thread roots whose place in the diff is gone: line comments the
   * re-anchor pass orphaned, plus file/line comments on files that left the
   * diff. They render in the orphaned bucket, never inline at a stale
   * position. Replies always follow their root, so they are never orphaned
   * on their own.
   */
  const orphanedComments = useMemo(() => {
    const paths = new Set(view?.summary.files.map((f) => f.path) ?? []);
    return comments.filter(
      (c) =>
        c.parentId === null &&
        c.level !== "review" &&
        (anchorStatuses.get(c.id) === "orphaned" ||
          (c.filePath !== null && !paths.has(c.filePath))),
    );
  }, [comments, anchorStatuses, view]);

  const diffComments = useMemo(() => {
    const orphaned = new Set(orphanedComments.map((c) => c.id));
    return comments.filter((c) => !orphaned.has(c.id));
  }, [comments, orphanedComments]);

  /** Open THREADS: lifecycle lives on roots, so replies never count. */
  const openCount = useMemo(
    () =>
      comments.filter((c) => c.state === "open" && c.parentId === null).length,
    [comments],
  );

  /** What the Export menu would export: the displayed diff's pinned params
   * plus its active review; null (disabled) when there is neither. */
  const exportTarget: ExportTarget | null =
    view !== null && review !== null && review.status === "active" && !branchMerged
      ? {
          repoPath: repo.path,
          base: view.base,
          head: view.head,
          mode: view.mode,
          reviewId: review.id,
        }
      : null;

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
        <WhitespaceToggle
          hidden={hideWhitespace}
          onChange={onHideWhitespaceChange}
        />
        <ExportMenu target={exportTarget} openCount={openCount} />
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
                repoPath={repo.path}
                openCounts={openCounts}
                reviewStates={reviewStates}
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
                repliesByRoot={repliesByRoot}
                onCreate={handleCreateReviewComment}
                onCreateReply={handleCreateReply}
                onUpdate={handleUpdate}
                onDelete={handleDelete}
                onSetState={handleSetState}
              />
              <DiffView
                key={view.generation}
                topContent={
                  <OrphanedComments
                    comments={orphanedComments}
                    repliesByRoot={repliesByRoot}
                    onCreateReply={handleCreateReply}
                    onUpdate={handleUpdate}
                    onDelete={handleDelete}
                    onSetState={handleSetState}
                  />
                }
                repoPath={repo.path}
                base={view.base}
                head={view.head}
                mode={view.mode}
                ignoreWhitespace={view.ignoreWhitespace}
                summary={view.summary}
                scrollTarget={scrollTarget}
                comments={diffComments}
                replies={repliesByRoot}
                anchorStatuses={anchorStatuses}
                reviewStates={reviewStates}
                onToggleReviewed={handleToggleReviewed}
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
