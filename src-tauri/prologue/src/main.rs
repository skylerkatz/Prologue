//! `prologue` — command-line access to Prologue code reviews:
//! read anything, add comments and replies. Opens the database, acts, and
//! exits; never a daemon. Thread lifecycle (resolve/dismiss/reopen/delete)
//! deliberately does not exist here — closing a thread is the reviewer's
//! act, in the app.

mod db;
mod guide;
mod resolve;
mod show;

use clap::{Parser, Subcommand, ValueEnum};
use prologue_core::diff::DiffSpec;
use prologue_core::error::CoreError;
use prologue_core::export::{self, ExportFormat};
use prologue_core::review::{self, Comment, CommentLevel, CommentSide, NewComment, Review};
use prologue_core::rusqlite::Connection;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "prologue", version, about = "Command-line access to Prologue code reviews")]
struct Cli {
    /// Path to reviews.db (default: the Prologue app's database)
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List reviews, active first (scoped to the cwd's repository by default)
    Reviews {
        /// Only reviews of this repository (a path, or a name like "my-app")
        #[arg(long, value_name = "REPO", conflicts_with = "all")]
        repo: Option<String>,
        /// Every repository's reviews, not just the cwd's
        #[arg(long)]
        all: bool,
        /// Include archived reviews
        #[arg(long)]
        archived: bool,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// Show a review's threads: roots with nested replies, states, anchors,
    /// and orphan flags
    Show {
        /// Review id or repo@branch (default: inferred from the cwd's
        /// repository and checked-out branch)
        review: Option<String>,
        /// Only threads on this file; with --diff, the file whose hunks to print
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// Print the file's current hunks with old/new line numbers instead
        /// of threads (valid coordinates for line comments)
        #[arg(long, requires = "file")]
        diff: bool,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// Print the review's stored guide: ordered sections grouping the
    /// changed files, with per-file status and line counts (read-only;
    /// guides are generated in the Prologue app)
    Guide {
        /// Review id or repo@branch (default: inferred from the cwd's
        /// repository and checked-out branch)
        review: Option<String>,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// Print a review's open comments in an export format (byte-identical
    /// to the app's clipboard export)
    Export {
        /// Review id or repo@branch (default: inferred from the cwd's
        /// repository and checked-out branch)
        review: Option<String>,
        #[arg(long, value_enum)]
        format: FormatArg,
    },
    /// Add a comment to a review: review-level by default, file-level with
    /// --file, line-level with --file and --line
    Comment {
        /// Review id or repo@branch (default: inferred from the cwd's
        /// repository and checked-out branch)
        review: Option<String>,
        /// File the comment is about (as shown in the diff)
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// Line or range on that file's current diff, e.g. 42 or 42-45
        /// (valid coordinates: `prologue show --file PATH --diff`)
        #[arg(long, value_name = "N[-M]", requires = "file")]
        line: Option<String>,
        /// Which side of the diff the lines are on
        #[arg(long, value_enum, default_value = "new", requires = "line")]
        side: SideArg,
        /// The comment text
        #[arg(long)]
        body: String,
        /// Who is writing (defaults to the PROLOGUE_AUTHOR environment
        /// variable, then 'agent'; the app badges non-reviewer authors)
        #[arg(long, env = "PROLOGUE_AUTHOR", default_value = "agent")]
        author: String,
        /// Print the created comment as JSON
        #[arg(long)]
        json: bool,
    },
    /// Reply to a comment's thread (replies attach to the thread root;
    /// closed threads refuse replies)
    Reply {
        /// Comment id, e.g. C12 or 12 — any comment in the thread works
        comment: String,
        /// The reply text
        #[arg(long)]
        body: String,
        /// Who is writing (defaults to the PROLOGUE_AUTHOR environment
        /// variable, then 'agent'; the app badges non-reviewer authors)
        #[arg(long, env = "PROLOGUE_AUTHOR", default_value = "agent")]
        author: String,
        /// Print the created reply as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Thread-lifecycle verbs that deliberately do not exist as subcommands.
/// They are intercepted before clap's usual unknown-subcommand error so the
/// caller learns why, not just that the verb is unknown.
const LIFECYCLE_VERBS: &[&str] = &["resolve", "dismiss", "reopen", "delete", "close"];

#[derive(ValueEnum, Clone, Copy)]
enum SideArg {
    Old,
    New,
}

impl From<SideArg> for CommentSide {
    fn from(arg: SideArg) -> Self {
        match arg {
            SideArg::Old => CommentSide::Old,
            SideArg::New => CommentSide::New,
        }
    }
}

#[derive(ValueEnum, Clone, Copy)]
enum FormatArg {
    #[value(alias = "markdown")]
    Md,
    Json,
    #[value(alias = "prompt-markdown")]
    PromptMd,
    PromptJson,
}

impl From<FormatArg> for ExportFormat {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Md => ExportFormat::Markdown,
            FormatArg::Json => ExportFormat::Json,
            FormatArg::PromptMd => ExportFormat::PromptMarkdown,
            FormatArg::PromptJson => ExportFormat::PromptJson,
        }
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if let Some(verb) = attempted_lifecycle_verb(&err) {
                eprintln!("{}", lifecycle_error(&verb));
                std::process::exit(1);
            }
            err.exit();
        }
    };
    if let Err(message) = run(cli) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

/// The invalid subcommand the user tried, when it is a lifecycle verb.
fn attempted_lifecycle_verb(err: &clap::Error) -> Option<String> {
    if err.kind() != clap::error::ErrorKind::InvalidSubcommand {
        return None;
    }
    match err.get(clap::error::ContextKind::InvalidSubcommand) {
        Some(clap::error::ContextValue::String(sub)) if LIFECYCLE_VERBS.contains(&sub.as_str()) => {
            Some(sub.clone())
        }
        _ => None,
    }
}

fn lifecycle_error(verb: &str) -> String {
    format!(
        "error: `{verb}` is not a prologue command — resolving, dismissing, reopening, \
         and deleting review threads are manual actions in the Prologue app, on purpose. \
         prologue can read reviews and add comments or replies; say what you did with \
         `prologue reply C12 --body \"...\"` and let the reviewer close the thread."
    )
}

fn run(cli: Cli) -> Result<(), String> {
    let db_path = match cli.db {
        Some(path) => path,
        None => db::default_db_path()?,
    };
    // Only the two write commands get a writable connection; every read
    // stays on a query_only one.
    let conn = match &cli.command {
        Command::Comment { .. } | Command::Reply { .. } => {
            db::open_reviews_db_for_write(&db_path)?
        }
        _ => db::open_reviews_db(&db_path)?,
    };
    let cwd = std::env::current_dir()
        .map_err(|e| format!("Cannot determine the working directory: {e}"))?;

    match cli.command {
        Command::Reviews { repo, all, archived, json } => {
            let cwd_repo =
                if all || repo.is_some() { None } else { resolve::containing_workdir(&cwd) };
            // A cwd outside any repository falls back to the unscoped list:
            // this command is the discovery escape hatch other errors point at.
            if !json && !all && repo.is_none() && cwd_repo.is_none() {
                eprintln!("note: not inside a git repository — listing every repository's reviews");
            }
            let reviews: Vec<Review> = review::list_reviews_impl(&conn, None, archived)?
                .into_iter()
                .filter(|r| resolve::review_in_scope(r, repo.as_deref(), cwd_repo.as_deref()))
                .collect();
            if json {
                println!("{}", to_json(&reviews)?);
            } else if reviews.is_empty() {
                if repo.is_some() || cwd_repo.is_some() {
                    println!(
                        "No reviews found for this repository — `prologue reviews --all` \
                         lists every repository"
                    );
                } else {
                    println!("No reviews found");
                }
            } else {
                print!("{}", reviews_table(&conn, &reviews)?);
            }
        }
        Command::Show { review, file, diff, json } => {
            let review = resolve::resolve_review(&conn, review.as_deref(), &cwd)?;
            if diff {
                // `requires` guarantees --file is present.
                let file_diff = show::file_diff(&review, &file.unwrap())?;
                if json {
                    println!("{}", to_json(&file_diff)?);
                } else {
                    print!("{}", show::render_file_diff_text(&file_diff));
                }
            } else {
                let mut data = show::show_data(&conn, review)?;
                if let Some(path) = &file {
                    data.threads.retain(|t| t.root.file_path.as_deref() == Some(path));
                }
                if json {
                    println!("{}", to_json(&data)?);
                } else {
                    print!("{}", show::render_text(&data));
                }
            }
        }
        Command::Guide { review, json } => {
            let review = resolve::resolve_review(&conn, review.as_deref(), &cwd)?;
            let guide = guide::find_guide(&conn, &review)?;
            if json {
                println!("{}", to_json(&guide)?);
            } else {
                let summary = guide::current_summary(&review);
                print!("{}", guide::render_text(&review, &guide, summary.as_ref()));
            }
        }
        Command::Export { review, format } => {
            let review = resolve::resolve_review(&conn, review.as_deref(), &cwd)?;
            let text = export::export_review_impl(
                &conn,
                &DiffSpec::from(&review),
                review.id,
                format.into(),
                false,
            )?;
            // Byte-for-byte what the app puts on the clipboard — no added
            // trailing newline.
            std::io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| format!("Failed to write output: {e}"))?;
        }
        Command::Comment { review, file, line, side, body, author, json } => {
            let review = resolve::resolve_review(&conn, review.as_deref(), &cwd)?;
            let (level, start_line, end_line, side) = match (&file, &line) {
                (Some(_), Some(range)) => {
                    let (start, end) = parse_line_range(range)?;
                    (CommentLevel::Line, Some(start), Some(end), Some(side.into()))
                }
                (Some(_), None) => (CommentLevel::File, None, None, None),
                // clap's `requires` keeps --line from appearing without --file.
                (None, _) => (CommentLevel::Review, None, None, None),
            };
            let created = review::try_create_comment(
                &conn,
                &DiffSpec::from(&review),
                NewComment {
                    review_id: review.id,
                    level,
                    file_path: file.clone(),
                    side,
                    start_line,
                    end_line,
                    parent_id: None,
                    body,
                    author: Some(author),
                },
            )
            .map_err(|e| with_anchor_hint(e, file.as_deref()))?;
            print_created(&created, json)?;
        }
        Command::Reply { comment, body, author, json } => {
            let parent_id = parse_comment_id(&comment)?;
            let review = review_of_comment(&conn, parent_id)?;
            let created = review::create_comment_impl(
                &conn,
                &DiffSpec::from(&review),
                NewComment {
                    review_id: review.id,
                    level: CommentLevel::Review, // ignored: replies inherit the root's level
                    file_path: None,
                    side: None,
                    start_line: None,
                    end_line: None,
                    parent_id: Some(parent_id),
                    body,
                    author: Some(author),
                },
            )?;
            print_created(&created, json)?;
        }
    }
    Ok(())
}

/// `C12` or `12` → 12.
fn parse_comment_id(arg: &str) -> Result<i64, String> {
    arg.strip_prefix(['C', 'c'])
        .unwrap_or(arg)
        .parse()
        .map_err(|_| format!("Cannot parse comment '{arg}' — expected an id like C12 or 12"))
}

/// `42` or `42-45` → (42, 45).
fn parse_line_range(arg: &str) -> Result<(u32, u32), String> {
    let bad = || format!("Cannot parse line range '{arg}' — expected a line like 42 or a range like 42-45");
    let (start, end) = match arg.split_once('-') {
        Some((s, e)) => (s, e),
        None => (arg, arg),
    };
    let (start, end): (u32, u32) =
        (start.parse().map_err(|_| bad())?, end.parse().map_err(|_| bad())?);
    if start < 1 || start > end {
        return Err(format!("Invalid line range '{arg}' — start must be ≥ 1 and ≤ end"));
    }
    Ok((start, end))
}

/// The review a comment belongs to (any status — core rejects writes to
/// archived reviews with its own message).
fn review_of_comment(conn: &Connection, comment_id: i64) -> Result<Review, String> {
    let comment = review::get_comment(conn, comment_id)?;
    review::get_review(conn, comment.review_id)
}

/// Anchor-capture failures usually mean the caller's view of the diff is
/// stale; point at the re-read helper.
fn with_anchor_hint(err: CoreError, file: Option<&str>) -> String {
    let anchor_error =
        matches!(err, CoreError::SelectionCrossesHunks | CoreError::NoDiffLines { .. });
    match (anchor_error, file) {
        (true, Some(path)) => format!(
            "{err}\nThe diff may have changed since you read it — re-read current line \
             numbers with `prologue show --file {path} --diff` and try again"
        ),
        _ => err.to_string(),
    }
}

fn print_created(comment: &Comment, json: bool) -> Result<(), String> {
    if json {
        println!("{}", to_json(comment)?);
        return Ok(());
    }
    let place = match (comment.parent_id, comment.level) {
        (Some(root), _) => format!("reply to thread C{root}"),
        (None, CommentLevel::Review) => "review-level".to_owned(),
        (None, CommentLevel::File) => {
            format!("on file {}", comment.file_path.as_deref().unwrap_or("?"))
        }
        (None, CommentLevel::Line) => format!(
            "on {}:{}-{} ({} side)",
            comment.file_path.as_deref().unwrap_or("?"),
            comment.start_line.unwrap_or(0),
            comment.end_line.unwrap_or(0),
            comment.side.unwrap_or(CommentSide::New).as_str(),
        ),
    };
    let head = comment.commit_sha.get(..7).unwrap_or(&comment.commit_sha);
    println!(
        "C{} created — {place}, review {}, author {}, head {head}",
        comment.id, comment.review_id, comment.author
    );
    Ok(())
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    // Compact on purpose: the output is read by agents, where pretty
    // printing only spends tokens on whitespace.
    serde_json::to_string(value).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn reviews_table(conn: &Connection, reviews: &[Review]) -> Result<String, String> {
    let mut rows = vec![[
        "ID".to_owned(),
        "STATUS".to_owned(),
        "REPO".to_owned(),
        "BRANCH".to_owned(),
        "BASE".to_owned(),
        "MODE".to_owned(),
        "COMMENTS".to_owned(),
        "UPDATED".to_owned(),
    ]];
    for review in reviews {
        let comments = review::comment_count(conn, review.id)?;
        rows.push([
            review.id.to_string(),
            review.status.to_string(),
            review.repo_path.clone(),
            review.branch.clone(),
            review.base_ref.clone(),
            review.mode.as_str().to_owned(),
            comments.to_string(),
            review.updated_at.clone(),
        ]);
    }
    let widths: Vec<usize> = (0..rows[0].len())
        .map(|col| rows.iter().map(|row| row[col].len()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for row in &rows {
        let mut line = String::new();
        for (cell, width) in row.iter().zip(&widths) {
            write!(line, "{cell:<width$}  ").unwrap();
        }
        writeln!(out, "{}", line.trim_end()).unwrap();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_surface_parses_as_documented() {
        Cli::try_parse_from(["prologue", "reviews"]).unwrap();
        Cli::try_parse_from(["prologue", "reviews", "--repo", "my-app", "--archived", "--json"])
            .unwrap();
        Cli::try_parse_from(["prologue", "reviews", "--all"]).unwrap();
        Cli::try_parse_from(["prologue", "reviews", "--all", "--archived", "--json"]).unwrap();
        // --all and --repo are competing scopes.
        assert!(Cli::try_parse_from(["prologue", "reviews", "--repo", "my-app", "--all"]).is_err());
        Cli::try_parse_from(["prologue", "show"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "7", "--json"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "my-app@main"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "--file", "src/a.rs", "--diff"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "--file", "src/a.rs"]).unwrap();
        // --diff needs --file.
        assert!(Cli::try_parse_from(["prologue", "show", "--diff"]).is_err());
        // --db is global.
        Cli::try_parse_from(["prologue", "show", "--db", "/tmp/x.db"]).unwrap();

        Cli::try_parse_from(["prologue", "guide"]).unwrap();
        Cli::try_parse_from(["prologue", "guide", "7", "--json"]).unwrap();
        Cli::try_parse_from(["prologue", "guide", "my-app@main"]).unwrap();
        // Read-only on purpose: no generation or refresh flags exist.
        assert!(Cli::try_parse_from(["prologue", "guide", "--generate"]).is_err());
        assert!(Cli::try_parse_from(["prologue", "guide", "--regenerate"]).is_err());

        for format in ["md", "markdown", "json", "prompt-md", "prompt-markdown", "prompt-json"] {
            Cli::try_parse_from(["prologue", "export", "7", "--format", format]).unwrap();
        }
        assert!(Cli::try_parse_from(["prologue", "export", "7", "--format", "xml"]).is_err());
        assert!(Cli::try_parse_from(["prologue", "export", "7"]).is_err());

        Cli::try_parse_from(["prologue", "comment", "--body", "overall note"]).unwrap();
        Cli::try_parse_from(["prologue", "comment", "7", "--file", "src/a.rs", "--body", "x"])
            .unwrap();
        Cli::try_parse_from([
            "prologue", "comment", "--file", "src/a.rs", "--line", "42-45", "--side", "old",
            "--body", "x", "--author", "skyler", "--json",
        ])
        .unwrap();
        // --line needs --file; --side needs --line; --body is required.
        assert!(Cli::try_parse_from(["prologue", "comment", "--line", "42", "--body", "x"]).is_err());
        assert!(
            Cli::try_parse_from(["prologue", "comment", "--file", "a", "--side", "old", "--body", "x"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["prologue", "comment"]).is_err());

        Cli::try_parse_from(["prologue", "reply", "C12", "--body", "done"]).unwrap();
        Cli::try_parse_from(["prologue", "reply", "12", "--body", "done", "--author", "agent"])
            .unwrap();
        assert!(Cli::try_parse_from(["prologue", "reply", "--body", "done"]).is_err());
        assert!(Cli::try_parse_from(["prologue", "reply", "C12"]).is_err());
    }

    #[test]
    fn author_defaults_to_agent() {
        let cli = Cli::try_parse_from(["prologue", "reply", "C1", "--body", "x"]).unwrap();
        // PROLOGUE_AUTHOR would override this default; the test environment
        // does not set it.
        match cli.command {
            Command::Reply { author, .. } => assert_eq!(author, "agent"),
            _ => panic!("parsed as the wrong command"),
        }
        let cli =
            Cli::try_parse_from(["prologue", "comment", "--body", "x", "--author", "skyler"])
                .unwrap();
        match cli.command {
            Command::Comment { author, .. } => assert_eq!(author, "skyler"),
            _ => panic!("parsed as the wrong command"),
        }
    }

    #[test]
    fn comment_ids_and_line_ranges_parse() {
        assert_eq!(parse_comment_id("C12").unwrap(), 12);
        assert_eq!(parse_comment_id("c12").unwrap(), 12);
        assert_eq!(parse_comment_id("12").unwrap(), 12);
        assert!(parse_comment_id("C").is_err());
        assert!(parse_comment_id("twelve").is_err());

        assert_eq!(parse_line_range("42").unwrap(), (42, 42));
        assert_eq!(parse_line_range("42-45").unwrap(), (42, 45));
        assert!(parse_line_range("45-42").is_err());
        assert!(parse_line_range("0").is_err());
        assert!(parse_line_range("a-b").is_err());
        assert!(parse_line_range("42-").is_err());
    }

    #[test]
    fn review_of_comment_finds_the_owning_review() {
        use prologue_core::diff::DiffMode;
        use prologue_core::testutil::FixtureRepo;

        let dir = tempfile::tempdir().unwrap();
        let conn = prologue_core::testutil::open_test_db(&dir);
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.create_branch("feature");
        fixture.commit_file("a.txt", "two\n", "change");
        let repo_path = fixture.dir.path().to_string_lossy().into_owned();
        let opened =
            review::open_review_impl(&conn, &repo_path, "feature", "main", DiffMode::Committed)
                .unwrap();
        let comment = review::create_comment_impl(
            &conn,
            &DiffSpec {
                repo_path: repo_path.clone(),
                base: "main".into(),
                head: "feature".into(),
                mode: DiffMode::Committed,
            },
            NewComment {
                review_id: opened.id,
                level: CommentLevel::Review,
                file_path: None,
                side: None,
                start_line: None,
                end_line: None,
                parent_id: None,
                body: "note".to_owned(),
                author: Some("agent".to_owned()),
            },
        )
        .unwrap();

        assert_eq!(review_of_comment(&conn, comment.id).unwrap().id, opened.id);
        let err = review_of_comment(&conn, 999).unwrap_err();
        assert!(err.contains("Comment not found: C999"), "{err}");
    }

    #[test]
    fn anchor_errors_carry_the_reread_hint() {
        let no_lines =
            CoreError::NoDiffLines { path: "a.rs".to_owned(), start: 5, end: 6, side: "new" };
        let hinted = with_anchor_hint(no_lines, Some("a.rs"));
        assert!(hinted.contains("No diff lines at a.rs:5-6 (new) to comment on"), "{hinted}");
        assert!(hinted.contains("prologue show --file a.rs --diff"), "{hinted}");
        let hinted = with_anchor_hint(CoreError::SelectionCrossesHunks, Some("a.rs"));
        assert!(hinted.contains("cannot cross hunk boundaries"), "{hinted}");
        assert!(hinted.contains("try again"), "{hinted}");
        // Non-anchor failures pass through untouched.
        let plain = with_anchor_hint("Comment text cannot be empty".into(), Some("a.rs"));
        assert_eq!(plain, "Comment text cannot be empty");
    }

    #[test]
    fn format_aliases_map_onto_the_export_formats() {
        assert!(matches!(ExportFormat::from(FormatArg::Md), ExportFormat::Markdown));
        assert!(matches!(ExportFormat::from(FormatArg::Json), ExportFormat::Json));
        assert!(matches!(ExportFormat::from(FormatArg::PromptMd), ExportFormat::PromptMarkdown));
        assert!(matches!(ExportFormat::from(FormatArg::PromptJson), ExportFormat::PromptJson));
    }

    /// Thread lifecycle must not exist as subcommands — not hidden, not
    /// flag-gated; the parser itself is the contract. The verbs are
    /// intercepted only to explain that resolution is manual, in-app.
    #[test]
    fn lifecycle_verbs_do_not_parse_and_get_the_manual_resolution_error() {
        for verb in LIFECYCLE_VERBS.iter().chain(&["edit"]) {
            let err = Cli::try_parse_from(["prologue", verb, "C1"])
                .err()
                .unwrap_or_else(|| panic!("unexpected subcommand: {verb}"));
            assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
        }
        // The lifecycle verbs map to the explanation; other unknowns don't.
        let err = Cli::try_parse_from(["prologue", "resolve", "C1"]).err().unwrap();
        let verb = attempted_lifecycle_verb(&err).expect("resolve must be intercepted");
        let message = lifecycle_error(&verb);
        assert!(message.contains("manual actions in the Prologue app"), "{message}");
        assert!(message.contains("let the reviewer close the thread"), "{message}");
        let err = Cli::try_parse_from(["prologue", "frobnicate"]).err().unwrap();
        assert_eq!(attempted_lifecycle_verb(&err), None);
    }
}
