import { useState } from "react";
import type { Comment, CommentState, RepliesByRoot } from "../types";
import { Chevron } from "./Chevron";
import { CommentThread, useThreadEditing } from "./Comments";

interface OrphanedCommentsProps {
  /** Thread roots whose anchor (or file) is gone from the current diff. */
  comments: Comment[];
  /** Replies grouped by root; orphaned threads keep theirs. */
  repliesByRoot: RepliesByRoot;
  onCreateReply: (rootId: number, body: string) => Promise<void>;
  onUpdate: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onSetState: (id: number, state: CommentState) => Promise<void>;
}

/**
 * The orphaned bucket: comments whose code can no longer be located in the
 * diff. They are shown here — with their last known location and captured
 * code — instead of being silently dropped or misplaced inline. Whole
 * threads orphan together: replies stay under their root.
 */
export function OrphanedComments({
  comments,
  repliesByRoot,
  onCreateReply,
  onUpdate,
  onDelete,
  onSetState,
}: OrphanedCommentsProps) {
  const threadEditing = useThreadEditing(onUpdate, onCreateReply);
  // Collapse the whole bucket to its header row, like a file card.
  // Starts minimized: orphans are reference material, not the review's
  // main path — expand on demand.
  const [expanded, setExpanded] = useState(false);

  if (comments.length === 0) {
    return null;
  }
  return (
    <section className="orphaned-comments" aria-label="Orphaned comments">
      <header className="orphaned-header">
        <button
          type="button"
          className="file-toggle"
          aria-expanded={expanded}
          aria-label={
            expanded ? "Collapse orphaned comments" : "Expand orphaned comments"
          }
          onClick={() => setExpanded((v) => !v)}
        >
          <Chevron expanded={expanded} />
        </button>
        <span className="orphaned-title">
          Orphaned comments ({comments.length})
        </span>
        {expanded && (
          <span className="orphaned-hint">
            The commented code is no longer in this diff — resolve, dismiss, or
            keep for reference.
          </span>
        )}
      </header>
      {expanded &&
        comments.map((comment) => (
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
            <CommentThread
              root={comment}
              replies={repliesByRoot.get(comment.id) ?? []}
              {...threadEditing}
              onDelete={onDelete}
              onSetState={onSetState}
            />
          </div>
        ))}
    </section>
  );
}
