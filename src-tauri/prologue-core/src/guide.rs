//! Review Guides: an AI-generated grouping of a review's changed files into
//! ordered, summarized sections (Linear-style). This module owns everything
//! except the model call itself — prompt-input assembly, the guard policy,
//! prompt construction, the output JSON Schema, response validation, and
//! persistence. The subprocess that runs `claude -p` lives in the app crate
//! behind [`GuideEngine`], so this crate stays Tauri-free and every stage is
//! testable with a fake engine.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::db::db_err;
use crate::diff::{DiffMode, DiffSummary, FileDiff, FileStatus, FileSummary, LineKind, RepoDiff};

/// Files over this many changed lines are guarded (names + counts only).
/// Mirrors MAX_AUTO_LINES in src/diff/guards.ts — the same files the UI
/// collapses behind a "Load diff" click.
const MAX_AUTO_LINES: usize = 5000;

/// Per-file patch cap: changed (+/−) lines beyond this are cut, with a
/// truncation marker left in the patch so the model knows.
const MAX_PATCH_CHANGED_LINES: usize = 400;

/// Total prompt budget. Patches that would push the prompt past this are
/// omitted entirely (the file stays in the changed-file list).
const MAX_PROMPT_CHARS: usize = 100_000;

/// Slack reserved out of [`MAX_PROMPT_CHARS`] for the trailing truncation
/// note, so appending it can never itself bust the budget.
const PROMPT_NOTE_SLACK: usize = 200;

/// Section title used for files the model failed to place; validation
/// appends them here so every changed file always appears in the guide.
pub const EVERYTHING_ELSE_TITLE: &str = "Everything else";

const EVERYTHING_ELSE_SUMMARY: &str =
    "Remaining changed files that were not grouped into a section above.";

// ---------------------------------------------------------------------------
// Guard policy (port of src/diff/guards.ts — keep the two in sync)
// ---------------------------------------------------------------------------

/// Lockfiles and other generated files nobody reviews line by line.
/// Mirrors GENERATED_NAMES in src/diff/guards.ts.
const GENERATED_NAMES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "deno.lock",
    "cargo.lock",
    "composer.lock",
    "gemfile.lock",
    "poetry.lock",
    "uv.lock",
    "pipfile.lock",
    "go.sum",
    "flake.lock",
    "packages.lock.json",
    "podfile.lock",
];

/// Mirrors GENERATED_SUFFIXES in src/diff/guards.ts.
const GENERATED_SUFFIXES: &[&str] = &[".min.js", ".min.css", ".map", ".snap"];

/// Why a file's patch content is withheld from the prompt (the file itself
/// still contributes its path, status, and +/− counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardReason {
    Binary,
    Oversize,
    Generated,
}

impl GuardReason {
    /// Stable text form, printed in the prompt's changed-file list.
    pub fn as_str(self) -> &'static str {
        match self {
            GuardReason::Binary => "binary",
            GuardReason::Oversize => "oversize",
            GuardReason::Generated => "generated",
        }
    }
}

fn is_generated_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    GENERATED_NAMES.contains(&name.as_str())
        || GENERATED_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Why a file's content is withheld from the prompt, or `None` to include
/// its patch. Same policy and precedence as `guardReason` in
/// src/diff/guards.ts.
pub fn guard_reason(file: &FileSummary) -> Option<GuardReason> {
    if file.binary {
        return Some(GuardReason::Binary);
    }
    if file.additions + file.deletions > MAX_AUTO_LINES {
        return Some(GuardReason::Oversize);
    }
    if is_generated_path(&file.path) {
        return Some(GuardReason::Generated);
    }
    None
}

// ---------------------------------------------------------------------------
// Prompt-input assembly
// ---------------------------------------------------------------------------

/// One changed file as the prompt sees it: identity and counts always,
/// patch text only when the guard policy allows content.
#[derive(Debug, Clone)]
pub struct GuideFileInput {
    pub path: String,
    /// Previous path; present only for renames.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: usize,
    pub deletions: usize,
    pub guard: Option<GuardReason>,
    /// Rendered unified patch; `None` when guarded.
    pub patch: Option<String>,
}

/// Assemble prompt inputs from an already-computed diff: one entry per
/// summary file, in summary order, with patch text for unguarded files.
pub fn guide_file_inputs(
    repo_diff: &RepoDiff,
    summary: &DiffSummary,
) -> Result<Vec<GuideFileInput>, String> {
    summary
        .files
        .iter()
        .map(|file| {
            let guard = guard_reason(file);
            let patch = match guard {
                Some(_) => None,
                None => {
                    let diff = repo_diff.file_diff(&file.path).map_err(|e| e.to_string())?;
                    Some(render_patch(&diff))
                }
            };
            Ok(GuideFileInput {
                path: file.path.clone(),
                old_path: file.old_path.clone(),
                status: file.status,
                additions: file.additions,
                deletions: file.deletions,
                guard,
                patch,
            })
        })
        .collect()
}

/// Unified-diff text from a computed [`FileDiff`]: hunk headers with their
/// prefixed lines, no file header (the prompt labels each patch by path).
fn render_patch(diff: &FileDiff) -> String {
    let mut out = String::new();
    for hunk in &diff.hunks {
        out.push_str(&hunk.header);
        out.push('\n');
        for line in &hunk.lines {
            out.push(match line.kind {
                LineKind::Context => ' ',
                LineKind::Addition => '+',
                LineKind::Deletion => '-',
            });
            out.push_str(&line.content);
            out.push('\n');
        }
    }
    out
}

/// Cut a rendered patch after `max_changed` changed (+/−) lines. Returns
/// the possibly-shortened text and whether anything was cut; cut patches
/// end with a marker line so the model knows it saw a prefix.
fn truncate_patch(patch: &str, max_changed: usize) -> (String, bool) {
    let total_changed = patch
        .lines()
        .filter(|l| l.starts_with('+') || l.starts_with('-'))
        .count();
    if total_changed <= max_changed {
        return (patch.to_owned(), false);
    }
    let mut out = String::new();
    let mut changed = 0usize;
    for line in patch.lines() {
        if line.starts_with('+') || line.starts_with('-') {
            if changed == max_changed {
                break;
            }
            changed += 1;
        }
        out.push_str(line);
        out.push('\n');
    }
    let _ = writeln!(
        out,
        "… [patch truncated: showing {max_changed} of {total_changed} changed lines]"
    );
    (out, true)
}

/// The assembled prompt plus how many files lost patch content to the
/// per-file cap or the total budget (the trailing note tells the model).
#[derive(Debug)]
pub struct GuidePrompt {
    pub text: String,
    pub truncated_files: usize,
}

fn status_letter(status: FileStatus) -> char {
    match status {
        FileStatus::Added => 'A',
        FileStatus::Modified => 'M',
        FileStatus::Deleted => 'D',
        FileStatus::Renamed => 'R',
    }
}

/// Build the one-shot prompt: instructions, the complete changed-file list
/// (every file, always), then patch blocks for unguarded files while the
/// budget lasts. Patch blocks are delimited by marker lines rather than
/// code fences so patch content can never terminate its own block.
pub fn build_prompt(files: &[GuideFileInput]) -> GuidePrompt {
    let mut text = String::from(
        "You are writing a review guide for a code change: group every changed file into \
         ordered sections that tell a reviewer the story of the change.\n\
         \n\
         Rules:\n\
         - Use exact file paths from the changed-file list below; never invent paths.\n\
         - Every file belongs to exactly one section.\n\
         - Order sections as the work was reasoned through: the core change first, then \
         its consequences, then supporting work, with mechanical churn (config, \
         lockfiles, generated files) last.\n\
         - Title each section by what the change accomplishes (\"New retry queue for \
         webhook jobs\"), never by layer or file type (\"Model changes\", \"Tests\").\n\
         - Each summary is 1-3 sentences of specific prose: name the key functions, \
         types, or jobs involved; when the change follows an existing pattern in the \
         codebase, say which one; when something is deleted or replaced, say so \
         explicitly (\"the old X job is removed entirely\").\n\
         - End each summary with the one thing a careful reviewer should verify in that \
         section.\n\
         - Every sentence must tell the reviewer something the file list doesn't — never \
         write \"this section contains changes to…\".\n\
         - Most changes want 2-6 sections; never one section per file.\n\
         \n\
         ## Changed files\n\n",
    );
    for file in files {
        let mut line = format!(
            "{} {} (+{}/-{})",
            status_letter(file.status),
            file.path,
            file.additions,
            file.deletions
        );
        if let Some(old_path) = &file.old_path {
            let _ = write!(line, " (renamed from {old_path})");
        }
        if let Some(guard) = file.guard {
            let _ = write!(line, " [{} — content omitted]", guard.as_str());
        }
        text.push_str(&line);
        text.push('\n');
    }
    text.push_str("\n## Patches\n");

    let budget = MAX_PROMPT_CHARS - PROMPT_NOTE_SLACK;
    let mut truncated_files = 0usize;
    for file in files {
        let Some(patch) = &file.patch else { continue };
        let (patch, cut) = truncate_patch(patch, MAX_PATCH_CHANGED_LINES);
        let block = format!(
            "\n--- begin patch: {path} ---\n{patch}--- end patch ---\n",
            path = file.path
        );
        if text.len() + block.len() > budget {
            truncated_files += 1;
            continue;
        }
        if cut {
            truncated_files += 1;
        }
        text.push_str(&block);
    }
    if truncated_files > 0 {
        let _ = write!(
            text,
            "\nNote: patches for {truncated_files} of the files above were truncated or \
             omitted to fit size limits; the changed-file list is still complete."
        );
    }
    GuidePrompt { text, truncated_files }
}

// ---------------------------------------------------------------------------
// Output shape and validation
// ---------------------------------------------------------------------------

/// One guide section: a titled, summarized group of changed-file paths.
/// The model's raw output parses into this shape, and validated sections
/// persist and serialize to the frontend as it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GuideSection {
    pub title: String,
    pub summary: String,
    pub files: Vec<String>,
}

/// The model's whole response: ordered sections.
#[derive(Deserialize, Debug)]
struct GuideOutput {
    sections: Vec<GuideSection>,
}

/// JSON Schema (draft 2020-12) for the model's output, passed to
/// `claude -p --json-schema` so the reply arrives pre-validated in shape.
/// Referential integrity (real paths, exact-once) is NOT the schema's job —
/// [`validate_sections`] enforces that; never trust the model for it.
pub const OUTPUT_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "sections": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "summary": { "type": "string" },
          "files": {
            "type": "array",
            "items": { "type": "string" }
          }
        },
        "required": ["title", "summary", "files"],
        "additionalProperties": false
      }
    }
  },
  "required": ["sections"],
  "additionalProperties": false
}"#;

/// Repair the model's sections into the guide invariant: every path in
/// `changed_paths` appears exactly once, nothing else appears at all.
/// Hallucinated paths and duplicates are dropped (first placement wins),
/// sections left empty are removed, and files the model missed land in a
/// final "Everything else" section in diff order.
pub fn validate_sections(
    structured: &serde_json::Value,
    changed_paths: &[String],
) -> Result<Vec<GuideSection>, String> {
    let output: GuideOutput = serde_json::from_value(structured.clone())
        .map_err(|e| format!("Guide response did not match the expected shape: {e}"))?;
    let changed: BTreeSet<&str> = changed_paths.iter().map(String::as_str).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    let mut sections: Vec<GuideSection> = Vec::new();
    for section in &output.sections {
        // Keep a path only if it's really in the diff and this is its first
        // placement (`insert` is false on a duplicate).
        let files: Vec<String> = section
            .files
            .iter()
            .filter(|path| changed.contains(path.as_str()) && seen.insert((*path).clone()))
            .cloned()
            .collect();
        if files.is_empty() {
            continue;
        }
        sections.push(GuideSection {
            title: section.title.clone(),
            summary: section.summary.clone(),
            files,
        });
    }

    let missed: Vec<String> = changed_paths
        .iter()
        .filter(|path| !seen.contains(path.as_str()))
        .cloned()
        .collect();
    if !missed.is_empty() {
        // Extend a model-authored trailing "Everything else" rather than
        // stacking a second one under it.
        match sections.last_mut() {
            Some(last) if last.title == EVERYTHING_ELSE_TITLE => last.files.extend(missed),
            _ => sections.push(GuideSection {
                title: EVERYTHING_ELSE_TITLE.to_owned(),
                summary: EVERYTHING_ELSE_SUMMARY.to_owned(),
                files: missed,
            }),
        }
    }
    Ok(sections)
}

// ---------------------------------------------------------------------------
// Engine boundary
// ---------------------------------------------------------------------------

/// What the engine must run: a one-shot prompt constrained to a JSON Schema.
#[derive(Debug, Clone)]
pub struct GuideRequest {
    pub prompt: String,
    /// Always [`OUTPUT_SCHEMA`]; carried on the request so the engine needs
    /// no knowledge of this module.
    pub schema: &'static str,
}

/// What came back: the schema-shaped JSON plus the run's metadata.
#[derive(Debug, Clone)]
pub struct GuideResponse {
    pub structured: serde_json::Value,
    pub model: String,
    pub cost_usd: Option<f64>,
}

/// The model-call boundary. The app crate implements this over a
/// `claude -p` subprocess (which needs tokio and process spawning — not
/// this crate's business); tests implement it with a canned response.
pub trait GuideEngine {
    fn generate(&self, request: &GuideRequest) -> Result<GuideResponse, String>;
}

/// The whole pipeline around an engine: build the prompt, run it, validate
/// the sections, and persist the result (replacing any previous guide for
/// the same diff coordinates). `files` comes from [`guide_file_inputs`] and
/// must belong to `summary`.
pub fn generate_guide_impl(
    conn: &Connection,
    engine: &dyn GuideEngine,
    review_id: i64,
    mode: DiffMode,
    summary: &DiffSummary,
    files: &[GuideFileInput],
) -> Result<Guide, String> {
    let prompt = build_prompt(files);
    let response = engine.generate(&GuideRequest {
        prompt: prompt.text,
        schema: OUTPUT_SCHEMA,
    })?;
    let changed_paths: Vec<String> = summary.files.iter().map(|f| f.path.clone()).collect();
    let sections = validate_sections(&response.structured, &changed_paths)?;
    let fingerprints: BTreeMap<String, String> = summary
        .files
        .iter()
        .map(|f| (f.path.clone(), f.fingerprint.clone()))
        .collect();
    save_guide_impl(
        conn,
        &NewGuide {
            review_id,
            base_ref: summary.base_ref.clone(),
            head_ref: summary.head_ref.clone(),
            mode,
            fingerprints,
            model: response.model,
            cost_usd: response.cost_usd,
        },
        &sections,
    )
}

// ---------------------------------------------------------------------------
// Persistence (schema v5: guides + guide_sections)
// ---------------------------------------------------------------------------

/// A stored guide with its ordered sections.
#[derive(Serialize, Debug, Clone)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Guide {
    /// SQLite rowid — far below 2^53, a plain JS number on the wire.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub review_id: i64,
    pub base_ref: String,
    pub head_ref: String,
    pub mode: DiffMode,
    /// File path → [`FileSummary`] fingerprint at generation time; comparing
    /// against the current summary detects files "changed since the guide".
    ///
    /// [`FileSummary`]: crate::diff::FileSummary
    pub fingerprints: BTreeMap<String, String>,
    pub model: String,
    /// Logged for the record only; no UI surfaces it.
    pub cost_usd: Option<f64>,
    pub created_at: String,
    pub sections: Vec<GuideSection>,
}

/// A guide about to be stored (everything but the row identity).
#[derive(Debug, Clone)]
pub struct NewGuide {
    pub review_id: i64,
    pub base_ref: String,
    pub head_ref: String,
    pub mode: DiffMode,
    pub fingerprints: BTreeMap<String, String>,
    pub model: String,
    pub cost_usd: Option<f64>,
}

/// Store a guide, replacing any existing guide for the same
/// (review, base, head, mode) — regeneration is a full replace.
pub fn save_guide_impl(
    conn: &Connection,
    guide: &NewGuide,
    sections: &[GuideSection],
) -> Result<Guide, String> {
    if crate::review::find_review(conn, guide.review_id)?.is_none() {
        return Err(format!("Review {} not found", guide.review_id));
    }
    let fingerprint_json =
        serde_json::to_string(&guide.fingerprints).map_err(|e| e.to_string())?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(db_err)?;
    {
        let conn = &*tx;
        conn.execute(
            "DELETE FROM guides
             WHERE review_id = ?1 AND base_ref = ?2 AND head_ref = ?3 AND mode = ?4",
            (guide.review_id, &guide.base_ref, &guide.head_ref, guide.mode.as_str()),
        )
        .map_err(db_err)?;
        conn.execute(
            "INSERT INTO guides
                 (review_id, base_ref, head_ref, mode, fingerprint_json, model, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                guide.review_id,
                &guide.base_ref,
                &guide.head_ref,
                guide.mode.as_str(),
                &fingerprint_json,
                &guide.model,
                guide.cost_usd,
            ),
        )
        .map_err(db_err)?;
        let guide_id = conn.last_insert_rowid();
        for (position, section) in sections.iter().enumerate() {
            let files_json = serde_json::to_string(&section.files).map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO guide_sections (guide_id, position, title, summary, files_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (guide_id, position as i64, &section.title, &section.summary, &files_json),
            )
            .map_err(db_err)?;
        }
    }
    tx.commit().map_err(db_err)?;
    find_guide_impl(conn, guide.review_id, &guide.base_ref, &guide.head_ref, guide.mode)?
        .ok_or_else(|| "Guide vanished after save".to_owned())
}

/// The stored guide for one diff's coordinates, sections in order.
pub fn find_guide_impl(
    conn: &Connection,
    review_id: i64,
    base_ref: &str,
    head_ref: &str,
    mode: DiffMode,
) -> Result<Option<Guide>, String> {
    let row = conn
        .query_row(
            "SELECT id, review_id, base_ref, head_ref, mode, fingerprint_json, model,
                    cost_usd, created_at
             FROM guides
             WHERE review_id = ?1 AND base_ref = ?2 AND head_ref = ?3 AND mode = ?4",
            (review_id, base_ref, head_ref, mode.as_str()),
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                    r.get::<_, String>(8)?,
                ))
            },
        )
        .optional()
        .map_err(db_err)?;
    let Some((id, review_id, base_ref, head_ref, mode, fingerprint_json, model, cost_usd, created_at)) =
        row
    else {
        return Ok(None);
    };
    let fingerprints: BTreeMap<String, String> = serde_json::from_str(&fingerprint_json)
        .map_err(|e| format!("Corrupt guide fingerprints: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT title, summary, files_json FROM guide_sections
             WHERE guide_id = ?1 ORDER BY position",
        )
        .map_err(db_err)?;
    let sections = stmt
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?
        .into_iter()
        .map(|(title, summary, files_json)| {
            let files: Vec<String> = serde_json::from_str(&files_json)
                .map_err(|e| format!("Corrupt guide section files: {e}"))?;
            Ok(GuideSection { title, summary, files })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Some(Guide {
        id,
        review_id,
        base_ref,
        head_ref,
        mode: DiffMode::parse(&mode)?,
        fingerprints,
        model,
        cost_usd,
        created_at,
        sections,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffSpec;
    use crate::testutil::{open_test_db, FixtureRepo};
    use serde_json::json;
    use std::cell::RefCell;

    fn summary_file(path: &str, additions: usize, deletions: usize, binary: bool) -> FileSummary {
        FileSummary {
            path: path.to_owned(),
            old_path: None,
            status: FileStatus::Modified,
            additions,
            deletions,
            binary,
            fingerprint: format!("old:{path}:644"),
        }
    }

    #[test]
    fn guard_policy_matches_guards_ts() {
        // Binary wins even for a generated name.
        let file = summary_file("yarn.lock", 1, 1, true);
        assert_eq!(guard_reason(&file), Some(GuardReason::Binary));
        // Oversize: strictly more than 5000 changed lines.
        assert_eq!(guard_reason(&summary_file("big.rs", 5000, 0, false)), None);
        assert_eq!(
            guard_reason(&summary_file("big.rs", 5000, 1, false)),
            Some(GuardReason::Oversize)
        );
        // Generated names are matched on the basename, case-insensitively.
        assert_eq!(
            guard_reason(&summary_file("sub/dir/Cargo.lock", 1, 1, false)),
            Some(GuardReason::Generated)
        );
        assert_eq!(
            guard_reason(&summary_file("go.sum", 1, 1, false)),
            Some(GuardReason::Generated)
        );
        // Generated suffixes.
        for path in ["app.min.js", "style.min.css", "bundle.js.map", "ui.test.tsx.snap"] {
            assert_eq!(
                guard_reason(&summary_file(path, 1, 1, false)),
                Some(GuardReason::Generated),
                "{path}"
            );
        }
        // A name that merely contains a lockfile name is not generated.
        assert_eq!(guard_reason(&summary_file("cargo.lock.md", 1, 1, false)), None);
        assert_eq!(guard_reason(&summary_file("src/main.rs", 10, 2, false)), None);
    }

    fn input(path: &str, patch: Option<&str>, guard: Option<GuardReason>) -> GuideFileInput {
        GuideFileInput {
            path: path.to_owned(),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            guard,
            patch: patch.map(str::to_owned),
        }
    }

    #[test]
    fn prompt_lists_every_file_and_embeds_unguarded_patches() {
        let files = vec![
            input("src/a.rs", Some("@@ -1 +1 @@\n-old\n+new\n"), None),
            input("Cargo.lock", None, Some(GuardReason::Generated)),
        ];
        let prompt = build_prompt(&files);
        assert_eq!(prompt.truncated_files, 0);
        assert!(prompt.text.contains("M src/a.rs (+1/-1)"));
        assert!(prompt.text.contains("M Cargo.lock (+1/-1) [generated — content omitted]"));
        assert!(prompt.text.contains("--- begin patch: src/a.rs ---"));
        assert!(prompt.text.contains("+new"));
        assert!(!prompt.text.contains("begin patch: Cargo.lock"));
        assert!(!prompt.text.contains("Note:"));
    }

    #[test]
    fn prompt_notes_renames() {
        let mut file = input("src/new.rs", Some("@@ -1 +1 @@\n-a\n+b\n"), None);
        file.old_path = Some("src/old.rs".to_owned());
        file.status = FileStatus::Renamed;
        let prompt = build_prompt(&[file]);
        assert!(prompt.text.contains("R src/new.rs (+1/-1) (renamed from src/old.rs)"));
    }

    #[test]
    fn per_file_patches_are_capped_at_400_changed_lines() {
        let mut patch = String::from("@@ -1,500 +1,500 @@\n");
        for n in 0..500 {
            patch.push_str(&format!("-line {n}\n"));
        }
        let prompt = build_prompt(&[input("src/churn.rs", Some(&patch), None)]);
        assert_eq!(prompt.truncated_files, 1);
        assert!(prompt.text.contains("-line 399"));
        assert!(!prompt.text.contains("-line 400\n"));
        assert!(prompt
            .text
            .contains("… [patch truncated: showing 400 of 500 changed lines]"));
        assert!(prompt.text.contains("patches for 1 of the files above"));
    }

    #[test]
    fn prompt_budget_omits_patches_that_do_not_fit() {
        // Three ~60k patches under the per-file changed-line cap: the first
        // fits the ~100k budget, the rest are omitted but stay listed.
        let long_line = format!("+{}\n", "x".repeat(600));
        let patch = format!("@@ -1 +1,100 @@\n{}", long_line.repeat(100));
        let files = vec![
            input("src/one.rs", Some(&patch), None),
            input("src/two.rs", Some(&patch), None),
            input("src/three.rs", Some(&patch), None),
        ];
        let prompt = build_prompt(&files);
        assert_eq!(prompt.truncated_files, 2);
        assert!(prompt.text.len() <= MAX_PROMPT_CHARS);
        assert!(prompt.text.contains("--- begin patch: src/one.rs ---"));
        assert!(!prompt.text.contains("begin patch: src/two.rs"));
        assert!(!prompt.text.contains("begin patch: src/three.rs"));
        assert!(prompt.text.contains("M src/two.rs (+1/-1)"));
        assert!(prompt.text.contains("M src/three.rs (+1/-1)"));
        assert!(prompt.text.contains("patches for 2 of the files above"));
    }

    fn changed(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    fn sections_json(sections: &[(&str, &[&str])]) -> serde_json::Value {
        json!({
            "sections": sections
                .iter()
                .map(|(title, files)| json!({
                    "title": title,
                    "summary": format!("{title} summary"),
                    "files": files,
                }))
                .collect::<Vec<_>>()
        })
    }

    #[test]
    fn validation_drops_hallucinated_paths() {
        let raw = sections_json(&[("Core", &["src/a.rs", "invented.rs"])]);
        let sections = validate_sections(&raw, &changed(&["src/a.rs"])).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].files, ["src/a.rs"]);
    }

    #[test]
    fn validation_keeps_only_the_first_placement_of_a_duplicate() {
        let raw = sections_json(&[("One", &["src/a.rs", "src/b.rs"]), ("Two", &["src/a.rs"])]);
        let sections = validate_sections(&raw, &changed(&["src/a.rs", "src/b.rs"])).unwrap();
        // Section "Two" lost its only file and was dropped entirely.
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "One");
        assert_eq!(sections[0].files, ["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn validation_appends_missed_files_to_everything_else() {
        let raw = sections_json(&[("Core", &["src/a.rs"])]);
        let sections =
            validate_sections(&raw, &changed(&["src/a.rs", "src/b.rs", "src/c.rs"])).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].title, EVERYTHING_ELSE_TITLE);
        // Missed files arrive in diff order.
        assert_eq!(sections[1].files, ["src/b.rs", "src/c.rs"]);
    }

    #[test]
    fn validation_extends_a_model_authored_trailing_everything_else() {
        let raw = sections_json(&[("Core", &["src/a.rs"]), ("Everything else", &["src/b.rs"])]);
        let sections =
            validate_sections(&raw, &changed(&["src/a.rs", "src/b.rs", "src/c.rs"])).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].files, ["src/b.rs", "src/c.rs"]);
    }

    #[test]
    fn validation_guarantees_every_changed_file_exactly_once() {
        let paths = changed(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        let raw = sections_json(&[
            ("One", &["b.rs", "fake.rs", "a.rs"]),
            ("Two", &["a.rs", "d.rs"]),
            ("Empty", &["fake2.rs"]),
        ]);
        let sections = validate_sections(&raw, &paths).unwrap();
        let mut all: Vec<&str> = sections
            .iter()
            .flat_map(|s| s.files.iter().map(String::as_str))
            .collect();
        assert_eq!(all.len(), paths.len());
        all.sort_unstable();
        assert_eq!(all, ["a.rs", "b.rs", "c.rs", "d.rs"]);
    }

    #[test]
    fn validation_rejects_a_malformed_response() {
        let raw = json!({ "sections": [{ "title": "no summary or files" }] });
        let err = validate_sections(&raw, &changed(&["a.rs"])).unwrap_err();
        assert!(err.contains("expected shape"), "{err}");
    }

    #[test]
    fn output_schema_is_valid_draft_2020_12_json() {
        let schema: serde_json::Value = serde_json::from_str(OUTPUT_SCHEMA).unwrap();
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(schema["required"], json!(["sections"]));
    }

    /// The standard fixture (code.txt modified, d.txt deleted) plus a
    /// generated Cargo.lock change on the feature branch.
    fn fixture_with_lockfile() -> FixtureRepo {
        let fixture = FixtureRepo::standard_review_fixture();
        fixture.write("Cargo.lock", "version = 4\n");
        fixture.stage(&["Cargo.lock"]);
        fixture.commit("lockfile churn");
        fixture
    }

    fn spec(fixture: &FixtureRepo) -> DiffSpec {
        DiffSpec {
            repo_path: fixture.path(),
            base: "main".to_owned(),
            head: "feature".to_owned(),
            mode: DiffMode::Committed,
        }
    }

    #[test]
    fn inputs_carry_patches_for_unguarded_files_only() {
        let fixture = fixture_with_lockfile();
        let spec = spec(&fixture);
        let repo_diff = RepoDiff::compute(&fixture.repo, &spec, false).unwrap();
        let summary = repo_diff.summary().unwrap();
        let inputs = guide_file_inputs(&repo_diff, &summary).unwrap();

        assert_eq!(inputs.len(), summary.files.len());
        let by_path = |p: &str| inputs.iter().find(|f| f.path == p).unwrap();
        let code = by_path("code.txt");
        assert_eq!(code.guard, None);
        let patch = code.patch.as_deref().unwrap();
        assert!(patch.contains("@@"), "{patch}");
        assert!(patch.contains("+beta 6a"), "{patch}");
        assert!(patch.contains("-alpha 6"), "{patch}");
        assert!(by_path("d.txt").patch.as_deref().unwrap().contains("-doomed one"));
        let lock = by_path("Cargo.lock");
        assert_eq!(lock.guard, Some(GuardReason::Generated));
        assert_eq!(lock.patch, None);
    }

    fn seeded_review(conn: &Connection, fixture: &FixtureRepo) -> i64 {
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES (?1, 'feature', 'main', 'committed')",
            [fixture.path()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn new_guide(review_id: i64) -> NewGuide {
        NewGuide {
            review_id,
            base_ref: "main".to_owned(),
            head_ref: "feature".to_owned(),
            mode: DiffMode::Committed,
            fingerprints: BTreeMap::from([("code.txt".to_owned(), "aaa:bbb:644".to_owned())]),
            model: "sonnet".to_owned(),
            cost_usd: Some(0.004),
        }
    }

    fn section(title: &str, files: &[&str]) -> GuideSection {
        GuideSection {
            title: title.to_owned(),
            summary: format!("{title} summary"),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    #[test]
    fn saved_guides_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_test_db(&dir);
        let fixture = FixtureRepo::standard_review_fixture();
        let review_id = seeded_review(&conn, &fixture);

        let sections = vec![section("Core", &["code.txt"]), section("Cleanup", &["d.txt"])];
        let saved = save_guide_impl(&conn, &new_guide(review_id), &sections).unwrap();
        assert_eq!(saved.review_id, review_id);
        assert_eq!(saved.mode, DiffMode::Committed);
        assert_eq!(saved.model, "sonnet");
        assert_eq!(saved.cost_usd, Some(0.004));
        assert_eq!(saved.fingerprints["code.txt"], "aaa:bbb:644");
        assert_eq!(saved.sections, sections);
        assert!(!saved.created_at.is_empty());

        let found =
            find_guide_impl(&conn, review_id, "main", "feature", DiffMode::Committed).unwrap();
        assert_eq!(found.unwrap().sections, sections);
        // Other coordinates have no guide.
        assert!(find_guide_impl(&conn, review_id, "main", "feature", DiffMode::All)
            .unwrap()
            .is_none());
    }

    #[test]
    fn resaving_replaces_the_previous_guide() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_test_db(&dir);
        let fixture = FixtureRepo::standard_review_fixture();
        let review_id = seeded_review(&conn, &fixture);

        save_guide_impl(&conn, &new_guide(review_id), &[section("First", &["code.txt"])])
            .unwrap();
        let second = save_guide_impl(
            &conn,
            &new_guide(review_id),
            &[section("Second", &["code.txt", "d.txt"])],
        )
        .unwrap();
        assert_eq!(second.sections.len(), 1);
        assert_eq!(second.sections[0].title, "Second");

        let guides: i64 = conn
            .query_row("SELECT COUNT(*) FROM guides", [], |r| r.get(0))
            .unwrap();
        let sections: i64 = conn
            .query_row("SELECT COUNT(*) FROM guide_sections", [], |r| r.get(0))
            .unwrap();
        assert_eq!((guides, sections), (1, 1));
    }

    #[test]
    fn saving_a_guide_for_a_missing_review_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_test_db(&dir);
        let err = save_guide_impl(&conn, &new_guide(42), &[]).unwrap_err();
        assert!(err.contains("Review 42 not found"), "{err}");
    }

    /// Canned engine: returns a fixed response and captures the prompt.
    struct FakeEngine {
        response: serde_json::Value,
        prompt: RefCell<Option<String>>,
    }

    impl GuideEngine for FakeEngine {
        fn generate(&self, request: &GuideRequest) -> Result<GuideResponse, String> {
            assert_eq!(request.schema, OUTPUT_SCHEMA);
            *self.prompt.borrow_mut() = Some(request.prompt.clone());
            Ok(GuideResponse {
                structured: self.response.clone(),
                model: "sonnet-test".to_owned(),
                cost_usd: Some(0.01),
            })
        }
    }

    #[test]
    fn generate_guide_runs_the_whole_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_test_db(&dir);
        let fixture = fixture_with_lockfile();
        let review_id = seeded_review(&conn, &fixture);

        let spec = spec(&fixture);
        let repo_diff = RepoDiff::compute(&fixture.repo, &spec, false).unwrap();
        let summary = repo_diff.summary().unwrap();
        let inputs = guide_file_inputs(&repo_diff, &summary).unwrap();

        // The model groups code.txt, hallucinates a path, and misses the rest.
        let engine = FakeEngine {
            response: sections_json(&[("Core change", &["code.txt", "invented.rs"])]),
            prompt: RefCell::new(None),
        };
        let guide = generate_guide_impl(&conn, &engine, review_id, spec.mode, &summary, &inputs)
            .unwrap();

        // The prompt embedded the unguarded patch but not the lockfile's.
        let prompt = engine.prompt.borrow().clone().unwrap();
        assert!(prompt.contains("--- begin patch: code.txt ---"));
        assert!(prompt.contains("+beta 6a"));
        assert!(!prompt.contains("begin patch: Cargo.lock"));
        assert!(prompt.contains("[generated — content omitted]"));

        // Validated sections: hallucination dropped, missed files appended.
        assert_eq!(guide.sections.len(), 2);
        assert_eq!(guide.sections[0].title, "Core change");
        assert_eq!(guide.sections[0].files, ["code.txt"]);
        assert_eq!(guide.sections[1].title, EVERYTHING_ELSE_TITLE);
        let mut everything_else = guide.sections[1].files.clone();
        everything_else.sort_unstable();
        assert_eq!(everything_else, ["Cargo.lock", "d.txt"]);

        // Metadata and fingerprints persisted from the run and the summary.
        assert_eq!(guide.model, "sonnet-test");
        assert_eq!(guide.cost_usd, Some(0.01));
        assert_eq!(guide.fingerprints.len(), summary.files.len());
        for file in &summary.files {
            assert_eq!(guide.fingerprints[&file.path], file.fingerprint);
        }
        assert!(
            find_guide_impl(&conn, review_id, &summary.base_ref, &summary.head_ref, spec.mode)
                .unwrap()
                .is_some()
        );
    }
}
