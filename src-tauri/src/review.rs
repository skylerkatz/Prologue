use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::{Db, NOW};
use crate::diff::{self, DiffLine, DiffMode, FileDiff};
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommentLevel {
    Review,
    File,
    Line,
}

impl CommentLevel {
    fn as_str(self) -> &'static str {
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
    fn as_str(self) -> &'static str {
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
    pub body: String,
}

/// Resume the active review for (repo, branch), creating one if none exists.
/// The stored base ref and mode follow the caller's current choice.
#[tauri::command]
pub fn open_review(
    db: tauri::State<'_, Db>,
    repo_path: String,
    branch: String,
    base_ref: String,
    mode: DiffMode,
) -> Result<Review, String> {
    let conn = lock(&db)?;
    open_review_impl(&conn, &repo_path, &branch, &base_ref, mode)
}

#[tauri::command]
pub fn list_comments(db: tauri::State<'_, Db>, review_id: i64) -> Result<Vec<Comment>, String> {
    let conn = lock(&db)?;
    list_comments_impl(&conn, review_id)
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
    create_comment_impl(&conn, &repo_path, &base, &head, mode, comment)
}

#[tauri::command]
pub fn update_comment(
    db: tauri::State<'_, Db>,
    comment_id: i64,
    body: String,
) -> Result<Comment, String> {
    let conn = lock(&db)?;
    update_comment_impl(&conn, comment_id, &body)
}

#[tauri::command]
pub fn delete_comment(db: tauri::State<'_, Db>, comment_id: i64) -> Result<(), String> {
    let conn = lock(&db)?;
    delete_comment_impl(&conn, comment_id)
}

fn lock<'a>(db: &'a tauri::State<'_, Db>) -> Result<std::sync::MutexGuard<'a, Connection>, String> {
    db.0.lock().map_err(|_| "Review database is unavailable".to_owned())
}

fn db_err(e: rusqlite::Error) -> String {
    format!("Review database error: {e}")
}

pub(crate) fn open_review_impl(
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

pub(crate) fn list_comments_impl(conn: &Connection, review_id: i64) -> Result<Vec<Comment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, review_id, level, file_path, side, start_line, end_line,
                    code_anchor, commit_sha, state, body, created_at, updated_at
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
        created_at,
        updated_at,
    })
}

fn get_comment(conn: &Connection, id: i64) -> Result<Comment, String> {
    conn.query_row(
        "SELECT id, review_id, level, file_path, side, start_line, end_line,
                code_anchor, commit_sha, state, body, created_at, updated_at
         FROM comments WHERE id = ?1",
        [id],
        comment_columns,
    )
    .optional()
    .map_err(db_err)?
    .ok_or_else(|| format!("Comment not found: C{id}"))
    .and_then(comment_from_columns)
}

pub(crate) fn create_comment_impl(
    conn: &Connection,
    repo_path: &str,
    base: &str,
    head: &str,
    mode: DiffMode,
    comment: NewComment,
) -> Result<Comment, String> {
    if comment.body.trim().is_empty() {
        return Err("Comment text cannot be empty".to_owned());
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
                _ => return Err("Invalid line range for comment".to_owned()),
            };
            let file_diff = diff::get_file_diff(
                repo_path.to_owned(),
                base.to_owned(),
                head.to_owned(),
                mode,
                path.clone(),
            )?;
            let anchor = extract_anchor(&file_diff, side, start, end)?;
            (Some(path), Some(side), Some(start), Some(end), Some(anchor))
        }
    };

    let repo = open_git_repo(repo_path)?;
    let commit_sha = diff::resolve_commit(&repo, head)?.id().to_string();
    let anchor_json = anchor
        .map(|a| serde_json::to_string(&a).map_err(|e| format!("Failed to encode anchor: {e}")))
        .transpose()?;

    conn.execute(
        "INSERT INTO comments (review_id, level, file_path, side, start_line, end_line,
                               code_anchor, commit_sha, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
        ),
    )
    .map_err(db_err)?;
    get_comment(conn, conn.last_insert_rowid())
}

pub(crate) fn update_comment_impl(
    conn: &Connection,
    comment_id: i64,
    body: &str,
) -> Result<Comment, String> {
    if body.trim().is_empty() {
        return Err("Comment text cannot be empty".to_owned());
    }
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

pub(crate) fn delete_comment_impl(conn: &Connection, comment_id: i64) -> Result<(), String> {
    let changed = conn
        .execute("DELETE FROM comments WHERE id = ?1", [comment_id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err(format!("Comment not found: C{comment_id}"));
    }
    Ok(())
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
) -> Result<CodeAnchor, String> {
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
            return Err("A comment selection cannot cross hunk boundaries".to_owned());
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
    Err(format!(
        "No diff lines at {}:{start}-{end} ({}) to comment on",
        diff.path,
        side.as_str()
    ))
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
            body: body.to_owned(),
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
            body: "needs work".to_owned(),
        }
    }

    fn create(conn: &Connection, fixture: &FixtureRepo, comment: NewComment) -> Comment {
        create_comment_impl(conn, &fixture.path(), "main", "feature", DiffMode::Committed, comment)
            .unwrap()
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
            &fixture.path(),
            "main",
            "feature",
            DiffMode::Committed,
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
            create_comment_impl(&conn, &fixture.path(), "main", "feature", DiffMode::Committed, c)
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
}
