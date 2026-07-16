import { useRef, useState } from "react";
import type { Comment, CommentState, RepliesByRoot } from "../types";
import { CommentComposer, CommentThread, type DraftStore } from "./Comments";

interface ReviewCommentsPanelProps {
  /** Review-level thread roots only. */
  comments: Comment[];
  /** Replies grouped by root, shared across the whole review. */
  repliesByRoot: RepliesByRoot;
  onCreate: (body: string) => Promise<void>;
  onCreateReply: (rootId: number, body: string) => Promise<void>;
  onUpdate: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onSetState: (id: number, state: CommentState) => Promise<void>;
}

/** Overall review notes, pinned above the diff. */
export function ReviewCommentsPanel({
  comments,
  repliesByRoot,
  onCreate,
  onCreateReply,
  onUpdate,
  onDelete,
  onSetState,
}: ReviewCommentsPanelProps) {
  const [adding, setAdding] = useState(false);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [replyingTo, setReplyingTo] = useState<number | null>(null);
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
        <CommentThread
          key={comment.id}
          root={comment}
          replies={repliesByRoot.get(comment.id) ?? []}
          editingId={editingId}
          drafts={drafts.current}
          replyingTo={replyingTo}
          onReplyStart={setReplyingTo}
          onReplyCancel={() => setReplyingTo(null)}
          onCreateReply={(rootId, body) =>
            onCreateReply(rootId, body).then(() => setReplyingTo(null))
          }
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
