import { useRef, useState } from "react";
import type { Comment, CommentState } from "../types";
import { CommentCard, type DraftStore } from "./Comments";

interface OrphanedCommentsProps {
  /** Comments whose anchor (or file) is gone from the current diff. */
  comments: Comment[];
  onUpdate: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onSetState: (id: number, state: CommentState) => Promise<void>;
}

/**
 * The orphaned bucket: comments whose code can no longer be located in the
 * diff. They are shown here — with their last known location and captured
 * code — instead of being silently dropped or misplaced inline.
 */
export function OrphanedComments({
  comments,
  onUpdate,
  onDelete,
  onSetState,
}: OrphanedCommentsProps) {
  const [editingId, setEditingId] = useState<number | null>(null);
  const drafts = useRef<DraftStore>(new Map());

  if (comments.length === 0) {
    return null;
  }
  return (
    <section className="orphaned-comments" aria-label="Orphaned comments">
      <header className="orphaned-header">
        <span className="orphaned-title">
          Orphaned comments ({comments.length})
        </span>
        <span className="orphaned-hint">
          The commented code is no longer in this diff — resolve, dismiss, or
          keep for reference.
        </span>
      </header>
      {comments.map((comment) => (
        <div key={comment.id} className="orphaned-comment">
          <div className="orphaned-origin">
            <code>{comment.filePath}</code>
            {comment.startLine !== null && (
              <span className="orphaned-lines">
                was at{" "}
                {comment.startLine === comment.endLine
                  ? `line ${comment.startLine}`
                  : `lines ${comment.startLine}–${comment.endLine}`}
                {comment.side === "old" ? " (old)" : ""}
              </span>
            )}
          </div>
          {comment.codeAnchor !== null && (
            <pre className="orphaned-anchor">
              {comment.codeAnchor.lines.join("\n")}
            </pre>
          )}
          <CommentCard
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
        </div>
      ))}
    </section>
  );
}
