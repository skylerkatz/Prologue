//! Review lifecycle (open/resume/archive) and per-file reviewed marks.
//! Comments and threads live in [`crate::comment`], anchors in
//! [`crate::anchor`].

use git2::{BranchType, Repository};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

use crate::db::{db_err, NOW};
use crate::diff::{self, DiffMode, DiffSpec};

// The comment and anchor halves of the original review module moved out;
// these re-exports keep the public paths external callers use
// (`prologue_core::review::…`) stable.
pub use crate::anchor::CodeAnchor;
pub use crate::comment::{
    comment_count, create_comment_impl, delete_comment_impl, get_comment, list_comments_impl,
    reanchor_comments_impl, resolve_threads, resolve_threads_with, stored_threads,
    try_create_comment, update_comment_impl, update_comment_state_impl, Comment, CommentLevel,
    CommentState, NewComment, ReanchorResult, Thread,
};
pub use crate::diff::CommentSide;

#[derive(Serialize, Debug, Clone)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// SQLite rowid — far below 2^53, a plain JS number on the wire.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    pub repo_path: String,
    pub branch: String,
    pub base_ref: String,
    pub mode: DiffMode,
    pub status: ReviewStatus,
    pub created_at: String,
    pub updated_at: String,
}

/// A review's lifecycle state. Serializes as the same lowercase strings the
/// raw `status` column carried, so the IPC/CLI wire shape is unchanged.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ReviewStatus {
    Active,
    Archived,
}

impl ReviewStatus {
    /// Stable text form: the reviews database value, also what CLI output
    /// prints.
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::Active => "active",
            ReviewStatus::Archived => "archived",
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "active" => Ok(ReviewStatus::Active),
            "archived" => Ok(ReviewStatus::Archived),
            other => Err(format!("Unknown review status: {other}")),
        }
    }
}

impl std::fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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

/// One per-file reviewed mark. `fingerprint` is the [`FileSummary`] content
/// identity at mark time; the frontend compares it with the current summary's
/// value — equal means "reviewed", different means "changed since review".
///
/// [`FileSummary`]: crate::diff::FileSummary
#[derive(Serialize, Debug, Clone)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct OpenReviewResult {
    pub review: Option<Review>,
    pub branch_merged: bool,
}

/// The canonical spelling of a repo path — symlinks resolved, so every
/// spelling of one directory (macOS /tmp vs /private/tmp) keys one review.
/// Uncanonicalizable paths (repo deleted since) keep the caller's spelling,
/// so archived reviews stay addressable.
fn canonical_repo_path(repo_path: &str) -> String {
    Path::new(repo_path)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| repo_path.to_owned())
}

pub fn open_review_impl(
    conn: &Connection,
    repo_path: &str,
    branch: &str,
    base_ref: &str,
    mode: DiffMode,
) -> Result<Review, String> {
    // Reviews key on the canonical path, but rows written before
    // canonicalization may carry the caller's spelling — match either, and
    // refresh the stored path on every open so legacy rows converge.
    let canonical = canonical_repo_path(repo_path);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path IN (?1, ?2) AND branch = ?3 AND status = 'active'
             ORDER BY (repo_path = ?1) DESC, id",
            [canonical.as_str(), repo_path, branch],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;

    let id = match existing {
        Some(id) => {
            conn.execute(
                &format!(
                    "UPDATE reviews
                     SET repo_path = ?1, base_ref = ?2, mode = ?3, updated_at = {NOW}
                     WHERE id = ?4"
                ),
                (&canonical, base_ref, mode.as_str(), id),
            )
            .map_err(db_err)?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO reviews (repo_path, branch, base_ref, mode)
                 VALUES (?1, ?2, ?3, ?4)",
                (&canonical, branch, base_ref, mode.as_str()),
            )
            .map_err(db_err)?;
            conn.last_insert_rowid()
        }
    };
    get_review(conn, id)
}

/// The review row for `id`, or `None` when no such review exists — for
/// callers that phrase "not found" themselves.
pub fn find_review(conn: &Connection, id: i64) -> Result<Option<Review>, String> {
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
    .map(|(id, repo_path, branch, base_ref, mode, status, created_at, updated_at)| {
        Ok(Review {
            id,
            repo_path,
            branch,
            base_ref,
            mode: DiffMode::parse(&mode)?,
            status: ReviewStatus::parse(&status)?,
            created_at,
            updated_at,
        })
    })
    .transpose()
}

/// The review row for `id`; a missing review is an error.
pub fn get_review(conn: &Connection, id: i64) -> Result<Review, String> {
    find_review(conn, id)?.ok_or_else(|| format!("Review not found: {id}"))
}

/// Archived reviews are read-only: every comment or reviewed-file mutation
/// checks its review's status first.
pub(crate) fn ensure_review_active(conn: &Connection, review_id: i64) -> Result<(), String> {
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

pub(crate) fn archive_review(conn: &Connection, id: i64) -> Result<(), String> {
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
    let canonical = canonical_repo_path(repo_path);
    let active: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path IN (?1, ?2) AND branch = ?3 AND status = 'active'
             ORDER BY (repo_path = ?1) DESC, id",
            [canonical.as_str(), repo_path, branch],
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
             WHERE repo_path IN (?1, ?2) AND branch = ?3 AND status = 'archived'
             ORDER BY id DESC LIMIT 1",
            [canonical.as_str(), repo_path, branch],
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
    let canonical = canonical_repo_path(repo_path);
    let mut stmt = conn
        .prepare(
            "SELECT id, branch, base_ref FROM reviews
             WHERE repo_path IN (?1, ?2) AND status = 'active' ORDER BY id",
        )
        .map_err(db_err)?;
    let active: Vec<(i64, String, String)> = stmt
        .query_map([canonical.as_str(), repo_path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ArchivedReview {
    #[serde(flatten)]
    pub review: Review,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub comment_count: i64,
}

pub fn list_archived_reviews_impl(
    conn: &Connection,
    repo_path: &str,
) -> Result<Vec<ArchivedReview>, String> {
    let canonical = canonical_repo_path(repo_path);
    let mut stmt = conn
        .prepare(
            "SELECT id, (SELECT COUNT(*) FROM comments c WHERE c.review_id = reviews.id)
             FROM reviews WHERE repo_path IN (?1, ?2) AND status = 'archived'
             ORDER BY updated_at DESC, id DESC",
        )
        .map_err(db_err)?;
    let rows: Vec<(i64, i64)> = stmt
        .query_map([canonical.as_str(), repo_path], |r| Ok((r.get(0)?, r.get(1)?)))
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
    let canonical = canonical_repo_path(repo_path);
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM reviews
             WHERE repo_path IN (?1, ?2) AND branch = ?3 AND status = 'active'
             ORDER BY (repo_path = ?1) DESC, id",
            [canonical.as_str(), repo_path, branch],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{open_test_db as test_db, FixtureRepo};

    fn fixture() -> FixtureRepo {
        FixtureRepo::standard_review_fixture()
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
        assert_eq!(created.status, ReviewStatus::Active);
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

    /// The /tmp vs /private/tmp situation, reproduced deterministically: a
    /// symlinked spelling of the repo and the real one must key one review.
    #[test]
    fn alternate_spellings_of_the_repo_path_resolve_to_the_same_review() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let link = dir.path().join("repo-link");
        std::os::unix::fs::symlink(fixture.dir.path(), &link).unwrap();
        let link_path = link.to_string_lossy().into_owned();

        let via_real =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let via_link =
            open_review_impl(&conn, &link_path, "feature", "main", DiffMode::Committed).unwrap();
        assert_eq!(via_link.id, via_real.id, "one review per physical repo+branch");
        // Both spellings store the same canonical path.
        assert_eq!(via_link.repo_path, via_real.repo_path);

        // Reads resolve through either spelling as well.
        let by_link = find_active_review_impl(&conn, &link_path, "feature").unwrap().unwrap();
        assert_eq!(by_link.id, via_real.id);
        let by_real =
            find_active_review_impl(&conn, &fixture.path(), "feature").unwrap().unwrap();
        assert_eq!(by_real.id, via_real.id);
    }

    /// Rows written before canonicalization carry whatever spelling the
    /// caller used; opening through that spelling must resume them (not
    /// duplicate) and converge the stored path.
    #[test]
    fn legacy_uncanonical_rows_are_adopted_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let link = dir.path().join("repo-link");
        std::os::unix::fs::symlink(fixture.dir.path(), &link).unwrap();
        let link_path = link.to_string_lossy().into_owned();

        // A pre-canonicalization row, stored under the symlinked spelling.
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES (?1, 'feature', 'main', 'committed')",
            [link_path.as_str()],
        )
        .unwrap();

        let resumed =
            open_review_impl(&conn, &link_path, "feature", "main", DiffMode::Committed).unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM reviews", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "the legacy row is resumed, not duplicated");
        assert_eq!(resumed.repo_path, canonical_repo_path(&link_path), "path converged");

        // After adoption the real spelling reaches the same review too.
        let via_real =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        assert_eq!(via_real.id, resumed.id);
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
        assert_eq!(archived[0].status, ReviewStatus::Archived);

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
        assert_eq!(review.status, ReviewStatus::Active);

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
        assert_eq!(archived.status, ReviewStatus::Archived);

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
        assert!(all[..2].iter().all(|r| r.status == ReviewStatus::Active));
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
