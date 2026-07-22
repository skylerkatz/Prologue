// The Rust-owned IPC shapes live in ./generated/ipc-types.ts, regenerated
// from prologue-core by `cargo test` in src-tauri (which fails while the
// file is stale). This module re-exports them and adds the frontend-only
// helpers built on top.

export type {
  AnchorStatus,
  ArchivedReview,
  BranchList,
  CodeAnchor,
  Comment,
  CommentLevel,
  CommentSide,
  CommentState,
  ContextLines,
  DiffLine,
  DiffMode,
  DiffSummary,
  ExportFormat,
  FileDiff,
  FileStatus,
  FileSummary,
  Hunk,
  IntralineRange,
  LineKind,
  NewComment,
  OpenReviewResult,
  ReanchorResult,
  RepoInfo,
  Review,
  ReviewedFile,
  ReviewStatus,
} from "./generated/ipc-types";

import type { Comment, DiffMode, NewComment } from "./generated/ipc-types";

/** Derived client-side per summary file: stored fingerprint matches the
 * current one → "reviewed"; a stored mark whose fingerprint differs →
 * "changed" (changed since review); no mark → absent from the map. */
export type FileReviewState = "reviewed" | "changed";

/** Caller-provided fields of a new comment; the review id is appended at
 * the IPC call site, and the app never names an author (Rust defaults it
 * to "reviewer"). */
export type NewCommentInput = Omit<NewComment, "reviewId" | "author">;

/** Replies grouped under their thread root, in chronological (id) order. */
export type RepliesByRoot = ReadonlyMap<number, Comment[]>;

export function groupReplies(comments: Comment[]): RepliesByRoot {
  const map = new Map<number, Comment[]>();
  for (const comment of comments) {
    if (comment.parentId === null) {
      continue;
    }
    const bucket = map.get(comment.parentId);
    if (bucket === undefined) {
      map.set(comment.parentId, [comment]);
    } else {
      bucket.push(comment);
    }
  }
  return map;
}

export const WORKING_TREE_MODES: ReadonlyArray<{
  value: DiffMode;
  label: string;
}> = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
];
