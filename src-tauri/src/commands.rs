//! Thin `#[tauri::command]` wrappers over `prologue-core`: lock the managed
//! database state (where needed) and delegate. No logic lives here.

use prologue_core::db::Db;
use prologue_core::diff::{self, ContextLines, DiffMode, DiffSummary, FileDiff};
use prologue_core::export::{self, ExportFormat};
use prologue_core::repo::{self, open_git_repo, BranchList, RepoInfo};
use prologue_core::review::{
    self, ArchivedReview, Comment, CommentState, NewComment, OpenReviewResult, ReanchorResult,
    Review,
};
use prologue_core::rusqlite::Connection;

fn lock<'a>(db: &'a tauri::State<'_, Db>) -> Result<std::sync::MutexGuard<'a, Connection>, String> {
    db.0.lock().map_err(|_| "Review database is unavailable".to_owned())
}

/// Validate that `path` points to a local git repository and return its identity.
#[tauri::command]
pub fn open_repo(path: String) -> Result<RepoInfo, String> {
    repo::open_repo(path)
}

/// List local and remote-tracking branches, the checked-out branch, and the
/// auto-detected default base ref.
#[tauri::command]
pub fn list_branches(repo_path: String) -> Result<BranchList, String> {
    repo::list_branches(repo_path)
}

/// Merge-base (three-dot) file summary: diff(merge-base(base, head), head),
/// where "head" is the branch tip, index, or working tree depending on `mode`.
#[tauri::command]
pub fn get_diff_summary(
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    ignore_whitespace: bool,
) -> Result<DiffSummary, String> {
    diff::get_diff_summary(repo_path, base, head, mode, ignore_whitespace)
}

/// Hunks for a single file from the same diff `get_diff_summary` computes;
/// fetched on demand so only the summary crosses IPC up front.
#[tauri::command]
pub fn get_file_diff(
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    ignore_whitespace: bool,
    path: String,
) -> Result<FileDiff, String> {
    diff::get_file_diff(repo_path, base, head, mode, ignore_whitespace, path)
}

/// Lines `start..=end` (1-based, clamped) of the file's new side — head tree,
/// index, or working tree depending on `mode`.
#[tauri::command]
pub fn get_context_lines(
    repo_path: String,
    head: String,
    mode: DiffMode,
    path: String,
    start: u32,
    end: u32,
) -> Result<ContextLines, String> {
    diff::get_context_lines(repo_path, head, mode, path, start, end)
}

/// Resume the active review for (repo, branch), creating one if none exists.
/// The stored base ref and mode follow the caller's current choice. A branch
/// already merged into the base gets no new active review — its existing
/// active review is archived and the latest archived one is returned
/// read-only instead.
#[tauri::command]
pub fn open_review(
    db: tauri::State<'_, Db>,
    repo_path: String,
    branch: String,
    base_ref: String,
    mode: DiffMode,
) -> Result<OpenReviewResult, String> {
    let repo = open_git_repo(&repo_path)?;
    let conn = lock(&db)?;
    review::open_review_checked_impl(&conn, &repo, &repo_path, &branch, &base_ref, mode)
}

/// The active review for (repo, branch), if any — read-only. Lets the app
/// restore the review's stored base ref when the repo is reopened instead of
/// falling back to the auto-detected default base.
#[tauri::command]
pub fn find_active_review(
    db: tauri::State<'_, Db>,
    repo_path: String,
    branch: String,
) -> Result<Option<Review>, String> {
    let conn = lock(&db)?;
    review::find_active_review_impl(&conn, &repo_path, &branch)
}

#[tauri::command]
pub fn list_comments(db: tauri::State<'_, Db>, review_id: i64) -> Result<Vec<Comment>, String> {
    let conn = lock(&db)?;
    review::list_comments_impl(&conn, review_id)
}

#[tauri::command]
pub fn create_comment(
    db: tauri::State<'_, Db>,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    comment: NewComment,
) -> Result<Comment, String> {
    let conn = lock(&db)?;
    review::create_comment_impl(&conn, &repo_path, &base, &head, mode, comment)
}

#[tauri::command]
pub fn update_comment(
    db: tauri::State<'_, Db>,
    comment_id: i64,
    body: String,
) -> Result<Comment, String> {
    let conn = lock(&db)?;
    review::update_comment_impl(&conn, comment_id, &body)
}

#[tauri::command]
pub fn delete_comment(db: tauri::State<'_, Db>, comment_id: i64) -> Result<(), String> {
    let conn = lock(&db)?;
    review::delete_comment_impl(&conn, comment_id)
}

/// Set a comment's lifecycle state (open | resolved | dismissed). Resolved
/// and dismissed comments stay in history; `updated_at` is untouched so
/// "(edited)" keeps meaning body edits.
#[tauri::command]
pub fn update_comment_state(
    db: tauri::State<'_, Db>,
    comment_id: i64,
    state: CommentState,
) -> Result<Comment, String> {
    let conn = lock(&db)?;
    review::update_comment_state_impl(&conn, comment_id, state)
}

/// Re-locate every line comment of `review_id` in the current diff via its
/// code anchor, persisting moved line ranges. Returns one status per line
/// comment; review- and file-level comments have no anchor and are skipped.
#[tauri::command]
pub fn reanchor_comments(
    db: tauri::State<'_, Db>,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    review_id: i64,
) -> Result<Vec<ReanchorResult>, String> {
    let conn = lock(&db)?;
    review::reanchor_comments_impl(&conn, &repo_path, &base, &head, mode, review_id, true)
}

/// Archive every active review of this repo whose branch was merged into
/// its base or deleted. Returns the reviews archived by this call.
#[tauri::command]
pub fn archive_stale_reviews(
    db: tauri::State<'_, Db>,
    repo_path: String,
) -> Result<Vec<Review>, String> {
    let repo = open_git_repo(&repo_path)?;
    let conn = lock(&db)?;
    review::archive_stale_reviews_impl(&conn, &repo, &repo_path)
}

/// Archived reviews of this repo, newest first, for the read-only browser.
#[tauri::command]
pub fn list_archived_reviews(
    db: tauri::State<'_, Db>,
    repo_path: String,
) -> Result<Vec<ArchivedReview>, String> {
    let conn = lock(&db)?;
    review::list_archived_reviews_impl(&conn, &repo_path)
}

/// Render the review's open comments as clipboard-ready text. Line ranges,
/// orphan status, and the header SHAs are all resolved against the diff as
/// it stands right now — the same computation the UI displays.
#[tauri::command]
pub fn export_review(
    db: tauri::State<'_, Db>,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    review_id: i64,
    format: ExportFormat,
) -> Result<String, String> {
    let conn = lock(&db)?;
    export::export_review_impl(&conn, &repo_path, &base, &head, mode, review_id, format, true)
}
