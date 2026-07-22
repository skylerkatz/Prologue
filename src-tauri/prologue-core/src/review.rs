use git2::{BranchType, Repository};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::anchor::{self, AnchorStatus};
use crate::db::NOW;
use crate::diff::{self, DiffLine, DiffMode, DiffSpec, FileDiff};
use crate::error::CoreError;
use crate::repo::open_git_repo;

/// How many unchanged same-side lines the code anchor keeps on each side of
/// the selection.
const ANCHOR_CONTEXT: usize = 3;

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub id: i64,
    pub repo_path: String,
    pub branch: String,
    pub base_ref: String,
    pub mode: DiffMode,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// The diff coordinates a review is stored at: its branch diffed against its
/// base ref, in its working-tree mode.
impl From<&Review> for DiffSpec {
    fn from(review: &Review) -> Self {
        DiffSpec {
            repo_path: review.repo_path.clone(),
            base: review.base_ref.clone(),
            head: review.branch.clone(),
            mode: review.mode,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommentLevel {
    Review,
    File,
    Line,
}

impl CommentLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CommentLevel::Review => "review",
            CommentLevel::File => "file",
            CommentLevel::Line => "line",
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "review" => Ok(CommentLevel::Review),
            "file" => Ok(CommentLevel::File),
            "line" => Ok(CommentLevel::Line),
            other => Err(format!("Unknown comment level: {other}")),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommentSide {
    Old,
    New,
}

impl CommentSide {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CommentSide::Old => "old",
            CommentSide::New => "new",
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "old" => Ok(CommentSide::Old),
            "new" => Ok(CommentSide::New),
            other => Err(format!("Unknown comment side: {other}")),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommentState {
    Open,
    Resolved,
    Dismissed,
}

impl CommentState {
    fn as_str(self) -> &'static str {
        match self {
            CommentState::Open => "open",
            CommentState::Resolved => "resolved",
            CommentState::Dismissed => "dismissed",
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "open" => Ok(CommentState::Open),
            "resolved" => Ok(CommentState::Resolved),
            "dismissed" => Ok(CommentState::Dismissed),
            other => Err(format!("Unknown comment state: {other}")),
        }
    }
}

/// Enough verbatim code to re-locate a line comment after edits: the selected
/// lines, up to [`ANCHOR_CONTEXT`] same-side lines around them, and the hunk
/// header. Stored as JSON in the `code_anchor` column.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnchor {
    pub hunk_header: String,
    pub context_before: Vec<String>,
    pub lines: Vec<String>,
    pub context_after: Vec<String>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: i64,
    pub review_id: i64,
    pub level: CommentLevel,
    pub file_path: Option<String>,
    pub side: Option<CommentSide>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub code_anchor: Option<CodeAnchor>,
    pub commit_sha: String,
    pub state: CommentState,
    pub body: String,
    /// Thread root this comment replies to; None for roots. Threads are one
    /// level deep — a reply's parent is always a root. Replies inherit the
    /// root's file/side/lines/anchor context (their own stay NULL), and
    /// their lifecycle is the root's (`state` is meaningless on replies).
    pub parent_id: Option<i64>,
    /// Who wrote it: 'reviewer' for the app's own writes, anything else for
    /// external writers (e.g. 'agent' via the prologue CLI). The UI badges
    /// non-reviewer authors.
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Frontend payload for `create_comment`; anchor and commit SHA are captured
/// here in Rust, not trusted from the caller.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewComment {
    pub review_id: i64,
    pub level: CommentLevel,
    pub file_path: Option<String>,
    pub side: Option<CommentSide>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    /// Set to any comment in a thread to reply; the reply attaches to the
    /// thread ROOT (replying to a reply joins the same flat thread). All
    /// positional fields above are ignored for replies — a reply inherits
    /// its context from the root.
    #[serde(default)]
    pub parent_id: Option<i64>,
    pub body: String,
    /// Who is writing. The app's IPC payloads never set it (None →
    /// 'reviewer'); external writers name themselves, e.g. 'agent'.
    #[serde(default)]
    pub author: Option<String>,
}

/// One per-file reviewed mark. `fingerprint` is the [`FileSummary`] content
/// identity at mark time; the frontend compares it with the current summary's
/// value — equal means "reviewed", different means "changed since review".
///
/// [`FileSummary`]: crate::diff::FileSummary
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReviewedFile {
    pub file_path: String,
    pub fingerprint: String,
    pub reviewed_at: String,
}

/// What `open_review` produced: the branch's review (absent when the branch
/// is merged and was never reviewed), plus whether the branch is already
/// merged into the base — in which case `review`, if present, is archived
/// and read-only.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenReviewResult {
    pub review: Option<Review>,
    pub branch_merged: bool,
}

fn db_err(e: rusqlite::Error) -> String {
    format!("Review database error: {e}")
}

pub fn open_review_impl(
    conn: &Connection,
    repo_path: &str,
    branch: &str,
    base_ref: &str,
    mode: DiffMode,
) -> Result<Review, String> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path = ?1 AND branch = ?2 AND status = 'active'",
            [repo_path, branch],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;

    let id = match existing {
        Some(id) => {
            conn.execute(
                &format!(
                    "UPDATE reviews SET base_ref = ?1, mode = ?2, updated_at = {NOW}
                     WHERE id = ?3"
                ),
                (base_ref, mode.as_str(), id),
            )
            .map_err(db_err)?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO reviews (repo_path, branch, base_ref, mode)
                 VALUES (?1, ?2, ?3, ?4)",
                (repo_path, branch, base_ref, mode.as_str()),
            )
            .map_err(db_err)?;
            conn.last_insert_rowid()
        }
    };
    get_review(conn, id)
}

fn get_review(conn: &Connection, id: i64) -> Result<Review, String> {
    conn.query_row(
        "SELECT id, repo_path, branch, base_ref, mode, status, created_at, updated_at
         FROM reviews WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        },
    )
    .optional()
    .map_err(db_err)?
    .ok_or_else(|| format!("Review not found: {id}"))
    .and_then(|(id, repo_path, branch, base_ref, mode, status, created_at, updated_at)| {
        Ok(Review {
            id,
            repo_path,
            branch,
            base_ref,
            mode: DiffMode::parse(&mode)?,
            status,
            created_at,
            updated_at,
        })
    })
}

/// Why a branch's review should be auto-archived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StaleReason {
    Merged,
    Deleted,
}

/// Whether `branch`'s review should auto-archive: the branch no longer
/// exists (checked as local, then remote-tracking — reviews can target
/// either), or its tip is a *strict* ancestor of the base. Equal tips do not
/// count as merged, so a fresh branch with no commits keeps its review; a
/// fast-forward merge is therefore only detected once the base moves on (or
/// the branch is deleted). An unresolvable base means merged-ness cannot be
/// determined — not stale.
fn stale_reason(repo: &Repository, branch: &str, base_ref: &str) -> Option<StaleReason> {
    let exists = repo.find_branch(branch, BranchType::Local).is_ok()
        || repo.find_branch(branch, BranchType::Remote).is_ok();
    if !exists {
        return Some(StaleReason::Deleted);
    }
    let (Ok(branch_tip), Ok(base_tip)) = (
        diff::resolve_commit(repo, branch),
        diff::resolve_commit(repo, base_ref),
    ) else {
        return None;
    };
    let merged = branch_tip.id() != base_tip.id()
        && repo
            .graph_descendant_of(base_tip.id(), branch_tip.id())
            .unwrap_or(false);
    merged.then_some(StaleReason::Merged)
}

fn archive_review(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute(
        &format!("UPDATE reviews SET status = 'archived', updated_at = {NOW} WHERE id = ?1"),
        [id],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn open_review_checked_impl(
    conn: &Connection,
    repo: &Repository,
    repo_path: &str,
    branch: &str,
    base_ref: &str,
    mode: DiffMode,
) -> Result<OpenReviewResult, String> {
    if stale_reason(repo, branch, base_ref).is_none() {
        let review = open_review_impl(conn, repo_path, branch, base_ref, mode)?;
        return Ok(OpenReviewResult {
            review: Some(review),
            branch_merged: false,
        });
    }
    // Merged (or somehow deleted) branch: close out any active review and
    // surface the newest archived one read-only instead of creating a fresh
    // active review that the next refresh would archive again.
    let active: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path = ?1 AND branch = ?2 AND status = 'active'",
            [repo_path, branch],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if let Some(id) = active {
        archive_review(conn, id)?;
    }
    let latest_archived: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path = ?1 AND branch = ?2 AND status = 'archived'
             ORDER BY id DESC LIMIT 1",
            [repo_path, branch],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(OpenReviewResult {
        review: latest_archived.map(|id| get_review(conn, id)).transpose()?,
        branch_merged: true,
    })
}

pub fn archive_stale_reviews_impl(
    conn: &Connection,
    repo: &Repository,
    repo_path: &str,
) -> Result<Vec<Review>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, branch, base_ref FROM reviews
             WHERE repo_path = ?1 AND status = 'active' ORDER BY id",
        )
        .map_err(db_err)?;
    let active: Vec<(i64, String, String)> = stmt
        .query_map([repo_path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    let mut archived = Vec::new();
    for (id, branch, base_ref) in active {
        if stale_reason(repo, &branch, &base_ref).is_some() {
            archive_review(conn, id)?;
            archived.push(get_review(conn, id)?);
        }
    }
    Ok(archived)
}

/// An archived review plus its comment count, for the read-only browser.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedReview {
    #[serde(flatten)]
    pub review: Review,
    pub comment_count: i64,
}

pub fn list_archived_reviews_impl(
    conn: &Connection,
    repo_path: &str,
) -> Result<Vec<ArchivedReview>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, (SELECT COUNT(*) FROM comments c WHERE c.review_id = reviews.id)
             FROM reviews WHERE repo_path = ?1 AND status = 'archived'
             ORDER BY updated_at DESC, id DESC",
        )
        .map_err(db_err)?;
    let rows: Vec<(i64, i64)> = stmt
        .query_map([repo_path], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter()
        .map(|(id, comment_count)| {
            Ok(ArchivedReview {
                review: get_review(conn, id)?,
                comment_count,
            })
        })
        .collect()
}

/// All reviews, active first (then most recently updated), optionally
/// filtered to one repo and optionally including archived ones. Read-only.
pub fn list_reviews_impl(
    conn: &Connection,
    repo_path: Option<&str>,
    include_archived: bool,
) -> Result<Vec<Review>, String> {
    let mut sql = String::from("SELECT id FROM reviews WHERE 1=1");
    if repo_path.is_some() {
        sql.push_str(" AND repo_path = ?1");
    }
    if !include_archived {
        sql.push_str(" AND status = 'active'");
    }
    sql.push_str(
        " ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END, updated_at DESC, id DESC",
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let ids: Vec<i64> = stmt
        .query_map(rusqlite::params_from_iter(repo_path), |r| r.get(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    ids.into_iter().map(|id| get_review(conn, id)).collect()
}

/// The active review for (repo, branch), if any. Read-only — unlike
/// `open_review_impl` this never creates or touches a row.
pub fn find_active_review_impl(
    conn: &Connection,
    repo_path: &str,
    branch: &str,
) -> Result<Option<Review>, String> {
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path = ?1 AND branch = ?2 AND status = 'active'",
            [repo_path, branch],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    id.map(|id| get_review(conn, id)).transpose()
}

pub fn list_reviewed_files_impl(
    conn: &Connection,
    review_id: i64,
) -> Result<Vec<ReviewedFile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT file_path, fingerprint, reviewed_at
             FROM reviewed_files WHERE review_id = ?1 ORDER BY file_path",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([review_id], |r| {
            Ok(ReviewedFile {
                file_path: r.get(0)?,
                fingerprint: r.get(1)?,
                reviewed_at: r.get(2)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Upsert: re-marking a file (e.g. one flagged "changed since review")
/// replaces its fingerprint and bumps `reviewed_at`.
pub fn mark_file_reviewed_impl(
    conn: &Connection,
    review_id: i64,
    file_path: &str,
    fingerprint: &str,
) -> Result<ReviewedFile, String> {
    ensure_review_active(conn, review_id)?;
    conn.execute(
        &format!(
            "INSERT INTO reviewed_files (review_id, file_path, fingerprint)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(review_id, file_path) DO UPDATE
                 SET fingerprint = excluded.fingerprint, reviewed_at = {NOW}"
        ),
        (review_id, file_path, fingerprint),
    )
    .map_err(db_err)?;
    conn.query_row(
        "SELECT file_path, fingerprint, reviewed_at
         FROM reviewed_files WHERE review_id = ?1 AND file_path = ?2",
        (review_id, file_path),
        |r| {
            Ok(ReviewedFile {
                file_path: r.get(0)?,
                fingerprint: r.get(1)?,
                reviewed_at: r.get(2)?,
            })
        },
    )
    .map_err(db_err)
}

/// Idempotent: unmarking a file that has no mark is not an error.
pub fn unmark_file_reviewed_impl(
    conn: &Connection,
    review_id: i64,
    file_path: &str,
) -> Result<(), String> {
    ensure_review_active(conn, review_id)?;
    conn.execute(
        "DELETE FROM reviewed_files WHERE review_id = ?1 AND file_path = ?2",
        (review_id, file_path),
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn list_comments_impl(conn: &Connection, review_id: i64) -> Result<Vec<Comment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, review_id, level, file_path, side, start_line, end_line,
                    code_anchor, commit_sha, state, body, parent_id, author, created_at, updated_at
             FROM comments WHERE review_id = ?1 ORDER BY id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([review_id], comment_columns)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter().map(comment_from_columns).collect()
}

/// Raw column tuple for one comment row; parsed into a [`Comment`] by
/// [`comment_from_columns`] outside the rusqlite row callback.
type CommentColumns = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<u32>,
    Option<String>,
    String,
    String,
    String,
    Option<i64>,
    String,
    String,
    String,
);

fn comment_columns(r: &rusqlite::Row<'_>) -> rusqlite::Result<CommentColumns> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
        r.get(13)?,
        r.get(14)?,
    ))
}

fn comment_from_columns(c: CommentColumns) -> Result<Comment, String> {
    let (
        id,
        review_id,
        level,
        file_path,
        side,
        start_line,
        end_line,
        code_anchor,
        commit_sha,
        state,
        body,
        parent_id,
        author,
        created_at,
        updated_at,
    ) = c;
    let code_anchor = code_anchor
        .map(|json| {
            serde_json::from_str::<CodeAnchor>(&json)
                .map_err(|e| format!("Corrupt code anchor on comment {id}: {e}"))
        })
        .transpose()?;
    Ok(Comment {
        id,
        review_id,
        level: CommentLevel::parse(&level)?,
        file_path,
        side: side.as_deref().map(CommentSide::parse).transpose()?,
        start_line,
        end_line,
        code_anchor,
        commit_sha,
        state: CommentState::parse(&state)?,
        body,
        parent_id,
        author,
        created_at,
        updated_at,
    })
}

fn get_comment(conn: &Connection, id: i64) -> Result<Comment, String> {
    conn.query_row(
        "SELECT id, review_id, level, file_path, side, start_line, end_line,
                code_anchor, commit_sha, state, body, parent_id, author, created_at, updated_at
         FROM comments WHERE id = ?1",
        [id],
        comment_columns,
    )
    .optional()
    .map_err(db_err)?
    .ok_or_else(|| format!("Comment not found: C{id}"))
    .and_then(comment_from_columns)
}

/// Archived reviews are read-only: every comment mutation checks its
/// review's status first.
fn ensure_review_active(conn: &Connection, review_id: i64) -> Result<(), String> {
    let status: Option<String> = conn
        .query_row("SELECT status FROM reviews WHERE id = ?1", [review_id], |r| r.get(0))
        .optional()
        .map_err(db_err)?;
    match status.as_deref() {
        Some("active") => Ok(()),
        Some(_) => Err("This review is archived and read-only".to_owned()),
        None => Err(format!("Review not found: {review_id}")),
    }
}

fn ensure_comment_mutable(conn: &Connection, comment_id: i64) -> Result<(), String> {
    let review_id: Option<i64> = conn
        .query_row(
            "SELECT review_id FROM comments WHERE id = ?1",
            [comment_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let review_id = review_id.ok_or_else(|| format!("Comment not found: C{comment_id}"))?;
    ensure_review_active(conn, review_id)
}

pub fn create_comment_impl(
    conn: &Connection,
    spec: &DiffSpec,
    comment: NewComment,
) -> Result<Comment, String> {
    try_create_comment(conn, spec, comment).map_err(String::from)
}

/// [`create_comment_impl`] with a typed error, for callers that branch on
/// specific failures (the CLI appends a re-read hint to anchor errors).
pub fn try_create_comment(
    conn: &Connection,
    spec: &DiffSpec,
    comment: NewComment,
) -> Result<Comment, CoreError> {
    if comment.body.trim().is_empty() {
        return Err("Comment text cannot be empty".into());
    }
    ensure_review_active(conn, comment.review_id)?;
    let author = comment.author.as_deref().unwrap_or("reviewer");
    if let Some(parent_id) = comment.parent_id {
        return create_reply(
            conn,
            &spec.repo_path,
            &spec.head,
            comment.review_id,
            parent_id,
            &comment.body,
            author,
        )
        .map_err(CoreError::from);
    }
    let (file_path, side, start_line, end_line, anchor) = match comment.level {
        CommentLevel::Review => (None, None, None, None, None),
        CommentLevel::File => {
            let path = comment
                .file_path
                .filter(|p| !p.is_empty())
                .ok_or("A file comment needs a file path")?;
            (Some(path), None, None, None, None)
        }
        CommentLevel::Line => {
            let path = comment
                .file_path
                .filter(|p| !p.is_empty())
                .ok_or("A line comment needs a file path")?;
            let side = comment.side.ok_or("A line comment needs a side")?;
            let (start, end) = match (comment.start_line, comment.end_line) {
                (Some(s), Some(e)) if s >= 1 && s <= e => (s, e),
                _ => return Err("Invalid line range for comment".into()),
            };
            // Anchors are always extracted from the canonical full diff,
            // never a whitespace-filtered view.
            let file_diff = diff::try_get_file_diff(spec, false, &path)?;
            let anchor = extract_anchor(&file_diff, side, start, end)?;
            (Some(path), Some(side), Some(start), Some(end), Some(anchor))
        }
    };

    let repo = open_git_repo(&spec.repo_path)?;
    let commit_sha = diff::resolve_commit(&repo, &spec.head)?.id().to_string();
    let anchor_json = anchor
        .map(|a| serde_json::to_string(&a).map_err(|e| format!("Failed to encode anchor: {e}")))
        .transpose()?;

    conn.execute(
        "INSERT INTO comments (review_id, level, file_path, side, start_line, end_line,
                               code_anchor, commit_sha, body, author)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        (
            comment.review_id,
            comment.level.as_str(),
            &file_path,
            side.map(CommentSide::as_str),
            start_line,
            end_line,
            &anchor_json,
            &commit_sha,
            &comment.body,
            author,
        ),
    )
    .map_err(db_err)?;
    get_comment(conn, conn.last_insert_rowid()).map_err(CoreError::from)
}

/// Append a reply to the thread containing `parent_id`. The reply attaches
/// to the thread root, keeping threads one level deep; it carries only
/// review_id, parent_id, body, and the head SHA at reply time — file, side,
/// lines, and anchor stay NULL (the root's context speaks for the thread).
fn create_reply(
    conn: &Connection,
    repo_path: &str,
    head: &str,
    review_id: i64,
    parent_id: i64,
    body: &str,
    author: &str,
) -> Result<Comment, String> {
    let parent = get_comment(conn, parent_id)?;
    let root = match parent.parent_id {
        None => parent,
        Some(root_id) => get_comment(conn, root_id)?,
    };
    if root.review_id != review_id {
        return Err(format!(
            "Comment C{} belongs to a different review",
            root.id
        ));
    }
    if root.state != CommentState::Open {
        return Err(format!(
            "Cannot reply to a {} thread — reopen it first",
            root.state.as_str()
        ));
    }
    let repo = open_git_repo(repo_path)?;
    let commit_sha = diff::resolve_commit(&repo, head)?.id().to_string();
    conn.execute(
        "INSERT INTO comments (review_id, level, parent_id, commit_sha, body, author)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (review_id, root.level.as_str(), root.id, &commit_sha, body, author),
    )
    .map_err(db_err)?;
    get_comment(conn, conn.last_insert_rowid())
}

pub fn update_comment_impl(
    conn: &Connection,
    comment_id: i64,
    body: &str,
) -> Result<Comment, String> {
    if body.trim().is_empty() {
        return Err("Comment text cannot be empty".to_owned());
    }
    ensure_comment_mutable(conn, comment_id)?;
    let changed = conn
        .execute(
            &format!("UPDATE comments SET body = ?1, updated_at = {NOW} WHERE id = ?2"),
            (body, comment_id),
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(format!("Comment not found: C{comment_id}"));
    }
    get_comment(conn, comment_id)
}

pub fn delete_comment_impl(conn: &Connection, comment_id: i64) -> Result<(), String> {
    ensure_comment_mutable(conn, comment_id)?;
    let changed = conn
        .execute("DELETE FROM comments WHERE id = ?1", [comment_id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err(format!("Comment not found: C{comment_id}"));
    }
    Ok(())
}

pub fn update_comment_state_impl(
    conn: &Connection,
    comment_id: i64,
    state: CommentState,
) -> Result<Comment, String> {
    ensure_comment_mutable(conn, comment_id)?;
    // Lifecycle lives on thread roots; a reply has no state of its own.
    let parent_id: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM comments WHERE id = ?1",
            [comment_id],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if parent_id.is_some() {
        return Err(
            "Replies have no independent state — resolve or dismiss the thread root".to_owned(),
        );
    }
    // `updated_at` deliberately untouched: it tracks body edits ("(edited)"),
    // not lifecycle changes.
    conn.execute(
        "UPDATE comments SET state = ?1 WHERE id = ?2",
        (state.as_str(), comment_id),
    )
    .map_err(db_err)?;
    get_comment(conn, comment_id)
}

/// One line comment's re-anchoring outcome. `start_line`/`end_line` are the
/// comment's current (possibly just-moved) range; orphaned comments keep
/// their last known range.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReanchorResult {
    pub comment_id: i64,
    pub status: AnchorStatus,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

/// When `persist` is false the relocation is computed but not written back —
/// read-only callers (the CLI) get identical results without touching rows.
pub fn reanchor_comments_impl(
    conn: &Connection,
    spec: &DiffSpec,
    review_id: i64,
    persist: bool,
) -> Result<Vec<ReanchorResult>, String> {
    let comments = list_comments_impl(conn, review_id)?;
    // One file-diff fetch per distinct commented file; None = the file has
    // no changes in the current diff (all its line comments orphan).
    let mut diffs: HashMap<String, Option<FileDiff>> = HashMap::new();

    let mut results = Vec::new();
    for comment in &comments {
        let (Some(path), Some(side), Some(anchor), Some(prev_start)) = (
            comment.file_path.as_deref(),
            comment.side,
            comment.code_anchor.as_ref(),
            comment.start_line,
        ) else {
            continue; // review/file-level comments carry no anchor
        };
        let diff = match diffs.get(path) {
            Some(cached) => cached,
            None => {
                // Re-anchoring runs against the canonical full diff so orphan
                // status never depends on the whitespace view preference.
                let fetched = match diff::try_get_file_diff(spec, false, path) {
                    Ok(d) => Some(d),
                    // The file left the diff entirely; anything else is a
                    // real failure worth surfacing.
                    Err(CoreError::NoChangesForFile(_)) => None,
                    Err(e) => return Err(e.to_string()),
                };
                diffs.entry(path.to_owned()).or_insert(fetched)
            }
        };

        let relocation = diff
            .as_ref()
            .and_then(|d| anchor::relocate(d, side, anchor, prev_start));
        let result = match relocation {
            Some(r) => {
                if persist
                    && (comment.start_line, comment.end_line)
                        != (Some(r.start_line), Some(r.end_line))
                {
                    // Follow the code; `updated_at` untouched (not an edit).
                    conn.execute(
                        "UPDATE comments SET start_line = ?1, end_line = ?2 WHERE id = ?3",
                        (r.start_line, r.end_line, comment.id),
                    )
                    .map_err(db_err)?;
                }
                ReanchorResult {
                    comment_id: comment.id,
                    status: if r.changed {
                        AnchorStatus::Changed
                    } else {
                        AnchorStatus::Anchored
                    },
                    start_line: Some(r.start_line),
                    end_line: Some(r.end_line),
                }
            }
            None => ReanchorResult {
                comment_id: comment.id,
                status: AnchorStatus::Orphaned,
                start_line: comment.start_line,
                end_line: comment.end_line,
            },
        };
        results.push(result);
    }
    Ok(results)
}

/// Build the code anchor for a line selection: the selected lines verbatim
/// (on `side`), up to [`ANCHOR_CONTEXT`] same-side lines around them, and the
/// containing hunk's header. The selection must fall inside a single hunk —
/// the UI constrains selections the same way.
fn extract_anchor(
    diff: &FileDiff,
    side: CommentSide,
    start: u32,
    end: u32,
) -> Result<CodeAnchor, CoreError> {
    let lineno = |line: &DiffLine| match side {
        CommentSide::Old => line.old_lineno,
        CommentSide::New => line.new_lineno,
    };
    for hunk in &diff.hunks {
        let mut first: Option<usize> = None;
        let mut last = 0;
        let mut selected = Vec::new();
        for (i, line) in hunk.lines.iter().enumerate() {
            let Some(n) = lineno(line) else { continue };
            if n < start || n > end {
                continue;
            }
            first.get_or_insert(i);
            last = i;
            selected.push(line.content.clone());
        }
        let Some(first) = first else { continue };
        if selected.len() as u32 != end - start + 1 {
            return Err(CoreError::SelectionCrossesHunks);
        }
        let mut context_before: Vec<String> = hunk.lines[..first]
            .iter()
            .rev()
            .filter(|l| lineno(l).is_some())
            .take(ANCHOR_CONTEXT)
            .map(|l| l.content.clone())
            .collect();
        context_before.reverse();
        let context_after: Vec<String> = hunk.lines[last + 1..]
            .iter()
            .filter(|l| lineno(l).is_some())
            .take(ANCHOR_CONTEXT)
            .map(|l| l.content.clone())
            .collect();
        return Ok(CodeAnchor {
            hunk_header: hunk.header.clone(),
            context_before,
            lines: selected,
            context_after,
        });
    }
    Err(CoreError::NoDiffLines {
        path: diff.path.clone(),
        start,
        end,
        side: side.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FixtureRepo;

    fn test_db(dir: &tempfile::TempDir) -> Connection {
        crate::db::open(&dir.path().join("reviews.db")).unwrap()
    }

    /// main has a 10-line file; feature replaces line 6 with two lines and
    /// deletes d.txt.
    fn fixture() -> FixtureRepo {
        let fixture = FixtureRepo::new();
        let lines: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        fixture.write("code.txt", &(lines.join("\n") + "\n"));
        fixture.write("d.txt", "doomed one\ndoomed two\n");
        fixture.stage(&["code.txt", "d.txt"]);
        fixture.commit("initial");

        fixture.create_branch("feature");
        let mut changed = lines.clone();
        changed[5] = "beta 6a\nbeta 6b".to_owned();
        fixture.write("code.txt", &(changed.join("\n") + "\n"));
        fixture.stage(&["code.txt"]);
        fixture.stage_removal(&["d.txt"]);
        fixture.commit("feature work");
        fixture
    }

    fn head_sha(fixture: &FixtureRepo) -> String {
        fixture.repo.head().unwrap().target().unwrap().to_string()
    }

    fn new_comment(review_id: i64, level: CommentLevel, body: &str) -> NewComment {
        NewComment {
            review_id,
            level,
            file_path: None,
            side: None,
            start_line: None,
            end_line: None,
            parent_id: None,
            body: body.to_owned(),
            author: None,
        }
    }

    /// A reply to `parent_id`; level and positional fields are ignored by
    /// the reply path, so any placeholder level works.
    fn reply(review_id: i64, parent_id: i64, body: &str) -> NewComment {
        NewComment {
            parent_id: Some(parent_id),
            ..new_comment(review_id, CommentLevel::Review, body)
        }
    }

    fn line_comment(
        review_id: i64,
        path: &str,
        side: CommentSide,
        start: u32,
        end: u32,
    ) -> NewComment {
        NewComment {
            review_id,
            level: CommentLevel::Line,
            file_path: Some(path.to_owned()),
            side: Some(side),
            start_line: Some(start),
            end_line: Some(end),
            parent_id: None,
            body: "needs work".to_owned(),
            author: None,
        }
    }

    /// main…feature committed-mode spec, as every test here wants it.
    fn spec(fixture: &FixtureRepo) -> DiffSpec {
        DiffSpec {
            repo_path: fixture.path(),
            base: "main".into(),
            head: "feature".into(),
            mode: DiffMode::Committed,
        }
    }

    fn create(conn: &Connection, fixture: &FixtureRepo, comment: NewComment) -> Comment {
        create_comment_impl(conn, &spec(fixture), comment).unwrap()
    }

    #[test]
    fn open_review_creates_then_resumes_the_active_review() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);

        let created =
            open_review_impl(&conn, "/repo", "feature", "origin/main", DiffMode::Committed)
                .unwrap();
        assert_eq!(created.status, "active");
        assert_eq!(created.base_ref, "origin/main");

        // Reopening the branch resumes the same review, following the
        // caller's current base and mode.
        let resumed = open_review_impl(&conn, "/repo", "feature", "main", DiffMode::All).unwrap();
        assert_eq!(resumed.id, created.id);
        assert_eq!(resumed.base_ref, "main");
        assert_eq!(resumed.mode, DiffMode::All);

        // Other branches and repos get their own reviews.
        let other = open_review_impl(&conn, "/repo", "fix", "main", DiffMode::Committed).unwrap();
        assert_ne!(other.id, created.id);
    }

    #[test]
    fn review_and_file_comments_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        let overall = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "LGTM-ish"));
        assert_eq!(overall.level, CommentLevel::Review);
        assert_eq!(overall.state, CommentState::Open);
        assert_eq!(overall.commit_sha, head_sha(&fixture));
        assert_eq!(overall.code_anchor, None);

        let mut file_comment = new_comment(review.id, CommentLevel::File, "split this file");
        file_comment.file_path = Some("code.txt".to_owned());
        let file_comment = create(&conn, &fixture, file_comment);
        assert_eq!(file_comment.file_path.as_deref(), Some("code.txt"));

        let listed = list_comments_impl(&conn, review.id).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, overall.id);
        assert_eq!(listed[1].body, "split this file");
    }

    #[test]
    fn line_comment_captures_anchor_with_context_and_hunk_header() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        // New-side lines 6-7 are the two added "beta" lines.
        let comment = create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );
        assert_eq!(comment.commit_sha, head_sha(&fixture));
        assert_eq!((comment.start_line, comment.end_line), (Some(6), Some(7)));

        let anchor = comment.code_anchor.unwrap();
        assert!(anchor.hunk_header.starts_with("@@"), "{}", anchor.hunk_header);
        assert_eq!(anchor.lines, vec!["beta 6a", "beta 6b"]);
        assert_eq!(anchor.context_before, vec!["alpha 3", "alpha 4", "alpha 5"]);
        assert_eq!(anchor.context_after, vec!["alpha 7", "alpha 8", "alpha 9"]);
    }

    #[test]
    fn old_side_line_comment_anchors_to_deleted_lines() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        let comment = create(
            &conn,
            &fixture,
            line_comment(review.id, "d.txt", CommentSide::Old, 1, 2),
        );
        let anchor = comment.code_anchor.unwrap();
        assert_eq!(anchor.lines, vec!["doomed one", "doomed two"]);
        assert!(anchor.context_before.is_empty());
        assert!(anchor.context_after.is_empty());
    }

    #[test]
    fn anchor_context_is_clipped_at_hunk_edges() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        // Line 4 is the second context line of the hunk (context starts at 3),
        // so only one same-side line precedes it.
        let comment = create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 4, 4),
        );
        let anchor = comment.code_anchor.unwrap();
        assert_eq!(anchor.lines, vec!["alpha 4"]);
        assert_eq!(anchor.context_before, vec!["alpha 3"]);
        assert_eq!(anchor.context_after, vec!["alpha 5", "beta 6a", "beta 6b"]);
    }

    #[test]
    fn line_comments_outside_any_hunk_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        let err = create_comment_impl(
            &conn,
            &spec(&fixture),
            line_comment(review.id, "code.txt", CommentSide::New, 1, 1),
        )
        .unwrap_err();
        assert!(err.contains("No diff lines"), "{err}");
    }

    #[test]
    fn comment_validation_rejects_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let create = |c: NewComment| {
            create_comment_impl(&conn, &spec(&fixture), c)
        };

        let empty = new_comment(review.id, CommentLevel::Review, "   ");
        assert!(create(empty).unwrap_err().contains("empty"));

        let no_path = new_comment(review.id, CommentLevel::File, "where?");
        assert!(create(no_path).unwrap_err().contains("file path"));

        let backwards = line_comment(review.id, "code.txt", CommentSide::New, 7, 6);
        assert!(create(backwards).unwrap_err().contains("line range"));
    }

    #[test]
    fn comments_can_be_edited_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let comment = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "v1"));

        let edited = update_comment_impl(&conn, comment.id, "v2").unwrap();
        assert_eq!(edited.body, "v2");
        assert_eq!(edited.id, comment.id);

        delete_comment_impl(&conn, comment.id).unwrap();
        assert!(list_comments_impl(&conn, review.id).unwrap().is_empty());
        assert!(delete_comment_impl(&conn, comment.id).is_err());
        assert!(update_comment_impl(&conn, comment.id, "gone").is_err());
    }

    #[test]
    fn comment_state_transitions_persist_without_touching_updated_at() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let comment = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "hm"));

        let resolved = update_comment_state_impl(&conn, comment.id, CommentState::Resolved).unwrap();
        assert_eq!(resolved.state, CommentState::Resolved);
        assert_eq!(resolved.updated_at, comment.updated_at, "state is not an edit");

        let reopened = update_comment_state_impl(&conn, comment.id, CommentState::Open).unwrap();
        assert_eq!(reopened.state, CommentState::Open);

        let dismissed =
            update_comment_state_impl(&conn, comment.id, CommentState::Dismissed).unwrap();
        assert_eq!(dismissed.state, CommentState::Dismissed);
        // Closed comments stay in history.
        assert_eq!(list_comments_impl(&conn, review.id).unwrap().len(), 1);

        assert!(update_comment_state_impl(&conn, 999, CommentState::Resolved).is_err());
    }

    #[test]
    fn replies_attach_to_the_thread_root_at_every_level() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        let review_root =
            create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "overall"));
        let mut file_root = new_comment(review.id, CommentLevel::File, "split this");
        file_root.file_path = Some("code.txt".to_owned());
        let file_root = create(&conn, &fixture, file_root);
        let line_root = create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );

        for root in [&review_root, &file_root, &line_root] {
            let r = create(&conn, &fixture, reply(review.id, root.id, "to clarify"));
            assert_eq!(r.parent_id, Some(root.id));
            assert_eq!(r.level, root.level);
            assert_eq!(r.review_id, review.id);
            assert_eq!(r.commit_sha, head_sha(&fixture));
            // Replies inherit context from the root: no position of their own.
            assert_eq!(r.file_path, None);
            assert_eq!(r.side, None);
            assert_eq!((r.start_line, r.end_line), (None, None));
            assert_eq!(r.code_anchor, None);
            assert_eq!(r.state, CommentState::Open);
        }
        assert_eq!(list_comments_impl(&conn, review.id).unwrap().len(), 6);
    }

    #[test]
    fn author_defaults_to_reviewer_and_external_authors_persist() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        // The app's payloads never set an author: comments and replies land
        // as 'reviewer'.
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        assert_eq!(root.author, "reviewer");
        let app_reply = create(&conn, &fixture, reply(review.id, root.id, "noted"));
        assert_eq!(app_reply.author, "reviewer");

        // External writers name themselves, on roots and replies alike.
        let mut agent_root = new_comment(review.id, CommentLevel::Review, "a concern");
        agent_root.author = Some("agent".to_owned());
        assert_eq!(create(&conn, &fixture, agent_root).author, "agent");
        let mut named_reply = reply(review.id, root.id, "done in abc123");
        named_reply.author = Some("skyler".to_owned());
        assert_eq!(create(&conn, &fixture, named_reply).author, "skyler");

        // The stored rows round-trip through the list read.
        let authors: Vec<String> = list_comments_impl(&conn, review.id)
            .unwrap()
            .into_iter()
            .map(|c| c.author)
            .collect();
        assert_eq!(authors, ["reviewer", "reviewer", "agent", "skyler"]);
    }

    #[test]
    fn replying_to_a_reply_joins_the_same_flat_thread() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        let first = create(&conn, &fixture, reply(review.id, root.id, "first reply"));
        // Replying to the reply lands under the root, never nests deeper.
        let second = create(&conn, &fixture, reply(review.id, first.id, "second reply"));
        assert_eq!(second.parent_id, Some(root.id));
    }

    #[test]
    fn reply_creation_is_validated() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        let try_create = |c: NewComment| {
            create_comment_impl(&conn, &spec(&fixture), c)
        };

        let err = try_create(reply(review.id, 999, "hi")).unwrap_err();
        assert!(err.contains("Comment not found"), "{err}");

        let err = try_create(reply(review.id, root.id, "   ")).unwrap_err();
        assert!(err.contains("empty"), "{err}");

        // A reply cannot cross into another review's thread.
        let other =
            open_review_impl(&conn, &fixture.path(), "other", "main", DiffMode::Committed)
                .unwrap();
        let err = try_create(reply(other.id, root.id, "hi")).unwrap_err();
        assert!(err.contains("different review"), "{err}");

        // Closed threads take no new replies until reopened.
        update_comment_state_impl(&conn, root.id, CommentState::Resolved).unwrap();
        let err = try_create(reply(review.id, root.id, "late")).unwrap_err();
        assert!(err.contains("reopen"), "{err}");
        update_comment_state_impl(&conn, root.id, CommentState::Open).unwrap();
        try_create(reply(review.id, root.id, "on time")).unwrap();
    }

    #[test]
    fn replies_have_no_independent_lifecycle_state() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        let r = create(&conn, &fixture, reply(review.id, root.id, "reply"));

        for state in [CommentState::Resolved, CommentState::Dismissed, CommentState::Open] {
            let err = update_comment_state_impl(&conn, r.id, state).unwrap_err();
            assert!(err.contains("thread root"), "{err}");
        }
        // The root's lifecycle still governs the thread.
        let resolved = update_comment_state_impl(&conn, root.id, CommentState::Resolved).unwrap();
        assert_eq!(resolved.state, CommentState::Resolved);
    }

    #[test]
    fn deleting_a_root_cascades_the_thread_but_a_reply_deletes_alone() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        let keep = create(&conn, &fixture, reply(review.id, root.id, "kept until cascade"));
        let mistake = create(&conn, &fixture, reply(review.id, root.id, "typo"));

        // Deleting a reply removes just it (mistakes only).
        delete_comment_impl(&conn, mistake.id).unwrap();
        let ids: Vec<i64> = list_comments_impl(&conn, review.id)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec![root.id, keep.id]);

        // Deleting the root cascades the whole thread (SQLite FK path).
        delete_comment_impl(&conn, root.id).unwrap();
        assert!(list_comments_impl(&conn, review.id).unwrap().is_empty());
    }

    #[test]
    fn replies_can_be_edited_like_any_comment() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        let r = create(&conn, &fixture, reply(review.id, root.id, "v1"));

        let edited = update_comment_impl(&conn, r.id, "v2").unwrap();
        assert_eq!(edited.body, "v2");
        assert_eq!(edited.parent_id, Some(root.id));
    }

    /// Edit code.txt on feature (as a new commit) and re-anchor the review's
    /// comments against the refreshed committed diff.
    fn reanchor_after_edit(
        conn: &Connection,
        fixture: &FixtureRepo,
        review_id: i64,
        new_lines: &[String],
    ) -> Vec<ReanchorResult> {
        fixture.commit_file("code.txt", &(new_lines.join("\n") + "\n"), "edit");
        reanchor_comments_impl(conn, &spec(fixture), review_id, true).unwrap()
    }

    fn feature_lines() -> Vec<String> {
        // code.txt on feature: alpha 1-5, beta 6a/6b, alpha 7-10.
        let mut lines: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        lines.splice(5..6, ["beta 6a".to_owned(), "beta 6b".to_owned()]);
        lines
    }

    #[test]
    fn reanchor_follows_code_shifted_by_edits_above() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let comment = create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );

        let mut lines = feature_lines();
        lines.splice(0..0, ["intro one".to_owned(), "intro two".to_owned()]);
        let results = reanchor_after_edit(&conn, &fixture, review.id, &lines);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].comment_id, comment.id);
        assert_eq!(results[0].status, AnchorStatus::Anchored);
        assert_eq!((results[0].start_line, results[0].end_line), (Some(8), Some(9)));

        // The move is persisted so placement survives reloads.
        let stored = get_comment(&conn, comment.id).unwrap();
        assert_eq!((stored.start_line, stored.end_line), (Some(8), Some(9)));
        assert_eq!(stored.updated_at, comment.updated_at);
    }

    #[test]
    fn reanchor_flags_changed_context_around_an_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );

        let mut lines = feature_lines();
        lines[4] = "alpha 5 REWRITTEN".to_owned(); // context line just above
        let results = reanchor_after_edit(&conn, &fixture, review.id, &lines);

        assert_eq!(results[0].status, AnchorStatus::Changed);
        assert_eq!((results[0].start_line, results[0].end_line), (Some(6), Some(7)));
    }

    #[test]
    fn reanchor_fuzzy_matches_lightly_edited_anchor_lines() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );

        let mut lines = feature_lines();
        lines[5] = "beta 6a tweaked".to_owned();
        let results = reanchor_after_edit(&conn, &fixture, review.id, &lines);

        assert_eq!(results[0].status, AnchorStatus::Changed);
        assert_eq!((results[0].start_line, results[0].end_line), (Some(6), Some(7)));
    }

    #[test]
    fn reanchor_orphans_comments_whose_code_left_the_diff() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let comment = create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );

        // Revert code.txt to main's content: the file leaves the diff.
        let lines: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        let results = reanchor_after_edit(&conn, &fixture, review.id, &lines);

        assert_eq!(results[0].status, AnchorStatus::Orphaned);
        // Last known position is kept, never nulled.
        let stored = get_comment(&conn, comment.id).unwrap();
        assert_eq!((stored.start_line, stored.end_line), (Some(6), Some(7)));
    }

    #[test]
    fn reanchor_skips_replies_and_moves_threads_as_a_unit() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );
        let r = create(&conn, &fixture, reply(review.id, root.id, "context from me"));

        // Shift the commented code down: only the root re-anchors; the reply
        // has no position and follows the thread implicitly.
        let mut lines = feature_lines();
        lines.splice(0..0, ["intro one".to_owned(), "intro two".to_owned()]);
        let results = reanchor_after_edit(&conn, &fixture, review.id, &lines);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].comment_id, root.id);
        assert_eq!(results[0].status, AnchorStatus::Anchored);
        let stored = get_comment(&conn, r.id).unwrap();
        assert_eq!(stored.parent_id, Some(root.id));
        assert_eq!((stored.start_line, stored.end_line), (None, None));

        // Orphan the root: the reply stays attached to it.
        let reverted: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        let results = reanchor_after_edit(&conn, &fixture, review.id, &reverted);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, AnchorStatus::Orphaned);
        assert_eq!(get_comment(&conn, r.id).unwrap().parent_id, Some(root.id));
    }

    #[test]
    fn archived_reviews_refuse_reply_mutations_too() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "root"));
        let r = create(&conn, &fixture, reply(review.id, root.id, "reply"));
        archive_review(&conn, review.id).unwrap();

        let err = create_comment_impl(
            &conn,
            &spec(&fixture),
            reply(review.id, root.id, "late reply"),
        )
        .unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err = update_comment_impl(&conn, r.id, "rewrite").unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err = delete_comment_impl(&conn, r.id).unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        // Archived threads stay browsable, replies included.
        assert_eq!(list_comments_impl(&conn, review.id).unwrap().len(), 2);
    }

    #[test]
    fn reanchor_leaves_untouched_comments_anchored_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        create(
            &conn,
            &fixture,
            line_comment(review.id, "code.txt", CommentSide::New, 6, 7),
        );
        // Old-side comment on the deleted file.
        create(
            &conn,
            &fixture,
            line_comment(review.id, "d.txt", CommentSide::Old, 1, 2),
        );
        // Review-level comments are skipped entirely.
        create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "overall"));

        let results = reanchor_comments_impl(
            &conn,
            &spec(&fixture),
            review.id,
            true,
        )
        .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status == AnchorStatus::Anchored));
        assert_eq!((results[0].start_line, results[0].end_line), (Some(6), Some(7)));
        assert_eq!((results[1].start_line, results[1].end_line), (Some(1), Some(2)));
    }

    #[test]
    fn merging_the_branch_archives_its_review() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        // Not merged yet: nothing to archive.
        let none = archive_stale_reviews_impl(&conn, &fixture.repo, &fixture.path()).unwrap();
        assert!(none.is_empty());

        fixture.merge_into("main", "feature");
        let archived = archive_stale_reviews_impl(&conn, &fixture.repo, &fixture.path()).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, review.id);
        assert_eq!(archived[0].status, "archived");

        // Idempotent: the next scan finds nothing active.
        let again = archive_stale_reviews_impl(&conn, &fixture.repo, &fixture.path()).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn deleting_the_branch_archives_its_review() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        // A side branch that is reviewed, then deleted (HEAD stays on feature).
        fixture.repo
            .branch("doomed", &fixture.repo.head().unwrap().peel_to_commit().unwrap(), false)
            .unwrap();
        let review =
            open_review_impl(&conn, &fixture.path(), "doomed", "main", DiffMode::Committed)
                .unwrap();

        fixture.delete_branch("doomed");
        let archived = archive_stale_reviews_impl(&conn, &fixture.repo, &fixture.path()).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, review.id);
    }

    #[test]
    fn fresh_and_remote_branches_are_not_stale() {
        let fixture = fixture();
        // Equal tips (fresh branch off its base) must not read as merged.
        fixture.repo
            .branch("fresh", &fixture.repo.head().unwrap().peel_to_commit().unwrap(), false)
            .unwrap();
        assert_eq!(stale_reason(&fixture.repo, "fresh", "feature"), None);
        // Remote-tracking branch names resolve through the remote lookup.
        fixture.add_remote_branch("feature", "feature");
        assert_eq!(stale_reason(&fixture.repo, "origin/feature", "main"), None);
        // A branch that never existed is stale.
        assert_eq!(
            stale_reason(&fixture.repo, "never-existed", "main"),
            Some(StaleReason::Deleted)
        );
    }

    #[test]
    fn open_review_on_a_merged_branch_returns_the_archived_review_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review = open_review_checked_impl(
            &conn,
            &fixture.repo,
            &fixture.path(),
            "feature",
            "main",
            DiffMode::Committed,
        )
        .unwrap();
        assert!(!review.branch_merged);
        let review = review.review.unwrap();
        assert_eq!(review.status, "active");

        fixture.merge_into("main", "feature");
        let opened = open_review_checked_impl(
            &conn,
            &fixture.repo,
            &fixture.path(),
            "feature",
            "main",
            DiffMode::Committed,
        )
        .unwrap();
        assert!(opened.branch_merged);
        let archived = opened.review.unwrap();
        assert_eq!(archived.id, review.id);
        assert_eq!(archived.status, "archived");

        // Re-opening never spawns a fresh active review for a merged branch.
        let reopened = open_review_checked_impl(
            &conn,
            &fixture.repo,
            &fixture.path(),
            "feature",
            "main",
            DiffMode::Committed,
        )
        .unwrap();
        assert_eq!(reopened.review.unwrap().id, review.id);
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM reviews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn archived_reviews_are_listed_and_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let comment = create(&conn, &fixture, new_comment(review.id, CommentLevel::Review, "hm"));
        create(&conn, &fixture, line_comment(review.id, "code.txt", CommentSide::New, 6, 7));

        assert!(list_archived_reviews_impl(&conn, &fixture.path()).unwrap().is_empty());
        archive_review(&conn, review.id).unwrap();

        let listed = list_archived_reviews_impl(&conn, &fixture.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].review.id, review.id);
        assert_eq!(listed[0].comment_count, 2);
        // Archived comments stay readable...
        assert_eq!(list_comments_impl(&conn, review.id).unwrap().len(), 2);
        // ...but every mutation is refused.
        let err = update_comment_impl(&conn, comment.id, "rewrite").unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err = delete_comment_impl(&conn, comment.id).unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err =
            update_comment_state_impl(&conn, comment.id, CommentState::Resolved).unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err = create_comment_impl(
            &conn,
            &spec(&fixture),
            new_comment(review.id, CommentLevel::Review, "late"),
        )
        .unwrap_err();
        assert!(err.contains("read-only"), "{err}");
    }

    #[test]
    fn comments_persist_across_database_reconnects() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = fixture();
        let review_id;
        {
            let conn = test_db(&dir);
            let review =
                open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                    .unwrap();
            review_id = review.id;
            create(&conn, &fixture, line_comment(review_id, "code.txt", CommentSide::New, 6, 7));
        }

        // A fresh connection — as after an app restart — sees the same data.
        let conn = test_db(&dir);
        let resumed =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        assert_eq!(resumed.id, review_id);
        let comments = list_comments_impl(&conn, review_id).unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].code_anchor.as_ref().unwrap().lines, vec![
            "beta 6a", "beta 6b"
        ]);
    }

    #[test]
    fn reanchor_without_persist_computes_moves_but_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        create(&conn, &fixture, line_comment(review.id, "code.txt", CommentSide::New, 6, 7));

        // Insert two lines above the commented ones so the anchor moves.
        let mut lines = feature_lines();
        lines.splice(0..0, ["intro one".to_owned(), "intro two".to_owned()]);
        fixture.commit_file("code.txt", &(lines.join("\n") + "\n"), "edit");

        let results = reanchor_comments_impl(
            &conn,
            &spec(&fixture),
            review.id,
            false,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!((results[0].start_line, results[0].end_line), (Some(8), Some(9)));

        // The computed move was NOT written back.
        let stored = list_comments_impl(&conn, review.id).unwrap();
        assert_eq!((stored[0].start_line, stored[0].end_line), (Some(6), Some(7)));

        // The same call with persist writes exactly the ranges it computed.
        let persisted = reanchor_comments_impl(
            &conn,
            &spec(&fixture),
            review.id,
            true,
        )
        .unwrap();
        assert_eq!((persisted[0].start_line, persisted[0].end_line), (Some(8), Some(9)));
        let stored = list_comments_impl(&conn, review.id).unwrap();
        assert_eq!((stored[0].start_line, stored[0].end_line), (Some(8), Some(9)));
    }

    #[test]
    fn list_reviews_puts_active_first_and_honors_filters() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let a = open_review_impl(&conn, "/repo-a", "feature", "main", DiffMode::Committed).unwrap();
        let b = open_review_impl(&conn, "/repo-b", "fix", "main", DiffMode::Committed).unwrap();
        let old = open_review_impl(&conn, "/repo-a", "done", "main", DiffMode::Committed).unwrap();
        archive_review(&conn, old.id).unwrap();

        // Default: active only, across repos.
        let active = list_reviews_impl(&conn, None, false).unwrap();
        let ids: Vec<i64> = active.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&a.id) && ids.contains(&b.id));

        // Archived included: active rows come first.
        let all = list_reviews_impl(&conn, None, true).unwrap();
        assert_eq!(all.len(), 3);
        assert!(all[..2].iter().all(|r| r.status == "active"));
        assert_eq!(all[2].id, old.id);

        // Repo filter applies to both shapes.
        let repo_a = list_reviews_impl(&conn, Some("/repo-a"), true).unwrap();
        let repo_a_ids: Vec<i64> = repo_a.iter().map(|r| r.id).collect();
        assert_eq!(repo_a_ids, vec![a.id, old.id]);
    }

    #[test]
    fn find_active_review_reads_without_creating() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        assert_eq!(find_active_review_impl(&conn, "/repo", "feature").unwrap().map(|r| r.id), None);
        // The miss above must not have created anything.
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM reviews", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);

        let review = open_review_impl(&conn, "/repo", "feature", "main", DiffMode::Committed).unwrap();
        let found = find_active_review_impl(&conn, "/repo", "feature").unwrap().unwrap();
        assert_eq!(found.id, review.id);

        // Archived reviews are not "active".
        archive_review(&conn, review.id).unwrap();
        assert!(find_active_review_impl(&conn, "/repo", "feature").unwrap().is_none());
    }

    #[test]
    fn reviewed_files_round_trip_and_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let review = open_review_impl(&conn, "/repo", "feature", "main", DiffMode::Committed).unwrap();

        assert!(list_reviewed_files_impl(&conn, review.id).unwrap().is_empty());

        let marked = mark_file_reviewed_impl(&conn, review.id, "src/a.rs", "aaa:bbb:644").unwrap();
        assert_eq!(marked.file_path, "src/a.rs");
        assert_eq!(marked.fingerprint, "aaa:bbb:644");
        mark_file_reviewed_impl(&conn, review.id, "src/b.rs", "ccc:ddd:644").unwrap();

        let listed = list_reviewed_files_impl(&conn, review.id).unwrap();
        assert_eq!(
            listed.iter().map(|f| f.file_path.as_str()).collect::<Vec<_>>(),
            vec!["src/a.rs", "src/b.rs"]
        );

        // Re-marking (a "changed since review" file) replaces the fingerprint.
        let remarked = mark_file_reviewed_impl(&conn, review.id, "src/a.rs", "aaa:eee:644").unwrap();
        assert_eq!(remarked.fingerprint, "aaa:eee:644");
        let listed = list_reviewed_files_impl(&conn, review.id).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].fingerprint, "aaa:eee:644");
    }

    #[test]
    fn unmark_file_reviewed_deletes_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let review = open_review_impl(&conn, "/repo", "feature", "main", DiffMode::Committed).unwrap();
        mark_file_reviewed_impl(&conn, review.id, "src/a.rs", "aaa:bbb:644").unwrap();

        unmark_file_reviewed_impl(&conn, review.id, "src/a.rs").unwrap();
        assert!(list_reviewed_files_impl(&conn, review.id).unwrap().is_empty());
        // Unmarking again (or a never-marked path) is not an error.
        unmark_file_reviewed_impl(&conn, review.id, "src/a.rs").unwrap();
        unmark_file_reviewed_impl(&conn, review.id, "never-marked.rs").unwrap();
    }

    #[test]
    fn reviewed_file_writes_are_rejected_on_archived_reviews() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let review = open_review_impl(&conn, "/repo", "feature", "main", DiffMode::Committed).unwrap();
        mark_file_reviewed_impl(&conn, review.id, "src/a.rs", "aaa:bbb:644").unwrap();
        archive_review(&conn, review.id).unwrap();

        let err = mark_file_reviewed_impl(&conn, review.id, "src/b.rs", "ccc:ddd:644").unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err = unmark_file_reviewed_impl(&conn, review.id, "src/a.rs").unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        // Reads still work on archived reviews.
        assert_eq!(list_reviewed_files_impl(&conn, review.id).unwrap().len(), 1);
    }

    #[test]
    fn reviewed_files_are_scoped_to_their_review() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let a = open_review_impl(&conn, "/repo-a", "feature", "main", DiffMode::Committed).unwrap();
        let b = open_review_impl(&conn, "/repo-b", "fix", "main", DiffMode::Committed).unwrap();
        mark_file_reviewed_impl(&conn, a.id, "src/a.rs", "aaa:bbb:644").unwrap();

        assert_eq!(list_reviewed_files_impl(&conn, a.id).unwrap().len(), 1);
        assert!(list_reviewed_files_impl(&conn, b.id).unwrap().is_empty());
    }
}
