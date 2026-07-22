import { useCallback, useRef } from "react";
import {
  createComment,
  deleteComment,
  markFileReviewed,
  unmarkFileReviewed,
  updateComment,
  updateCommentState,
} from "../ipc";
import type { CommentState, NewCommentInput } from "../types";
import type { ReviewSession } from "./useReviewSession";

/**
 * The review's mutation callbacks: comment CRUD, lifecycle transitions, and
 * the reviewed-mark toggle. Every callback reads the session through refs
 * and stays stable across re-renders, so the memoized diff rows never
 * re-render for a handler identity change.
 */
export function useCommentMutations(session: ReviewSession) {
  const { current, setComments, setReviewedFiles } = session;

  // Ref mirror so the stable toggle callback reads the latest map without
  // depending on it (state updaters run at render time, too late to pick
  // the IPC call from inside one).
  const reviewedRef = useRef(session.reviewedFiles);
  reviewedRef.current = session.reviewedFiles;

  /** Toggle a file's reviewed mark, optimistically: marking stores the
   * fingerprint the user is looking at; unmarking deletes the row. */
  const onToggleReviewed = useCallback(
    (path: string) => {
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
    },
    [current, setReviewedFiles],
  );

  const onCreate = useCallback(
    async (input: NewCommentInput) => {
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
    },
    [current, setComments],
  );

  const onUpdate = useCallback(
    async (id: number, body: string) => {
      const updated = await updateComment(id, body);
      setComments((prev) => prev.map((c) => (c.id === id ? updated : c)));
    },
    [setComments],
  );

  const onDelete = useCallback(
    async (id: number) => {
      await deleteComment(id);
      // Deleting a thread root cascades its replies server-side; drop them
      // from local state the same way.
      setComments((prev) =>
        prev.filter((c) => c.id !== id && c.parentId !== id),
      );
    },
    [setComments],
  );

  const onCreateReply = useCallback(
    (rootId: number, body: string) =>
      onCreate({ level: "review", parentId: rootId, body }),
    [onCreate],
  );

  const onSetState = useCallback(
    async (id: number, state: CommentState) => {
      const updated = await updateCommentState(id, state);
      setComments((prev) => prev.map((c) => (c.id === id ? updated : c)));
    },
    [setComments],
  );

  const onCreateReviewComment = useCallback(
    (body: string) => onCreate({ level: "review", body }),
    [onCreate],
  );

  return {
    onToggleReviewed,
    onCreate,
    onUpdate,
    onDelete,
    onCreateReply,
    onSetState,
    onCreateReviewComment,
  };
}
