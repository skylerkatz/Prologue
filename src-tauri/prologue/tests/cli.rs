//! End-to-end tests of the prologue binary: a real process against a temp
//! database and fixture repository. Covers run()'s dispatch — every
//! subcommand, the read-only vs write connection selection, the
//! not-inside-a-repository note, and show's degraded path.

use prologue_core::diff::{DiffMode, DiffSpec};
use prologue_core::export::{export_review_impl, ExportFormat};
use prologue_core::review::{self, CommentLevel, CommentSide, NewComment, Review};
use prologue_core::testutil::{open_test_db, FixtureRepo};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn prologue(db: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prologue"))
        .arg("--db")
        .arg(db)
        .args(args)
        .current_dir(cwd)
        // --author reads PROLOGUE_AUTHOR from the environment; the tests
        // must see the built-in default regardless of the outer shell.
        .env_remove("PROLOGUE_AUTHOR")
        .output()
        .expect("prologue binary runs")
}

fn out_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn err_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn line_comment(review_id: i64, start: u32, end: u32, body: &str) -> NewComment {
    NewComment {
        review_id,
        level: CommentLevel::Line,
        file_path: Some("code.txt".to_owned()),
        side: Some(CommentSide::New),
        start_line: Some(start),
        end_line: Some(end),
        parent_id: None,
        body: body.to_owned(),
        author: None,
    }
}

/// A database holding one review of the standard fixture's feature branch,
/// with one line comment (C1) on code.txt:6-7.
fn seeded(dir: &tempfile::TempDir) -> (PathBuf, FixtureRepo, Review, i64) {
    let conn = open_test_db(dir);
    let fixture = FixtureRepo::standard_review_fixture();
    let review =
        review::open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
            .unwrap();
    let comment = review::create_comment_impl(
        &conn,
        &DiffSpec::from(&review),
        line_comment(review.id, 6, 7, "tighten this"),
    )
    .unwrap();
    (dir.path().join("reviews.db"), fixture, review, comment.id)
}

#[test]
fn reviews_renders_the_table_scoped_to_the_cwd_repository() {
    let dir = tempfile::tempdir().unwrap();
    let (db, fixture, review, _) = seeded(&dir);

    let output = prologue(&db, Path::new(&fixture.path()), &["reviews"]);
    assert!(output.status.success(), "{}", err_str(&output));
    let table = out_str(&output);
    assert!(table.contains("ID"), "{table}");
    assert!(table.contains(&review.id.to_string()), "{table}");
    assert!(table.contains("feature"), "{table}");
    assert!(table.contains("committed"), "{table}");
    // In-repo invocations carry no fallback note.
    assert!(!err_str(&output).contains("note:"), "{}", err_str(&output));
}

#[test]
fn reviews_outside_a_repository_notes_the_fallback_and_lists_all() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _fixture, review, _) = seeded(&dir);
    let outside = tempfile::tempdir().unwrap();

    let output = prologue(&db, outside.path(), &["reviews"]);
    assert!(output.status.success(), "{}", err_str(&output));
    assert!(
        err_str(&output).contains("not inside a git repository"),
        "{}",
        err_str(&output)
    );
    assert!(out_str(&output).contains(&review.id.to_string()), "{}", out_str(&output));
}

#[test]
fn reviews_json_is_machine_readable() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _fixture, review, _) = seeded(&dir);
    let outside = tempfile::tempdir().unwrap();

    let output = prologue(&db, outside.path(), &["reviews", "--all", "--json"]);
    assert!(output.status.success(), "{}", err_str(&output));
    let parsed: serde_json::Value = serde_json::from_str(out_str(&output).trim()).unwrap();
    assert_eq!(parsed[0]["id"], serde_json::json!(review.id));
    assert_eq!(parsed[0]["branch"], "feature");
    assert_eq!(parsed[0]["status"], "active");
}

#[test]
fn show_prints_threads_with_anchors() {
    let dir = tempfile::tempdir().unwrap();
    let (db, fixture, review, comment_id) = seeded(&dir);

    let output = prologue(&db, Path::new(&fixture.path()), &["show", &review.id.to_string()]);
    assert!(output.status.success(), "{}", err_str(&output));
    let text = out_str(&output);
    assert!(text.contains("Review #"), "{text}");
    assert!(text.contains(&format!("C{comment_id} [open] code.txt:6-7")), "{text}");
    assert!(text.contains("> beta 6a"), "{text}");
    assert!(text.contains("tighten this"), "{text}");
}

#[test]
fn show_file_diff_prints_current_hunk_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let (db, fixture, review, _) = seeded(&dir);

    let output = prologue(
        &db,
        Path::new(&fixture.path()),
        &["show", &review.id.to_string(), "--file", "code.txt", "--diff"],
    );
    assert!(output.status.success(), "{}", err_str(&output));
    let text = out_str(&output);
    assert!(text.contains("@@"), "{text}");
    assert!(text.contains("- alpha 6"), "{text}");
    assert!(text.contains("+ beta 6a"), "{text}");
}

/// Deleting the reviewed branch makes the diff uncomputable; show must
/// degrade to stored positions with a warning, not fail.
#[test]
fn show_degrades_to_stored_positions_when_the_branch_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_test_db(&dir);
    let db = dir.path().join("reviews.db");
    let fixture = FixtureRepo::standard_review_fixture();
    // Review a side branch at the feature tip, then delete it.
    let head = fixture.repo.head().unwrap().peel_to_commit().unwrap();
    fixture.repo.branch("doomed", &head, false).unwrap();
    let review =
        review::open_review_impl(&conn, &fixture.path(), "doomed", "main", DiffMode::Committed)
            .unwrap();
    review::create_comment_impl(
        &conn,
        &DiffSpec::from(&review),
        line_comment(review.id, 6, 7, "sank with the branch"),
    )
    .unwrap();
    fixture.delete_branch("doomed");

    let output = prologue(&db, Path::new(&fixture.path()), &["show", &review.id.to_string()]);
    assert!(output.status.success(), "{}", err_str(&output));
    assert!(
        err_str(&output).contains("could not recompute anchors"),
        "{}",
        err_str(&output)
    );
    let text = out_str(&output);
    assert!(text.contains("line numbers are stored values"), "{text}");
    assert!(text.contains("code.txt:6-7"), "{text}");

    // The JSON shape flags the degradation too.
    let output = prologue(
        &db,
        Path::new(&fixture.path()),
        &["show", &review.id.to_string(), "--json"],
    );
    assert!(output.status.success(), "{}", err_str(&output));
    let parsed: serde_json::Value = serde_json::from_str(out_str(&output).trim()).unwrap();
    assert_eq!(parsed["anchorsCurrent"], serde_json::json!(false));
}

#[test]
fn export_writes_the_exact_clipboard_payload() {
    let dir = tempfile::tempdir().unwrap();
    let (db, fixture, review, _) = seeded(&dir);

    let output = prologue(
        &db,
        Path::new(&fixture.path()),
        &["export", &review.id.to_string(), "--format", "md"],
    );
    assert!(output.status.success(), "{}", err_str(&output));

    // Byte-identical to the read-only core export the app's clipboard uses.
    let conn = open_test_db(&dir);
    let expected = export_review_impl(
        &conn,
        &DiffSpec::from(&review),
        review.id,
        ExportFormat::Markdown,
        false,
    )
    .unwrap();
    assert!(expected.starts_with("# Code review — feature vs main"), "{expected}");
    assert_eq!(out_str(&output), expected);
}

/// The Comment arm must select the writable connection — the read-only one
/// refuses INSERTs at the SQLite level (covered by the CLI db unit tests).
#[test]
fn comment_uses_the_write_connection_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let (db, fixture, review, _) = seeded(&dir);

    let output = prologue(
        &db,
        Path::new(&fixture.path()),
        &["comment", &review.id.to_string(), "--body", "from the cli", "--json"],
    );
    assert!(output.status.success(), "{}", err_str(&output));
    let parsed: serde_json::Value = serde_json::from_str(out_str(&output).trim()).unwrap();
    assert_eq!(parsed["reviewId"], serde_json::json!(review.id));
    assert_eq!(parsed["level"], "review");
    assert_eq!(parsed["author"], "agent", "default author when PROLOGUE_AUTHOR is unset");

    // The row really landed on disk.
    let conn = open_test_db(&dir);
    let stored = review::get_comment(&conn, parsed["id"].as_i64().unwrap()).unwrap();
    assert_eq!(stored.body, "from the cli");
}

#[test]
fn reply_attaches_to_the_thread_root() {
    let dir = tempfile::tempdir().unwrap();
    let (db, fixture, review, comment_id) = seeded(&dir);

    let output = prologue(
        &db,
        Path::new(&fixture.path()),
        &["reply", &format!("C{comment_id}"), "--body", "done in abc123"],
    );
    assert!(output.status.success(), "{}", err_str(&output));
    let text = out_str(&output);
    assert!(text.contains(&format!("reply to thread C{comment_id}")), "{text}");
    assert!(text.contains("author agent"), "{text}");

    let conn = open_test_db(&dir);
    let comments = review::list_comments_impl(&conn, review.id).unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1].parent_id, Some(comment_id));
    assert_eq!(comments[1].body, "done in abc123");
}

#[test]
fn a_missing_database_is_a_clean_error() {
    let dir = tempfile::tempdir().unwrap();

    let output = prologue(&dir.path().join("nope.db"), dir.path(), &["reviews"]);
    assert!(!output.status.success());
    assert!(
        err_str(&output).contains("error: No reviews database"),
        "{}",
        err_str(&output)
    );
}
