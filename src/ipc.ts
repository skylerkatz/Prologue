import { invoke } from "@tauri-apps/api/core";
import type {
  BranchList,
  Comment,
  ContextLines,
  DiffSummary,
  FileDiff,
  NewCommentInput,
  RepoInfo,
  Review,
  WorkingTreeMode,
} from "./types";

// Typed wrappers around the Tauri commands.

export function openRepo(path: string): Promise<RepoInfo> {
  return invoke("open_repo", { path });
}

export function listBranches(repoPath: string): Promise<BranchList> {
  return invoke("list_branches", { repoPath });
}

export function getDiffSummary(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
): Promise<DiffSummary> {
  return invoke("get_diff_summary", { repoPath, base, head, mode });
}

export function getFileDiff(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  path: string,
): Promise<FileDiff> {
  return invoke("get_file_diff", { repoPath, base, head, mode, path });
}

/** Resume (or create) the active review for this repo + branch. */
export function openReview(
  repoPath: string,
  branch: string,
  baseRef: string,
  mode: WorkingTreeMode,
): Promise<Review> {
  return invoke("open_review", { repoPath, branch, baseRef, mode });
}

export function listComments(reviewId: number): Promise<Comment[]> {
  return invoke("list_comments", { reviewId });
}

/**
 * Create a comment; base/head/mode let Rust capture the code anchor and
 * commit SHA for line comments.
 */
export function createComment(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  comment: NewCommentInput & { reviewId: number },
): Promise<Comment> {
  return invoke("create_comment", { repoPath, base, head, mode, comment });
}

export function updateComment(
  commentId: number,
  body: string,
): Promise<Comment> {
  return invoke("update_comment", { commentId, body });
}

export function deleteComment(commentId: number): Promise<void> {
  return invoke("delete_comment", { commentId });
}

/** New-side lines `start..=end` (1-based, clamped) for expand-context. */
export function getContextLines(
  repoPath: string,
  head: string,
  mode: WorkingTreeMode,
  path: string,
  start: number,
  end: number,
): Promise<ContextLines> {
  return invoke("get_context_lines", { repoPath, head, mode, path, start, end });
}
