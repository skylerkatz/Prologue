//! `prologue` — read-only command-line access to Prologue (Diff Viewer)
//! code reviews. Opens the database, acts, and exits; never a daemon.

mod db;
mod resolve;
mod show;

use clap::{Parser, Subcommand, ValueEnum};
use prologue_core::export::{self, ExportFormat};
use prologue_core::review::{self, Review};
use prologue_core::rusqlite::Connection;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "prologue", version, about = "Read-only access to Prologue code reviews")]
struct Cli {
    /// Path to reviews.db (default: the Diff Viewer app's database)
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List reviews, active first
    Reviews {
        /// Only reviews of this repository (a path, or a name like "diff-viewer")
        #[arg(long, value_name = "REPO")]
        repo: Option<String>,
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
    /// Print a review's open comments in an export format (byte-identical
    /// to the app's clipboard export)
    Export {
        /// Review id or repo@branch (default: inferred from the cwd's
        /// repository and checked-out branch)
        review: Option<String>,
        #[arg(long, value_enum)]
        format: FormatArg,
    },
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
    let cli = Cli::parse();
    if let Err(message) = run(cli) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let db_path = match cli.db {
        Some(path) => path,
        None => db::default_db_path()?,
    };
    let conn = db::open_reviews_db(&db_path)?;
    let cwd = std::env::current_dir()
        .map_err(|e| format!("Cannot determine the working directory: {e}"))?;

    match cli.command {
        Command::Reviews { repo, archived, json } => {
            let reviews: Vec<Review> = review::list_reviews_impl(&conn, None, archived)?
                .into_iter()
                .filter(|r| repo.as_deref().is_none_or(|q| resolve::repo_matches(r, q)))
                .collect();
            if json {
                println!("{}", to_json(&reviews)?);
            } else if reviews.is_empty() {
                println!("No reviews found");
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
        Command::Export { review, format } => {
            let review = resolve::resolve_review(&conn, review.as_deref(), &cwd)?;
            let text = export::export_review_impl(
                &conn,
                &review.repo_path,
                &review.base_ref,
                &review.branch,
                review.mode,
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
    }
    Ok(())
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("Failed to serialize output: {e}"))
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
        let comments: i64 = review::list_comments_impl(conn, review.id)?.len() as i64;
        rows.push([
            review.id.to_string(),
            review.status.clone(),
            review.repo_path.clone(),
            review.branch.clone(),
            review.base_ref.clone(),
            format!("{:?}", review.mode).to_lowercase(),
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
        Cli::try_parse_from(["prologue", "reviews", "--repo", "diff-viewer", "--archived", "--json"])
            .unwrap();
        Cli::try_parse_from(["prologue", "show"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "7", "--json"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "diff-viewer@main"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "--file", "src/a.rs", "--diff"]).unwrap();
        Cli::try_parse_from(["prologue", "show", "--file", "src/a.rs"]).unwrap();
        // --diff needs --file.
        assert!(Cli::try_parse_from(["prologue", "show", "--diff"]).is_err());
        // --db is global.
        Cli::try_parse_from(["prologue", "show", "--db", "/tmp/x.db"]).unwrap();

        for format in ["md", "markdown", "json", "prompt-md", "prompt-markdown", "prompt-json"] {
            Cli::try_parse_from(["prologue", "export", "7", "--format", format]).unwrap();
        }
        assert!(Cli::try_parse_from(["prologue", "export", "7", "--format", "xml"]).is_err());
        assert!(Cli::try_parse_from(["prologue", "export", "7"]).is_err());
    }

    #[test]
    fn format_aliases_map_onto_the_export_formats() {
        assert!(matches!(ExportFormat::from(FormatArg::Md), ExportFormat::Markdown));
        assert!(matches!(ExportFormat::from(FormatArg::Json), ExportFormat::Json));
        assert!(matches!(ExportFormat::from(FormatArg::PromptMd), ExportFormat::PromptMarkdown));
        assert!(matches!(ExportFormat::from(FormatArg::PromptJson), ExportFormat::PromptJson));
    }

    /// No write subcommand may exist in this phase; the parser itself is the
    /// contract.
    #[test]
    fn the_cli_has_no_write_subcommands() {
        for verb in ["comment", "reply", "resolve", "dismiss", "reopen", "delete", "edit"] {
            assert!(
                Cli::try_parse_from(["prologue", verb]).is_err(),
                "unexpected write subcommand: {verb}"
            );
        }
    }
}
