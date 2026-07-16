import { useEffect, useRef, useState } from "react";
import { listArchivedReviews, listComments } from "../ipc";
import type { ArchivedReview, Comment } from "../types";
import { CommentCard, type DraftStore } from "./Comments";

interface ArchivedReviewsProps {
  repoPath: string;
  onClose: () => void;
}

/**
 * Read-only browser for archived reviews (branch merged into base or
 * deleted). List first; selecting a review shows its comments. Nothing here
 * can mutate — the backend refuses writes to archived reviews too.
 */
export function ArchivedReviews({ repoPath, onClose }: ArchivedReviewsProps) {
  const [reviews, setReviews] = useState<ArchivedReview[] | null>(null);
  const [selected, setSelected] = useState<ArchivedReview | null>(null);
  const [comments, setComments] = useState<Comment[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // CommentCard requires a draft store even though read-only cards never
  // write to it.
  const drafts = useRef<DraftStore>(new Map());

  useEffect(() => {
    let cancelled = false;
    listArchivedReviews(repoPath)
      .then((list) => {
        if (!cancelled) {
          setReviews(list);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(typeof e === "string" ? e : String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  const open = (review: ArchivedReview) => {
    setSelected(review);
    setComments(null);
    listComments(review.id)
      .then(setComments)
      .catch((e: unknown) => setError(typeof e === "string" ? e : String(e)));
  };

  const noop = () => Promise.resolve();

  return (
    <div className="archive-overlay" role="dialog" aria-label="Archived reviews">
      <div className="archive-panel">
        <header className="archive-header">
          {selected !== null ? (
            <button
              type="button"
              className="comment-action"
              onClick={() => setSelected(null)}
            >
              ← All archived reviews
            </button>
          ) : (
            <span className="archive-title">Archived reviews</span>
          )}
          <span className="archive-readonly">read-only</span>
          <span className="comment-header-spacer" />
          <button type="button" className="comment-action" onClick={onClose}>
            Close
          </button>
        </header>
        {error !== null && <p className="composer-error">{error}</p>}
        {selected === null ? (
          reviews === null ? (
            <p className="archive-empty">Loading…</p>
          ) : reviews.length === 0 ? (
            <p className="archive-empty">
              No archived reviews for this repository yet. Reviews archive
              automatically when their branch is merged or deleted.
            </p>
          ) : (
            <ul className="archive-list">
              {reviews.map((review) => (
                <li key={review.id}>
                  <button
                    type="button"
                    className="archive-row"
                    onClick={() => open(review)}
                  >
                    <span className="archive-branch">{review.branch}</span>
                    <span className="archive-base">← {review.baseRef}</span>
                    <span className="comment-header-spacer" />
                    <span className="archive-count">
                      {review.commentCount}{" "}
                      {review.commentCount === 1 ? "comment" : "comments"}
                    </span>
                    <span className="archive-date">
                      {formatDate(review.updatedAt)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )
        ) : (
          <div className="archive-detail">
            <p className="archive-detail-meta">
              <strong>{selected.branch}</strong> ← {selected.baseRef} ·{" "}
              {selected.mode} · archived {formatDate(selected.updatedAt)}
            </p>
            {comments === null ? (
              <p className="archive-empty">Loading comments…</p>
            ) : comments.length === 0 ? (
              <p className="archive-empty">This review has no comments.</p>
            ) : (
              comments.map((comment) => (
                <div key={comment.id} className="archive-comment">
                  {comment.filePath !== null && (
                    <div className="orphaned-origin">
                      <code>{comment.filePath}</code>
                    </div>
                  )}
                  {comment.codeAnchor !== null && (
                    <pre className="orphaned-anchor">
                      {comment.codeAnchor.lines.join("\n")}
                    </pre>
                  )}
                  <CommentCard
                    comment={comment}
                    editing={false}
                    drafts={drafts.current}
                    readOnly
                    onEditStart={() => {}}
                    onEditCancel={() => {}}
                    onSave={noop}
                    onDelete={noop}
                  />
                </div>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function formatDate(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
}
