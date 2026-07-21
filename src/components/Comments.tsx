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
  /** How many replies this thread root has; deleting it cascades them. */
  replyCount?: number;
  onEditStart: (id: number) => void;
  onEditCancel: () => void;
  /** Resolve to close the editor; reject to keep it open with the error. */
  onSave: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  /** Lifecycle transitions: resolve / dismiss / reopen. */
  onSetState?: (id: number, state: CommentState) => Promise<void>;
  /** Open a reply composer for this thread (open roots only). */
  onReply?: () => void;
}

export function CommentCard({
  comment,
  editing,
  drafts,
  codeChanged = false,
  readOnly = false,
  replyCount = 0,
  onEditStart,
  onEditCancel,
  onSave,
  onDelete,
  onSetState,
  onReply,
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

  const isReply = comment.parentId !== null;
  const location = describeLocation(comment);
  return (
    <article
      className={`comment-card${closed ? " comment-card-closed" : ""}${isReply ? " comment-card-reply" : ""}`}
      aria-label={isReply ? `Reply C${comment.id}` : `Comment C${comment.id}`}
    >
      <header className="comment-header">
        {/* Teal avatar circle carrying the mono comment ID. */}
        <span className="comment-id">C{comment.id}</span>
        {comment.author === "reviewer" ? (
          <span className="comment-you">You</span>
        ) : (
          <span
            className="comment-author"
            title={`Written by ${comment.author} (externally, via the prologue CLI)`}
          >
            {comment.author}
          </span>
        )}
        {isReply && <span className="comment-reply-tag">(reply)</span>}
        {location !== null && (
          <span className="comment-location">{location}</span>
        )}
        <span className="comment-time">
          {formatTime(comment.createdAt)}
          {comment.updatedAt !== comment.createdAt && " (edited)"}
        </span>
        {closed && (
          <span className={`comment-state comment-state-${comment.state}`}>
            {STATE_BADGES[comment.state as Exclude<CommentState, "open">]}
          </span>
        )}
        {codeChanged && (
          <span
            className="comment-flag"
            title="The commented code changed since this comment was made — re-review it."
          >
            ⚠ code changed since commented
          </span>
        )}
        {closed && replyCount > 0 && (
          <span
            className="comment-hidden-replies"
            title="The whole thread is collapsed — reopen to see its replies."
          >
            {replyCount} {replyCount === 1 ? "reply" : "replies"} hidden
          </span>
        )}
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
                {onReply !== undefined && (
                  <button
                    type="button"
                    className="comment-action"
                    title="Reply to this thread"
                    onClick={onReply}
                  >
                    ↳ Reply
                  </button>
                )}
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
              {busy
                ? "Deleting…"
                : replyCount > 0
                  ? `Really delete? Its ${
                      replyCount === 1 ? "reply" : `${replyCount} replies`
                    } will be deleted too`
                  : "Really delete?"}
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

export const replyDraftKey = (rootId: number): string => `reply:${rootId}`;

interface CommentThreadProps {
  root: Comment;
  /** The thread's replies in chronological order. */
  replies: Comment[];
  editingId: number | null;
  drafts: DraftStore;
  codeChanged?: boolean;
  readOnly?: boolean;
  /** Root id whose reply composer is open; state lives with the caller. */
  replyingTo?: number | null;
  onReplyStart?: (rootId: number) => void;
  onReplyCancel?: () => void;
  /** Resolve to close the reply composer; reject to keep it open. */
  onCreateReply?: (rootId: number, body: string) => Promise<void>;
  onEditStart: (id: number) => void;
  onEditCancel: () => void;
  onSave: (id: number, body: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onSetState?: (id: number, state: CommentState) => Promise<void>;
}

/**
 * A root comment with its flat reply thread, for the non-virtualized panels
 * (review comments, orphaned bucket, archive browser). The diff view builds
 * the same structure out of virtualized rows instead.
 *
 * Lifecycle is thread-level: resolving/dismissing the root collapses the
 * whole thread (replies included). The read-only archive browser always
 * shows full history.
 */
export function CommentThread({
  root,
  replies,
  editingId,
  drafts,
  codeChanged = false,
  readOnly = false,
  replyingTo = null,
  onReplyStart,
  onReplyCancel,
  onCreateReply,
  onEditStart,
  onEditCancel,
  onSave,
  onDelete,
  onSetState,
}: CommentThreadProps) {
  const open = root.state === "open";
  const showReplies = open || readOnly;
  return (
    <div className="comment-thread">
      <CommentCard
        comment={root}
        editing={editingId === root.id}
        drafts={drafts}
        codeChanged={codeChanged}
        readOnly={readOnly}
        replyCount={replies.length}
        onEditStart={onEditStart}
        onEditCancel={onEditCancel}
        onSave={onSave}
        onDelete={onDelete}
        onSetState={onSetState}
        onReply={
          !readOnly && open && onReplyStart !== undefined
            ? () => onReplyStart(root.id)
            : undefined
        }
      />
      {showReplies &&
        replies.map((reply) => (
          <CommentCard
            key={reply.id}
            comment={reply}
            editing={editingId === reply.id}
            drafts={drafts}
            readOnly={readOnly}
            onEditStart={onEditStart}
            onEditCancel={onEditCancel}
            onSave={onSave}
            onDelete={onDelete}
          />
        ))}
      {!readOnly && replyingTo === root.id && onCreateReply !== undefined && (
        <div className="reply-composer">
          <CommentComposer
            drafts={drafts}
            draftKey={replyDraftKey(root.id)}
            placeholder={`Reply to C${root.id}…`}
            submitLabel="Reply"
            onSubmit={(body) => onCreateReply(root.id, body)}
            onCancel={() => onReplyCancel?.()}
          />
        </div>
      )}
    </div>
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
