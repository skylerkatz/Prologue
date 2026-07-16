import { useState } from "react";
import type { Comment } from "../types";

/**
 * Draft store shared by composers and comment editors. Virtualized rows
 * unmount when scrolled far away, so component state alone would lose
 * in-progress text; writing through to this map (held in a ref by the owner)
 * lets a remounted editor pick the draft back up.
 */
export type DraftStore = Map<string, string>;

export const editDraftKey = (commentId: number): string => `edit:${commentId}`;

interface CommentComposerProps {
  drafts: DraftStore;
  draftKey: string;
  placeholder: string;
  submitLabel?: string;
  /** Resolve to close the composer; reject to keep it open with the error. */
  onSubmit: (body: string) => Promise<void>;
  onCancel: () => void;
}

export function CommentComposer({
  drafts,
  draftKey,
  placeholder,
  submitLabel = "Comment",
  onSubmit,
  onCancel,
}: CommentComposerProps) {
  const [text, setText] = useState(() => drafts.get(draftKey) ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    if (text.trim() === "" || busy) {
      return;
    }
    setBusy(true);
    setError(null);
    onSubmit(text)
      .then(() => {
        drafts.delete(draftKey);
      })
      .catch((e: unknown) => {
        setError(typeof e === "string" ? e : String(e));
        setBusy(false);
      });
  };

  const cancel = () => {
    drafts.delete(draftKey);
    onCancel();
  };

  return (
    <div className="comment-composer">
      <textarea
        className="composer-text"
        rows={3}
        autoFocus
        placeholder={placeholder}
        value={text}
        disabled={busy}
        onChange={(e) => {
          setText(e.currentTarget.value);
          drafts.set(draftKey, e.currentTarget.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          }
        }}
      />
      {error !== null && <p className="composer-error">{error}</p>}
      <div className="composer-actions">
        <button
          type="button"
          className="composer-submit"
          disabled={text.trim() === "" || busy}
          onClick={submit}
        >
          {busy ? "Saving…" : submitLabel}
        </button>
        <button type="button" className="composer-cancel" onClick={cancel}>
          Cancel
        </button>
        <span className="composer-hint">⌘↩ to submit</span>
      </div>
    </div>
  );
}

interface CommentCardProps {
  comment: Comment;
  editing: boolean;
  drafts: DraftStore;
  onEditStart: (id: number) => void;
  onEditCancel: () => void;
  /** Resolve to close the editor; reject to keep it open with the error. */
  onSave: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
}

export function CommentCard({
  comment,
  editing,
  drafts,
  onEditStart,
  onEditCancel,
  onSave,
  onDelete,
}: CommentCardProps) {
  // Two-step delete instead of confirm(): native dialogs would block the
  // webview (and any automation driving it).
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const remove = () => {
    setBusy(true);
    setError(null);
    onDelete(comment.id).catch((e: unknown) => {
      setError(typeof e === "string" ? e : String(e));
      setBusy(false);
      setConfirmingDelete(false);
    });
  };

  const location = describeLocation(comment);
  return (
    <article className="comment-card" aria-label={`Comment C${comment.id}`}>
      <header className="comment-header">
        <span className="comment-id">C{comment.id}</span>
        {location !== null && (
          <span className="comment-location">{location}</span>
        )}
        <span className="comment-time">
          {formatTime(comment.createdAt)}
          {comment.updatedAt !== comment.createdAt && " (edited)"}
        </span>
        <span className="comment-header-spacer" />
        {!editing && !confirmingDelete && (
          <>
            <button
              type="button"
              className="comment-action"
              onClick={() => {
                drafts.set(editDraftKey(comment.id), comment.body);
                onEditStart(comment.id);
              }}
            >
              Edit
            </button>
            <button
              type="button"
              className="comment-action"
              onClick={() => setConfirmingDelete(true)}
            >
              Delete
            </button>
          </>
        )}
        {confirmingDelete && (
          <>
            <button
              type="button"
              className="comment-action comment-action-danger"
              disabled={busy}
              onClick={remove}
            >
              {busy ? "Deleting…" : "Really delete?"}
            </button>
            <button
              type="button"
              className="comment-action"
              disabled={busy}
              onClick={() => setConfirmingDelete(false)}
            >
              Keep
            </button>
          </>
        )}
      </header>
      {error !== null && <p className="composer-error">{error}</p>}
      {editing ? (
        <CommentComposer
          drafts={drafts}
          draftKey={editDraftKey(comment.id)}
          placeholder="Comment text"
          submitLabel="Save"
          onSubmit={(body) => onSave(comment.id, body)}
          onCancel={onEditCancel}
        />
      ) : (
        <p className="comment-body">{comment.body}</p>
      )}
    </article>
  );
}

function describeLocation(comment: Comment): string | null {
  if (comment.level !== "line" || comment.startLine === null) {
    return null;
  }
  const range =
    comment.startLine === comment.endLine
      ? `Line ${comment.startLine}`
      : `Lines ${comment.startLine}–${comment.endLine}`;
  return comment.side === "old" ? `${range} (old)` : range;
}

function formatTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
}
