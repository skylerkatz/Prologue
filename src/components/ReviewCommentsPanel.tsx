import { useRef, useState } from "react";
import type { Comment, CommentState } from "../types";
import { CommentCard, CommentComposer, type DraftStore } from "./Comments";

interface ReviewCommentsPanelProps {
  /** Review-level comments only. */
  comments: Comment[];
  onCreate: (body: string) => Promise<void>;
  onUpdate: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onSetState: (id: number, state: CommentState) => Promise<void>;
}

/** Overall review notes, pinned above the diff. */
export function ReviewCommentsPanel({
  comments,
  onCreate,
  onUpdate,
  onDelete,
  onSetState,
}: ReviewCommentsPanelProps) {
  const [adding, setAdding] = useState(false);
  const [editingId, setEditingId] = useState<number | null>(null);
  const drafts = useRef<DraftStore>(new Map());

  return (
    <section className="review-comments" aria-label="Review comments">
      <header className="review-comments-header">
        <span>
          Review comments
          {comments.length > 0 && ` (${comments.length})`}
        </span>
        {!adding && (
          <button
            type="button"
            className="add-comment-button"
            onClick={() => setAdding(true)}
          >
            + Add review comment
          </button>
        )}
      </header>
      {comments.map((comment) => (
        <CommentCard
          key={comment.id}
          comment={comment}
          editing={editingId === comment.id}
          drafts={drafts.current}
          onEditStart={setEditingId}
          onEditCancel={() => setEditingId(null)}
          onSave={(id, body) =>
            onUpdate(id, body).then(() => setEditingId(null))
          }
          onDelete={onDelete}
          onSetState={onSetState}
        />
      ))}
      {adding && (
        <CommentComposer
          drafts={drafts.current}
          draftKey="review-new"
          placeholder="Overall notes about this review…"
          onSubmit={(body) => onCreate(body).then(() => setAdding(false))}
          onCancel={() => setAdding(false)}
        />
      )}
    </section>
  );
}
