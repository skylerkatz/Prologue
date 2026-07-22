//! Comments and threads: types, CRUD, replies, re-anchoring, and the shared
//! thread-resolution pipeline behind exports and the CLI's show.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::anchor::{self, extract_anchor, AnchorStatus, CodeAnchor};
use crate::db::{db_err, NOW};
use crate::diff::{self, CommentSide, DiffSpec, FileDiff};
use crate::error::CoreError;
use crate::repo::open_git_repo;
use crate::review::ensure_review_active;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum CommentState {
    Open,
    Resolved,
    Dismissed,
}

impl CommentState {
    /// Stable text form: the reviews database value, also what CLI output
    /// and exports print.
    pub fn as_str(self) -> &'static str {
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

#[derive(Serialize, Debug, Clone)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// SQLite rowid — far below 2^53, a plain JS number on the wire.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
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
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NewComment {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub review_id: i64,
    pub level: CommentLevel,
    #[cfg_attr(feature = "ts", ts(optional))]
    pub file_path: Option<String>,
    #[cfg_attr(feature = "ts", ts(optional))]
    pub side: Option<CommentSide>,
    #[cfg_attr(feature = "ts", ts(optional))]
    pub start_line: Option<u32>,
    #[cfg_attr(feature = "ts", ts(optional))]
    pub end_line: Option<u32>,
    /// Set to any comment in a thread to reply; the reply attaches to the
    /// thread ROOT (replying to a reply joins the same flat thread). All
    /// positional fields above are ignored for replies — a reply inherits
    /// its context from the root.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub parent_id: Option<i64>,
    pub body: String,
    /// Who is writing. The app's IPC payloads never set it (None →
    /// 'reviewer'); external writers name themselves, e.g. 'agent'.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub author: Option<String>,
}

/// How many comments (roots and replies) a review holds — a bare COUNT, so
/// listings don't have to fetch and parse every comment row.
pub fn comment_count(conn: &Connection, review_id: i64) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM comments WHERE review_id = ?1", [review_id], |r| {
        r.get(0)
    })
    .map_err(db_err)
}

/// Column list shared by every comment SELECT. Rows are read by NAME (see
/// [`comment_row`]) so this list's order can never silently drift out of
/// sync with the mapping; the constant just keeps the queries explicit.
const COMMENT_COLUMNS: &str = "id, review_id, level, file_path, side, start_line, end_line, \
     code_anchor, commit_sha, state, body, parent_id, author, created_at, updated_at";

/// Raw column values for one comment row, read by name inside the rusqlite
/// row callback; parsed into a [`Comment`] by [`comment_from_row`] outside
/// it (enum and anchor parsing errors are String-typed).
struct CommentRow {
    id: i64,
    review_id: i64,
    level: String,
    file_path: Option<String>,
    side: Option<String>,
    start_line: Option<u32>,
    end_line: Option<u32>,
    code_anchor: Option<String>,
    commit_sha: String,
    state: String,
    body: String,
    parent_id: Option<i64>,
    author: String,
    created_at: String,
    updated_at: String,
}

fn comment_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CommentRow> {
    Ok(CommentRow {
        id: r.get("id")?,
        review_id: r.get("review_id")?,
        level: r.get("level")?,
        file_path: r.get("file_path")?,
        side: r.get("side")?,
        start_line: r.get("start_line")?,
        end_line: r.get("end_line")?,
        code_anchor: r.get("code_anchor")?,
        commit_sha: r.get("commit_sha")?,
        state: r.get("state")?,
        body: r.get("body")?,
        parent_id: r.get("parent_id")?,
        author: r.get("author")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

fn comment_from_row(row: CommentRow) -> Result<Comment, String> {
    let code_anchor = row
        .code_anchor
        .map(|json| {
            serde_json::from_str::<CodeAnchor>(&json)
                .map_err(|e| format!("Corrupt code anchor on comment {}: {e}", row.id))
        })
        .transpose()?;
    Ok(Comment {
        id: row.id,
        review_id: row.review_id,
        level: CommentLevel::parse(&row.level)?,
        file_path: row.file_path,
        side: row.side.as_deref().map(CommentSide::parse).transpose()?,
        start_line: row.start_line,
        end_line: row.end_line,
        code_anchor,
        commit_sha: row.commit_sha,
        state: CommentState::parse(&row.state)?,
        body: row.body,
        parent_id: row.parent_id,
        author: row.author,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub fn list_comments_impl(conn: &Connection, review_id: i64) -> Result<Vec<Comment>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COMMENT_COLUMNS} FROM comments WHERE review_id = ?1 ORDER BY id"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([review_id], comment_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter().map(comment_from_row).collect()
}

/// The comment row for `id`; a missing comment is an error.
pub fn get_comment(conn: &Connection, id: i64) -> Result<Comment, String> {
    conn.query_row(
        &format!("SELECT {COMMENT_COLUMNS} FROM comments WHERE id = ?1"),
        [id],
        comment_row,
    )
    .optional()
    .map_err(db_err)?
    .ok_or_else(|| format!("Comment not found: C{id}"))
    .and_then(comment_from_row)
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ReanchorResult {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
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
    // Nothing anchored means nothing to relocate — don't touch the repo.
    if !comments.iter().any(|c| c.code_anchor.is_some()) {
        return Ok(Vec::new());
    }
    let repo = open_git_repo(&spec.repo_path)?;
    let repo_diff = diff::RepoDiff::compute(&repo, spec, false)?;
    reanchor_listed(conn, &repo_diff, &comments, persist)
}

/// The re-anchoring loop over already-listed comments, extracting every
/// file's hunks from ONE computed diff. Runs against the canonical full
/// diff (`RepoDiff` computed with `ignore_whitespace = false`) so orphan
/// status never depends on the whitespace view preference.
fn reanchor_listed(
    conn: &Connection,
    repo_diff: &diff::RepoDiff<'_>,
    comments: &[Comment],
    persist: bool,
) -> Result<Vec<ReanchorResult>, String> {
    // One file-diff extraction per distinct commented file; None = the file
    // has no changes in the current diff (all its line comments orphan).
    let mut diffs: HashMap<String, Option<FileDiff>> = HashMap::new();

    let mut results = Vec::new();
    for comment in comments {
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
                let fetched = match repo_diff.file_diff(path) {
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

/// One resolved thread: the root with current (possibly relocated) line
/// ranges, whether it is orphaned in the current diff (its anchor no longer
/// matches, or its file left the diff entirely; `None` when anchors were
/// not recomputed), and the root's replies in chronological (id) order.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub root: Comment,
    pub orphaned: Option<bool>,
    pub replies: Vec<Comment>,
}

/// Re-locate the review's line comments in the current diff and group its
/// comments into threads — the shared pipeline behind exports and the CLI's
/// show. Relocated ranges are patched onto the returned comments; with
/// `persist` they are also written back (as the app's refresh does), without
/// it nothing is written and the result is identical.
pub fn resolve_threads(
    conn: &Connection,
    spec: &DiffSpec,
    review_id: i64,
    persist: bool,
) -> Result<Vec<Thread>, String> {
    let repo = open_git_repo(&spec.repo_path)?;
    let repo_diff = diff::RepoDiff::compute(&repo, spec, false)?;
    resolve_threads_with(conn, &repo_diff, review_id, persist)
}

/// [`resolve_threads`] against an already-computed diff, for callers (the
/// export) that also read the diff themselves — the whole operation costs
/// one diff computation.
pub fn resolve_threads_with(
    conn: &Connection,
    repo_diff: &diff::RepoDiff<'_>,
    review_id: i64,
    persist: bool,
) -> Result<Vec<Thread>, String> {
    let mut comments = list_comments_impl(conn, review_id)?;
    let reanchored = reanchor_listed(conn, repo_diff, &comments, persist)?;
    let orphaned_anchors: HashSet<i64> = reanchored
        .iter()
        .filter(|r| r.status == AnchorStatus::Orphaned)
        .map(|r| r.comment_id)
        .collect();
    let relocated: HashMap<i64, (Option<u32>, Option<u32>)> = reanchored
        .iter()
        .map(|r| (r.comment_id, (r.start_line, r.end_line)))
        .collect();
    // Orphaning also covers files with no delta left in the diff.
    let summary = repo_diff.summary()?;
    let diff_paths: HashSet<String> = summary.files.into_iter().map(|f| f.path).collect();

    // Present the computed ranges, not the stored ones — a no-op when they
    // were just persisted, the whole point when they were not.
    for comment in &mut comments {
        if let Some(&(start, end)) = relocated.get(&comment.id) {
            comment.start_line = start;
            comment.end_line = end;
        }
    }
    Ok(group_threads(comments, |root| {
        Some(
            orphaned_anchors.contains(&root.id)
                || root.file_path.as_deref().is_some_and(|p| !diff_paths.contains(p)),
        )
    }))
}

/// Threads from stored values alone: stored line ranges, no orphan flags.
/// The fallback when the current diff is unavailable (e.g. an archived
/// review whose branch is gone).
pub fn stored_threads(conn: &Connection, review_id: i64) -> Result<Vec<Thread>, String> {
    Ok(group_threads(list_comments_impl(conn, review_id)?, |_| None))
}

/// Split comments into roots and replies grouped under their root, keeping
/// the incoming (id) order on both.
fn group_threads(
    comments: Vec<Comment>,
    orphaned: impl Fn(&Comment) -> Option<bool>,
) -> Vec<Thread> {
    let mut replies_by_root: HashMap<i64, Vec<Comment>> = HashMap::new();
    let mut roots = Vec::new();
    for comment in comments {
        match comment.parent_id {
            Some(root_id) => replies_by_root.entry(root_id).or_default().push(comment),
            None => roots.push(comment),
        }
    }
    roots
        .into_iter()
        .map(|root| Thread {
            orphaned: orphaned(&root),
            replies: replies_by_root.remove(&root.id).unwrap_or_default(),
            root,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffMode;
    use crate::review::{archive_review, open_review_impl};
    use crate::testutil::{open_test_db as test_db, FixtureRepo};

    fn fixture() -> FixtureRepo {
        FixtureRepo::standard_review_fixture()
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
}
