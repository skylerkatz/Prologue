use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::anchor::AnchorStatus;
use crate::db::Db;
use crate::diff::{self, DiffMode};
use crate::repo::open_git_repo;
use crate::review::{self, CodeAnchor, Comment, CommentLevel, CommentSide, CommentState};

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExportFormat {
    Markdown,
    Json,
    PromptMarkdown,
    PromptJson,
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
    let conn = db
        .0
        .lock()
        .map_err(|_| "Review database is unavailable".to_owned())?;
    export_review_impl(&conn, &repo_path, &base, &head, mode, review_id, format)
}

/// One open comment plus whether it is orphaned in the current diff (its
/// anchor no longer matches, or its file left the diff entirely). Orphaned
/// comments still export — the reviewer wrote them — marked with their
/// last known location.
struct ExportComment<'a> {
    comment: &'a Comment,
    orphaned: bool,
}

/// Everything the renderers need, already ordered: review-level comments
/// first, then per-file groups sorted by path (file-level before line-level,
/// line-level by start line).
struct ExportData<'a> {
    repo: String,
    branch: &'a str,
    base_ref: &'a str,
    base_sha: String,
    head_sha: String,
    mode: DiffMode,
    review_level: Vec<ExportComment<'a>>,
    files: Vec<(String, Vec<ExportComment<'a>>)>,
}

pub(crate) fn export_review_impl(
    conn: &Connection,
    repo_path: &str,
    base: &str,
    head: &str,
    mode: DiffMode,
    review_id: i64,
    format: ExportFormat,
) -> Result<String, String> {
    // Re-locate line comments first so exported ranges and orphan status
    // match the diff being exported (persisting moves, as a refresh would).
    let reanchored = review::reanchor_comments_impl(conn, repo_path, base, head, mode, review_id)?;
    let orphaned_anchors: HashSet<i64> = reanchored
        .iter()
        .filter(|r| r.status == AnchorStatus::Orphaned)
        .map(|r| r.comment_id)
        .collect();
    let summary =
        diff::get_diff_summary(repo_path.to_owned(), base.to_owned(), head.to_owned(), mode)?;
    let diff_paths: HashSet<&str> = summary.files.iter().map(|f| f.path.as_str()).collect();

    let comments = review::list_comments_impl(conn, review_id)?;
    let open: Vec<ExportComment> = comments
        .iter()
        .filter(|c| c.state == CommentState::Open)
        .map(|c| ExportComment {
            comment: c,
            orphaned: orphaned_anchors.contains(&c.id)
                || c.file_path
                    .as_deref()
                    .is_some_and(|p| !diff_paths.contains(p)),
        })
        .collect();
    if open.is_empty() {
        return Err("This review has no open comments to export".to_owned());
    }

    let repo = open_git_repo(repo_path)?;
    let base_commit = diff::resolve_commit(&repo, base)?;
    let head_commit = diff::resolve_commit(&repo, head)?;
    let base_sha = repo
        .merge_base(base_commit.id(), head_commit.id())
        .map_err(|_| format!("No merge base between '{base}' and '{head}'"))?
        .to_string();

    let mut review_level = Vec::new();
    let mut by_file: Vec<(String, Vec<ExportComment>)> = Vec::new();
    for entry in open {
        match entry.comment.file_path.clone() {
            None => review_level.push(entry),
            Some(path) => match by_file.iter_mut().find(|(p, _)| *p == path) {
                Some((_, group)) => group.push(entry),
                None => by_file.push((path, vec![entry])),
            },
        }
    }
    by_file.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, group) in &mut by_file {
        // File-level comments (no start line) ahead of line comments; the
        // incoming id order breaks ties.
        group.sort_by_key(|e| e.comment.start_line.unwrap_or(0));
    }

    let data = ExportData {
        repo: Path::new(repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo_path.to_owned()),
        branch: head,
        base_ref: base,
        base_sha,
        head_sha: head_commit.id().to_string(),
        mode,
        review_level,
        files: by_file,
    };

    Ok(match format {
        ExportFormat::Markdown => render_markdown(&data),
        ExportFormat::Json => render_json(&data)?,
        ExportFormat::PromptMarkdown => render_prompt(&data, ExportFormat::Markdown)?,
        ExportFormat::PromptJson => render_prompt(&data, ExportFormat::Json)?,
    })
}

/// How the mode reads in the header. In the working-tree modes the head SHA
/// can only name the branch tip — the diff also covers uncommitted changes,
/// which no SHA can pin down.
fn mode_note(mode: DiffMode) -> &'static str {
    match mode {
        DiffMode::Committed => "committed",
        DiffMode::Staged => {
            "staged (diff includes staged uncommitted changes; head SHA is the branch tip)"
        }
        DiffMode::All => {
            "all (diff includes staged and unstaged working-tree changes; head SHA is the branch tip)"
        }
    }
}

/// `C3 — src/foo.rs:6-7 (old side — removed code; orphaned — last known location)`
fn comment_heading(entry: &ExportComment) -> String {
    let c = entry.comment;
    let mut heading = format!("C{}", c.id);
    if let Some(path) = c.file_path.as_deref() {
        write!(heading, " — {path}").unwrap();
        if let (Some(start), Some(end)) = (c.start_line, c.end_line) {
            write!(heading, ":{start}-{end}").unwrap();
        }
        let mut notes = Vec::new();
        if c.level == CommentLevel::File {
            notes.push("file-level");
        }
        if c.side == Some(CommentSide::Old) {
            notes.push("old side — removed code");
        }
        if entry.orphaned {
            notes.push("orphaned — last known location");
        }
        if !notes.is_empty() {
            write!(heading, " ({})", notes.join("; ")).unwrap();
        }
    }
    heading
}

/// The anchor quoted in a code fence: hunk header first, `>` marking the
/// commented lines, two spaces of padding on context lines. The fence is
/// longer than any backtick run in the content so anchors containing
/// fences cannot break out.
fn fenced_anchor(anchor: &CodeAnchor) -> String {
    let mut lines = vec![anchor.hunk_header.clone()];
    lines.extend(anchor.context_before.iter().map(|l| format!("  {l}")));
    lines.extend(anchor.lines.iter().map(|l| format!("> {l}")));
    lines.extend(anchor.context_after.iter().map(|l| format!("  {l}")));
    fenced(&lines.join("\n"), "")
}

/// Wrap `body` in a backtick fence guaranteed longer than any backtick run
/// inside it (minimum three).
fn fenced(body: &str, info: &str) -> String {
    let longest_run = body
        .lines()
        .flat_map(|l| l.split(|c| c != '`').map(str::len))
        .max()
        .unwrap_or(0);
    let fence = "`".repeat((longest_run + 1).max(3));
    format!("{fence}{info}\n{body}\n{fence}")
}

fn header_lines(data: &ExportData) -> String {
    format!(
        "- Repo: {}\n- Branch: {}\n- Base ref: {}\n- Base SHA: {}\n- Head SHA: {}\n- Mode: {}\n",
        data.repo,
        data.branch,
        data.base_ref,
        data.base_sha,
        data.head_sha,
        mode_note(data.mode)
    )
}

fn render_markdown(data: &ExportData) -> String {
    let mut out = format!("# Code review — {} vs {}\n\n", data.branch, data.base_ref);
    out.push_str(&header_lines(data));

    let has_anchor = data
        .files
        .iter()
        .flat_map(|(_, group)| group)
        .any(|e| e.comment.code_anchor.is_some());
    if has_anchor {
        out.push_str(
            "\nLine comments quote a code anchor: the first line is the hunk header, \
             `>` marks the commented lines, and the unmarked lines are surrounding context.\n",
        );
    }

    if !data.review_level.is_empty() {
        out.push_str("\n## Review comments\n");
        for entry in &data.review_level {
            write!(out, "\n### {}\n\n{}\n", comment_heading(entry), entry.comment.body).unwrap();
        }
    }
    for (path, group) in &data.files {
        write!(out, "\n## {path}\n").unwrap();
        for entry in group {
            write!(out, "\n### {}\n", comment_heading(entry)).unwrap();
            if let Some(anchor) = &entry.comment.code_anchor {
                write!(out, "\n{}\n", fenced_anchor(anchor)).unwrap();
            }
            write!(out, "\n{}\n", entry.comment.body).unwrap();
        }
    }
    out
}

/// JSON export shape. Field names are the documented export contract
/// (snake_case, per the spec) — do not let them drift with internal
/// serde conventions. `code_anchor` keeps its stored camelCase inner shape.
#[derive(Serialize)]
struct JsonExport<'a> {
    repo: &'a str,
    branch: &'a str,
    base_ref: &'a str,
    base_sha: &'a str,
    head_sha: &'a str,
    mode: &'a str,
    comments: Vec<JsonComment<'a>>,
}

#[derive(Serialize)]
struct JsonComment<'a> {
    id: i64,
    level: &'static str,
    file: Option<&'a str>,
    side: Option<&'static str>,
    start_line: Option<u32>,
    end_line: Option<u32>,
    code_anchor: Option<&'a CodeAnchor>,
    comment: &'a str,
    commit_sha: &'a str,
    orphaned: bool,
}

impl<'a> JsonComment<'a> {
    fn from(entry: &'a ExportComment) -> Self {
        let c = entry.comment;
        JsonComment {
            id: c.id,
            level: c.level.as_str(),
            file: c.file_path.as_deref(),
            side: c.side.map(CommentSide::as_str),
            start_line: c.start_line,
            end_line: c.end_line,
            code_anchor: c.code_anchor.as_ref(),
            comment: &c.body,
            commit_sha: &c.commit_sha,
            orphaned: entry.orphaned,
        }
    }
}

fn render_json(data: &ExportData) -> Result<String, String> {
    let comments = data
        .review_level
        .iter()
        .chain(data.files.iter().flat_map(|(_, group)| group))
        .map(JsonComment::from)
        .collect();
    serde_json::to_string_pretty(&JsonExport {
        repo: &data.repo,
        branch: data.branch,
        base_ref: data.base_ref,
        base_sha: &data.base_sha,
        head_sha: &data.head_sha,
        mode: data.mode.as_str(),
        comments,
    })
    .map_err(|e| format!("Failed to encode export JSON: {e}"))
}

fn render_prompt(data: &ExportData, payload: ExportFormat) -> Result<String, String> {
    let mut out = format!(
        "You are addressing code review comments for the repository \"{}\", branch \"{}\" \
         (diffed against {}; base SHA {}, head SHA {}, mode: {}).\n\n\
         Address every comment below:\n\n\
         - Comment IDs look like C12; every comment has one.\n\
         - Line numbers were captured at review time and may have drifted since. Each line \
         comment carries a code anchor quoting the exact commented lines plus surrounding \
         context — locate the target by matching the anchor text, not just the line numbers.\n\
         - Comments on the old side refer to removed or replaced code: address the intent \
         behind the comment (the replacement code, or the removal itself), not a literal \
         line that no longer exists.\n\
         - Comments marked orphaned could not be re-located in the current diff; their file \
         and line range are the last known location. Do your best to find and address them; \
         if the code is truly gone, say so in the checklist.\n\
         - Make the code changes only; do not try to update or resolve the review itself.\n\n\
         When you are finished, end your reply with a checklist mapping every comment ID to \
         what was done, one line per comment, for example:\n\n\
         - C12 — extracted the duplicated query into a helper\n\
         - C13 — no change needed: the null case is already handled upstream\n",
        data.repo,
        data.branch,
        data.base_ref,
        data.base_sha,
        data.head_sha,
        mode_note(data.mode),
    );
    match payload {
        ExportFormat::Markdown => {
            out.push_str("\nThe review follows as Markdown.\n\n---\n\n");
            out.push_str(&render_markdown(data));
        }
        ExportFormat::Json => {
            out.push_str(
                "\nThe review follows as JSON. Each comment object has: id, level \
                 (review | file | line), file, side (old | new), start_line, end_line, \
                 code_anchor ({hunkHeader, contextBefore, lines, contextAfter} — `lines` \
                 are the commented lines verbatim), comment (the reviewer's text), \
                 commit_sha (the head SHA when the comment was written), and orphaned.\n\n",
            );
            out.push_str(&fenced(&render_json(data)?, "json"));
            out.push('\n');
        }
        _ => return Err("Prompt payload must be markdown or json".to_owned()),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{
        create_comment_impl, open_review_impl, update_comment_state_impl, NewComment,
    };
    use crate::testutil::FixtureRepo;

    fn test_db(dir: &tempfile::TempDir) -> Connection {
        crate::db::open(&dir.path().join("reviews.db")).unwrap()
    }

    /// Same shape as review.rs's fixture: main has a 10-line code.txt and
    /// d.txt; feature replaces code.txt line 6 with two lines and deletes
    /// d.txt.
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

    fn sha(fixture: &FixtureRepo, refname: &str) -> String {
        diff::resolve_commit(&fixture.repo, refname).unwrap().id().to_string()
    }

    fn repo_name(fixture: &FixtureRepo) -> String {
        Path::new(&fixture.path())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        conn: &Connection,
        fixture: &FixtureRepo,
        review_id: i64,
        level: CommentLevel,
        file_path: Option<&str>,
        side: Option<CommentSide>,
        lines: Option<(u32, u32)>,
        body: &str,
    ) -> Comment {
        create_comment_impl(
            conn,
            &fixture.path(),
            "main",
            "feature",
            DiffMode::Committed,
            NewComment {
                review_id,
                level,
                file_path: file_path.map(str::to_owned),
                side,
                start_line: lines.map(|(s, _)| s),
                end_line: lines.map(|(_, e)| e),
                body: body.to_owned(),
            },
        )
        .unwrap()
    }

    fn export(
        conn: &Connection,
        fixture: &FixtureRepo,
        review_id: i64,
        format: ExportFormat,
    ) -> Result<String, String> {
        export_review_impl(
            conn,
            &fixture.path(),
            "main",
            "feature",
            DiffMode::Committed,
            review_id,
            format,
        )
    }

    /// Review-level, file-level, new-side line, and old-side line comments,
    /// as most tests want them. Returns (review_id, line comment on
    /// code.txt, line comment on d.txt).
    fn seeded_review(conn: &Connection, fixture: &FixtureRepo) -> (i64, Comment, Comment) {
        let review =
            open_review_impl(conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        create(conn, fixture, review.id, CommentLevel::Review, None, None, None,
            "Overall: tighten naming");
        create(conn, fixture, review.id, CommentLevel::File, Some("code.txt"), None, None,
            "split this file");
        let on_code = create(conn, fixture, review.id, CommentLevel::Line, Some("code.txt"),
            Some(CommentSide::New), Some((6, 7)), "rename these");
        let on_deleted = create(conn, fixture, review.id, CommentLevel::Line, Some("d.txt"),
            Some(CommentSide::Old), Some((1, 2)), "why delete this file?");
        (review.id, on_code, on_deleted)
    }

    #[test]
    fn markdown_export_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let (review_id, on_code, on_deleted) = seeded_review(&conn, &fixture);

        let out = export(&conn, &fixture, review_id, ExportFormat::Markdown).unwrap();
        let expected = format!(
            r#"# Code review — feature vs main

- Repo: {repo}
- Branch: feature
- Base ref: main
- Base SHA: {base_sha}
- Head SHA: {head_sha}
- Mode: committed

Line comments quote a code anchor: the first line is the hunk header, `>` marks the commented lines, and the unmarked lines are surrounding context.

## Review comments

### C1

Overall: tighten naming

## code.txt

### C2 — code.txt (file-level)

split this file

### C3 — code.txt:6-7

```
{code_hunk}
  alpha 3
  alpha 4
  alpha 5
> beta 6a
> beta 6b
  alpha 7
  alpha 8
  alpha 9
```

rename these

## d.txt

### C4 — d.txt:1-2 (old side — removed code)

```
{d_hunk}
> doomed one
> doomed two
```

why delete this file?
"#,
            repo = repo_name(&fixture),
            base_sha = sha(&fixture, "main"),
            head_sha = sha(&fixture, "feature"),
            code_hunk = on_code.code_anchor.unwrap().hunk_header,
            d_hunk = on_deleted.code_anchor.unwrap().hunk_header,
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn json_export_is_exact_and_parses() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let (review_id, on_code, on_deleted) = seeded_review(&conn, &fixture);

        let out = export(&conn, &fixture, review_id, ExportFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let head_sha = sha(&fixture, "feature");
        let expected = serde_json::json!({
            "repo": repo_name(&fixture),
            "branch": "feature",
            "base_ref": "main",
            "base_sha": sha(&fixture, "main"),
            "head_sha": head_sha,
            "mode": "committed",
            "comments": [
                {
                    "id": 1, "level": "review", "file": null, "side": null,
                    "start_line": null, "end_line": null, "code_anchor": null,
                    "comment": "Overall: tighten naming", "commit_sha": head_sha,
                    "orphaned": false
                },
                {
                    "id": 2, "level": "file", "file": "code.txt", "side": null,
                    "start_line": null, "end_line": null, "code_anchor": null,
                    "comment": "split this file", "commit_sha": head_sha,
                    "orphaned": false
                },
                {
                    "id": 3, "level": "line", "file": "code.txt", "side": "new",
                    "start_line": 6, "end_line": 7,
                    "code_anchor": {
                        "hunkHeader": on_code.code_anchor.unwrap().hunk_header,
                        "contextBefore": ["alpha 3", "alpha 4", "alpha 5"],
                        "lines": ["beta 6a", "beta 6b"],
                        "contextAfter": ["alpha 7", "alpha 8", "alpha 9"]
                    },
                    "comment": "rename these", "commit_sha": head_sha,
                    "orphaned": false
                },
                {
                    "id": 4, "level": "line", "file": "d.txt", "side": "old",
                    "start_line": 1, "end_line": 2,
                    "code_anchor": {
                        "hunkHeader": on_deleted.code_anchor.unwrap().hunk_header,
                        "contextBefore": [],
                        "lines": ["doomed one", "doomed two"],
                        "contextAfter": []
                    },
                    "comment": "why delete this file?", "commit_sha": head_sha,
                    "orphaned": false
                }
            ]
        });
        assert_eq!(parsed, expected);
    }

    #[test]
    fn only_open_comments_are_exported() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let (review_id, on_code, on_deleted) = seeded_review(&conn, &fixture);

        update_comment_state_impl(&conn, on_code.id, CommentState::Resolved).unwrap();
        update_comment_state_impl(&conn, on_deleted.id, CommentState::Dismissed).unwrap();

        let out = export(&conn, &fixture, review_id, ExportFormat::Markdown).unwrap();
        assert!(out.contains("### C1"), "{out}");
        assert!(out.contains("### C2"), "{out}");
        assert!(!out.contains("C3"), "resolved comment must not export: {out}");
        assert!(!out.contains("C4"), "dismissed comment must not export: {out}");
        assert!(!out.contains("## d.txt"), "file group with no open comments: {out}");
    }

    #[test]
    fn orphaned_open_comments_are_included_and_marked() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let (review_id, on_code, _) = seeded_review(&conn, &fixture);

        // Revert code.txt to main's content: the file leaves the diff, so
        // its file- and line-level comments orphan; d.txt's stays anchored.
        let lines: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        fixture.commit_file("code.txt", &(lines.join("\n") + "\n"), "revert");

        let out = export(&conn, &fixture, review_id, ExportFormat::Markdown).unwrap();
        assert!(
            out.contains("### C2 — code.txt (file-level; orphaned — last known location)"),
            "{out}"
        );
        assert!(
            out.contains("### C3 — code.txt:6-7 (orphaned — last known location)"),
            "{out}"
        );
        assert!(
            out.contains("### C4 — d.txt:1-2 (old side — removed code)"),
            "still-anchored comment must not be marked: {out}"
        );
        // The header must carry the refreshed head SHA (the revert commit).
        assert!(out.contains(&format!("- Head SHA: {}", sha(&fixture, "feature"))), "{out}");
        assert!(!out.contains(&on_code.commit_sha) || on_code.commit_sha == sha(&fixture, "feature"));

        let json = export(&conn, &fixture, review_id, ExportFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let orphaned: Vec<(i64, bool)> = parsed["comments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["id"].as_i64().unwrap(), c["orphaned"].as_bool().unwrap()))
            .collect();
        assert_eq!(orphaned, vec![(1, false), (2, true), (3, true), (4, false)]);
    }

    #[test]
    fn export_without_open_comments_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        let err = export(&conn, &fixture, review.id, ExportFormat::Markdown).unwrap_err();
        assert!(err.contains("no open comments"), "{err}");

        // A review whose comments are all closed exports nothing either.
        let c = create(&conn, &fixture, review.id, CommentLevel::Review, None, None, None, "hm");
        update_comment_state_impl(&conn, c.id, CommentState::Resolved).unwrap();
        let err = export(&conn, &fixture, review.id, ExportFormat::Json).unwrap_err();
        assert!(err.contains("no open comments"), "{err}");
    }

    #[test]
    fn prompt_exports_wrap_their_payload() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let (review_id, ..) = seeded_review(&conn, &fixture);

        let markdown = export(&conn, &fixture, review_id, ExportFormat::Markdown).unwrap();
        let prompt_md = export(&conn, &fixture, review_id, ExportFormat::PromptMarkdown).unwrap();
        assert!(prompt_md.starts_with("You are addressing code review comments"), "{prompt_md}");
        assert!(prompt_md.contains("locate the target by matching the anchor text"));
        assert!(prompt_md.contains("removed or replaced code"));
        assert!(prompt_md.contains("marked orphaned"));
        assert!(prompt_md.contains("checklist mapping every comment ID"));
        assert!(prompt_md.contains("- C12 — extracted the duplicated query into a helper"));
        assert!(prompt_md.ends_with(&markdown), "prompt must embed the markdown payload verbatim");

        let json = export(&conn, &fixture, review_id, ExportFormat::Json).unwrap();
        let prompt_json = export(&conn, &fixture, review_id, ExportFormat::PromptJson).unwrap();
        assert!(prompt_json.contains("The review follows as JSON"));
        assert!(prompt_json.contains(&format!("```json\n{json}\n```")), "{prompt_json}");
    }

    #[test]
    fn working_tree_modes_are_noted_in_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::All).unwrap();
        create(&conn, &fixture, review.id, CommentLevel::Review, None, None, None, "note");

        let out = export_review_impl(
            &conn,
            &fixture.path(),
            "main",
            "feature",
            DiffMode::All,
            review.id,
            ExportFormat::Markdown,
        )
        .unwrap();
        assert!(
            out.contains(
                "- Mode: all (diff includes staged and unstaged working-tree changes; \
                 head SHA is the branch tip)"
            ),
            "{out}"
        );
        assert!(out.contains(&format!("- Head SHA: {}", sha(&fixture, "feature"))), "{out}");
    }
}
