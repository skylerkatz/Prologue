import { invoke } from "@tauri-apps/api/core";
import type {
  ArchivedReview,
  BranchList,
  Comment,
  CommentState,
  ContextLines,
  DiffSummary,
  ExportFormat,
  FileDiff,
  NewCommentInput,
  OpenReviewResult,
  ReanchorResult,
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
  ignoreWhitespace: boolean,
): Promise<DiffSummary> {
  return invoke("get_diff_summary", {
    repoPath,
    base,
    head,
    mode,
    ignoreWhitespace,
  });
}

export function getFileDiff(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  ignoreWhitespace: boolean,
  path: string,
): Promise<FileDiff> {
  return invoke("get_file_diff", {
    repoPath,
    base,
    head,
    mode,
    ignoreWhitespace,
    path,
  });
}

/**
 * Resume (or create) the active review for this repo + branch. A merged
 * branch gets its archived review back read-only instead of a new one.
 */
export function openReview(
  repoPath: string,
  branch: string,
  baseRef: string,
  mode: WorkingTreeMode,
): Promise<OpenReviewResult> {
  return invoke("open_review", { repoPath, branch, baseRef, mode });
}

/**
 * The active review for (repo, branch), if any — read-only. Used on repo
 * open to restore the base ref the review was last computed against.
 */
export function findActiveReview(
  repoPath: string,
  branch: string,
): Promise<Review | null> {
  return invoke("find_active_review", { repoPath, branch });
}

export function updateCommentState(
  commentId: number,
  state: CommentState,
): Promise<Comment> {
  return invoke("update_comment_state", { commentId, state });
}

/**
 * Re-locate the review's line comments in the current diff via their code
 * anchors; moved line ranges are persisted server-side.
 */
export function reanchorComments(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  reviewId: number,
): Promise<ReanchorResult[]> {
  return invoke("reanchor_comments", { repoPath, base, head, mode, reviewId });
}

/** Archive reviews whose branch is merged into its base or deleted. */
export function archiveStaleReviews(repoPath: string): Promise<Review[]> {
  return invoke("archive_stale_reviews", { repoPath });
}

export function listArchivedReviews(
  repoPath: string,
): Promise<ArchivedReview[]> {
  return invoke("list_archived_reviews", { repoPath });
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

/**
 * Render the review's open comments as clipboard-ready text; SHAs, line
 * ranges, and orphan status are resolved against the current diff in Rust.
 */
export function exportReview(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  reviewId: number,
  format: ExportFormat,
): Promise<string> {
  return invoke("export_review", {
    repoPath,
    base,
    head,
    mode,
    reviewId,
    format,
  });
}

/**
 * Watch the repo (working tree + .git) and emit a debounced `repo-changed`
 * event on activity. One watch at a time; a new call re-targets it.
 */
export function startWatching(repoPath: string): Promise<void> {
  return invoke("start_watching", { repoPath });
}

/** Drop the active repo watch (repo closed). */
export function stopWatching(): Promise<void> {
  return invoke("stop_watching");
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
