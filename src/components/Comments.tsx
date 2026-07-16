import { useState } from "react";
import type { Comment, CommentState } from "../types";

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

const STATE_BADGES: Record<Exclude<CommentState, "open">, string> = {
  resolved: "Resolved",
  dismissed: "Dismissed",
};

interface CommentCardProps {
  comment: Comment;
  editing: boolean;
  drafts: DraftStore;
  /** The commented code changed since the comment was made (re-review cue). */
  codeChanged?: boolean;
  /** Hide every mutating control (archived reviews are read-only). */
  readOnly?: boolean;
  onEditStart: (id: number) => void;
  onEditCancel: () => void;
  /** Resolve to close the editor; reject to keep it open with the error. */
  onSave: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  /** Lifecycle transitions: resolve / dismiss / reopen. */
  onSetState?: (id: number, state: CommentState) => Promise<void>;
}

export function CommentCard({
  comment,
  editing,
  drafts,
  codeChanged = false,
  readOnly = false,
  onEditStart,
  onEditCancel,
  onSave,
  onDelete,
  onSetState,
}: CommentCardProps) {
  // Two-step delete instead of confirm(): native dialogs would block the
  // webview (and any automation driving it).
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Resolved/dismissed comments collapse their body until toggled.
  const [showClosedBody, setShowClosedBody] = useState(false);

  const closed = comment.state !== "open";

  const fail = (e: unknown) => {
    setError(typeof e === "string" ? e : String(e));
    setBusy(false);
    setConfirmingDelete(false);
  };

  const remove = () => {
    setBusy(true);
    setError(null);
    onDelete(comment.id).catch(fail);
  };

  const setState = (state: CommentState) => {
    if (onSetState === undefined) {
      return;
    }
    setBusy(true);
    setError(null);
    onSetState(comment.id, state)
      .then(() => setBusy(false))
      .catch(fail);
  };

  const location = describeLocation(comment);
  return (
    <article
      className={`comment-card${closed ? " comment-card-closed" : ""}`}
      aria-label={`Comment C${comment.id}`}
    >
      <header className="comment-header">
        <span className="comment-id">C{comment.id}</span>
        {closed && (
          <span className={`comment-state comment-state-${comment.state}`}>
            {STATE_BADGES[comment.state as Exclude<CommentState, "open">]}
          </span>
        )}
        {location !== null && (
          <span className="comment-location">{location}</span>
        )}
        {codeChanged && (
          <span
            className="comment-flag"
            title="The commented code changed since this comment was made — re-review it."
          >
            ⚠ code changed since commented
          </span>
        )}
        <span className="comment-time">
          {formatTime(comment.createdAt)}
          {comment.updatedAt !== comment.createdAt && " (edited)"}
        </span>
        <span className="comment-header-spacer" />
        {!readOnly && !editing && !confirmingDelete && (
          <>
            {closed ? (
              <button
                type="button"
                className="comment-action"
                disabled={busy}
                onClick={() => setState("open")}
              >
                Reopen
              </button>
            ) : (
              <>
                {onSetState !== undefined && (
                  <>
                    <button
                      type="button"
                      className="comment-action"
                      title="Handled — a change was made or the new code is fine"
                      disabled={busy}
                      onClick={() => setState("resolved")}
                    >
                      Resolve
                    </button>
                    <button
                      type="button"
                      className="comment-action"
                      title="Won't fix — decided it doesn't matter"
                      disabled={busy}
                      onClick={() => setState("dismissed")}
                    >
                      Dismiss
                    </button>
                  </>
                )}
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
      ) : closed ? (
        <button
          type="button"
          className={`comment-body-toggle${showClosedBody ? "" : " comment-body-collapsed"}`}
          title={showClosedBody ? "Collapse" : "Show full comment"}
          onClick={() => setShowClosedBody((v) => !v)}
        >
          {comment.body}
        </button>
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
