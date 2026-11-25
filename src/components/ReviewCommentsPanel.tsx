import type { Comment, CommentState, RepliesByRoot } from "../types";
import { CommentComposer, CommentThread, useThreadEditing } from "./Comments";

interface ReviewCommentsPanelProps {
  /** Review-level thread roots only. */
  comments: Comment[];
  /** Replies grouped by root, shared across the whole review. */
  repliesByRoot: RepliesByRoot;
  onCreate: (body: string) => Promise<void>;
  onCreateReply: (rootId: number, body: string) => Promise<void>;
  onUpdate: (id: number, body: string, expectedUpdatedAt?: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onSetState: (id: number, state: CommentState) => Promise<void>;
  /** The new-comment composer is open; ReviewShell owns it so the phone
   * toolbar's menu can open it too. */
  adding: boolean;
  onAddingChange: (adding: boolean) => void;
  /** Phone layout: render nothing while there is nothing to show, and
   * leave "Add review comment" to the toolbar menu. */
  compact?: boolean;
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
  adding,
  onAddingChange: setAdding,
  compact = false,
}: ReviewCommentsPanelProps) {
  const threadEditing = useThreadEditing(onUpdate, onCreateReply);

  if (compact && comments.length === 0 && !adding) {
    return null;
  }

  return (
    <section className="review-comments" aria-label="Review comments">
      <header className="review-comments-header">
        <span>
          Review comments
          {comments.length > 0 && ` (${comments.length})`}
        </span>
        {!adding && !compact && (
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
          {...threadEditing}
          onDelete={onDelete}
          onSetState={onSetState}
        />
      ))}
      {adding && (
        <CommentComposer
          drafts={threadEditing.drafts}
          draftKey="review-new"
          placeholder="Overall notes about this review…"
          onSubmit={(body) => onCreate(body).then(() => setAdding(false))}
          onCancel={() => setAdding(false)}
        />
      )}
    </section>
  );
}
