//! `prologue show`: a review's threads (roots with nested replies, states,
//! anchors, orphan flags) and the per-file hunk view.

use prologue_core::diff::{self, DiffLine, DiffMode, DiffSpec, FileDiff, LineKind};
use prologue_core::review::{self, Comment, CommentLevel, CommentSide, CommentState, Review};
use prologue_core::rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// One thread: its root, the root's replies in chronological order, and
/// whether the root's anchor is orphaned in the current diff. `orphaned` is
/// `None` when anchors could not be recomputed (e.g. the branch is gone).
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub root: Comment,
    pub orphaned: Option<bool>,
    pub replies: Vec<Comment>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShowData {
    pub review: Review,
    /// False when line ranges/orphan flags reflect stored values rather
    /// than the current diff (recomputing failed; warning on stderr).
    pub anchors_current: bool,
    pub threads: Vec<Thread>,
}

/// Assemble the review's threads with current line numbers and orphan flags,
/// exactly as the app computes them — but without writing anything back.
pub fn show_data(conn: &Connection, review: Review) -> Result<ShowData, String> {
    let mut comments = review::list_comments_impl(conn, review.id)?;

    // Recompute anchors against the diff as it stands right now. This can
    // legitimately fail (archived review whose branch is gone); degrade to
    // stored positions rather than refusing to show anything.
    let recomputed = recompute_anchors(conn, &review);
    let (anchors_current, orphaned_ids, diff_paths) = match recomputed {
        Ok((relocations, diff_paths)) => {
            let orphaned: HashSet<i64> = relocations
                .iter()
                .filter(|r| r.status == prologue_core::anchor::AnchorStatus::Orphaned)
                .map(|r| r.comment_id)
                .collect();
            let moved: HashMap<i64, (Option<u32>, Option<u32>)> = relocations
                .iter()
                .map(|r| (r.comment_id, (r.start_line, r.end_line)))
                .collect();
            for comment in &mut comments {
                if let Some(&(start, end)) = moved.get(&comment.id) {
                    comment.start_line = start;
                    comment.end_line = end;
                }
            }
            (true, orphaned, Some(diff_paths))
        }
        Err(e) => {
            eprintln!("warning: could not recompute anchors ({e}); showing stored line numbers");
            (false, HashSet::new(), None)
        }
    };

    let mut replies_by_root: HashMap<i64, Vec<Comment>> = HashMap::new();
    let mut roots = Vec::new();
    for comment in comments {
        match comment.parent_id {
            Some(root_id) => replies_by_root.entry(root_id).or_default().push(comment),
            None => roots.push(comment),
        }
    }
    let threads = roots
        .into_iter()
        .map(|root| {
            let orphaned = diff_paths.as_ref().map(|paths| {
                orphaned_ids.contains(&root.id)
                    || root.file_path.as_deref().is_some_and(|p| !paths.contains(p))
            });
            Thread {
                replies: replies_by_root.remove(&root.id).unwrap_or_default(),
                orphaned,
                root,
            }
        })
        .collect();
    Ok(ShowData { review, anchors_current, threads })
}

type Recomputed = (Vec<review::ReanchorResult>, HashSet<String>);

fn recompute_anchors(conn: &Connection, review: &Review) -> Result<Recomputed, String> {
    let spec = DiffSpec::from(review);
    let relocations = review::reanchor_comments_impl(conn, &spec, review.id, false)?;
    let summary = diff::get_diff_summary(&spec, false)?;
    let paths = summary.files.into_iter().map(|f| f.path).collect();
    Ok((relocations, paths))
}

/// `code.txt:6-7 (new side; orphaned)` — where a thread lives, or
/// `review-level`.
fn location(thread: &Thread) -> String {
    let root = &thread.root;
    let mut notes = Vec::new();
    if root.level == CommentLevel::File {
        notes.push("file-level".to_owned());
    }
    if root.side == Some(CommentSide::Old) {
        notes.push("old side — removed code".to_owned());
    }
    if thread.orphaned == Some(true) {
        notes.push("orphaned — last known location".to_owned());
    }
    let mut out = match root.file_path.as_deref() {
        None => "review-level".to_owned(),
        Some(path) => match (root.start_line, root.end_line) {
            (Some(start), Some(end)) => format!("{path}:{start}-{end}"),
            _ => path.to_owned(),
        },
    };
    if !notes.is_empty() {
        write!(out, " ({})", notes.join("; ")).unwrap();
    }
    out
}

fn state_str(state: CommentState) -> &'static str {
    match state {
        CommentState::Open => "open",
        CommentState::Resolved => "resolved",
        CommentState::Dismissed => "dismissed",
    }
}

fn push_indented(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            writeln!(out, "{indent}{line}").unwrap();
        }
    }
}

pub fn render_text(data: &ShowData) -> String {
    let r = &data.review;
    let mut out = format!(
        "Review #{} — {} @ {} vs {} ({}, {})\n",
        r.id,
        repo_name(&r.repo_path),
        r.branch,
        r.base_ref,
        mode_str(r.mode),
        r.status,
    );
    let open = data
        .threads
        .iter()
        .filter(|t| t.root.state == CommentState::Open)
        .count();
    writeln!(out, "{} thread(s), {} open", data.threads.len(), open).unwrap();
    if !data.anchors_current {
        out.push_str("(line numbers are stored values; the current diff was unavailable)\n");
    }

    for thread in &data.threads {
        let root = &thread.root;
        writeln!(out, "\nC{} [{}] {}", root.id, state_str(root.state), location(thread)).unwrap();
        if let Some(anchor) = &root.code_anchor {
            for line in &anchor.lines {
                writeln!(out, "  > {line}").unwrap();
            }
        }
        push_indented(&mut out, &root.body, "  ");
        for reply in &thread.replies {
            writeln!(out, "  ↳ C{} (reply)", reply.id).unwrap();
            push_indented(&mut out, &reply.body, "    ");
        }
    }
    out
}

fn mode_str(mode: DiffMode) -> &'static str {
    match mode {
        DiffMode::Committed => "committed",
        DiffMode::Staged => "staged",
        DiffMode::All => "all",
    }
}

fn repo_name(repo_path: &str) -> String {
    std::path::Path::new(repo_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo_path.to_owned())
}

/// The current hunks of one file, with old/new line numbers per line — the
/// coordinates a caller needs to place line comments later.
pub fn file_diff(review: &Review, path: &str) -> Result<FileDiff, String> {
    diff::get_file_diff(&DiffSpec::from(review), false, path)
}

pub fn render_file_diff_text(diff: &FileDiff) -> String {
    let mut out = diff.path.clone();
    if let Some(old) = &diff.old_path {
        write!(out, " (renamed from {old})").unwrap();
    }
    out.push('\n');
    if diff.binary {
        out.push_str("(binary file)\n");
        return out;
    }
    for hunk in &diff.hunks {
        out.push_str(&hunk.header);
        if !hunk.header.ends_with('\n') {
            out.push('\n');
        }
        for line in &hunk.lines {
            writeln!(out, "{}", render_line(line)).unwrap();
        }
    }
    out
}

/// `   6      + beta 6a` — old column, new column, change marker, content.
fn render_line(line: &DiffLine) -> String {
    let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
    let marker = match line.kind {
        LineKind::Context => ' ',
        LineKind::Addition => '+',
        LineKind::Deletion => '-',
    };
    format!("{:>5} {:>5} {} {}", num(line.old_lineno), num(line.new_lineno), marker, line.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prologue_core::diff::DiffMode;
    use prologue_core::review::{create_comment_impl, open_review_impl, NewComment};
    use prologue_core::testutil::FixtureRepo;

    fn test_db(dir: &tempfile::TempDir) -> Connection {
        prologue_core::db::open(&dir.path().join("reviews.db")).unwrap()
    }

    /// main: 10-line code.txt; feature: line 6 replaced by two lines.
    fn fixture() -> FixtureRepo {
        let fixture = FixtureRepo::new();
        let lines: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        fixture.commit_file("code.txt", &(lines.join("\n") + "\n"), "initial");
        fixture.create_branch("feature");
        let mut changed = lines.clone();
        changed[5] = "beta 6a\nbeta 6b".to_owned();
        fixture.commit_file("code.txt", &(changed.join("\n") + "\n"), "feature work");
        fixture
    }

    fn comment(review_id: i64, parent_id: Option<i64>, body: &str) -> NewComment {
        NewComment {
            review_id,
            level: CommentLevel::Line,
            file_path: Some("code.txt".to_owned()),
            side: Some(CommentSide::New),
            start_line: Some(6),
            end_line: Some(7),
            parent_id,
            body: body.to_owned(),
            author: None,
        }
    }

    #[test]
    fn threads_nest_replies_and_recompute_current_line_numbers_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let repo_path = fixture.dir.path().to_string_lossy().into_owned();
        let review =
            open_review_impl(&conn, &repo_path, "feature", "main", DiffMode::Committed).unwrap();
        let spec = DiffSpec {
            repo_path: repo_path.clone(),
            base: "main".into(),
            head: "feature".into(),
            mode: DiffMode::Committed,
        };
        let root = create_comment_impl(&conn, &spec, comment(review.id, None, "needs work"))
            .unwrap();
        create_comment_impl(&conn, &spec, comment(review.id, Some(root.id), "agreed")).unwrap();

        // Shift the commented lines down by two.
        let mut lines: Vec<String> = (1..=10).map(|n| format!("alpha {n}")).collect();
        lines[5] = "beta 6a\nbeta 6b".to_owned();
        lines.splice(0..0, ["intro one".to_owned(), "intro two".to_owned()]);
        fixture.commit_file("code.txt", &(lines.join("\n") + "\n"), "shift");

        let data = show_data(&conn, review.clone()).unwrap();
        assert!(data.anchors_current);
        assert_eq!(data.threads.len(), 1);
        let thread = &data.threads[0];
        assert_eq!(thread.root.id, root.id);
        assert_eq!(thread.orphaned, Some(false));
        // Current (shifted) coordinates, not the stored ones...
        assert_eq!((thread.root.start_line, thread.root.end_line), (Some(8), Some(9)));
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].body, "agreed");

        // ...and nothing was written back.
        let stored = review::list_comments_impl(&conn, review.id).unwrap();
        assert_eq!((stored[0].start_line, stored[0].end_line), (Some(6), Some(7)));

        let text = render_text(&data);
        assert!(text.contains("code.txt:8-9"), "{text}");
        assert!(text.contains("↳ C"), "{text}");
        assert!(text.contains("> beta 6a"), "{text}");

        // The JSON shape round-trips.
        let json = serde_json::to_string(&data).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["threads"][0]["root"]["startLine"], 8);
        assert_eq!(parsed["threads"][0]["orphaned"], false);
    }

    #[test]
    fn file_diff_lists_per_hunk_current_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = fixture();
        let repo_path = fixture.dir.path().to_string_lossy().into_owned();
        let review =
            open_review_impl(&conn, &repo_path, "feature", "main", DiffMode::Committed).unwrap();
        drop(conn);

        let diff = file_diff(&review, "code.txt").unwrap();
        assert_eq!(diff.hunks.len(), 1);
        let text = render_file_diff_text(&diff);
        assert!(text.contains("@@"), "{text}");
        assert!(text.contains("- alpha 6"), "{text}");
        assert!(text.contains("+ beta 6a"), "{text}");

        assert!(file_diff(&review, "unchanged.txt").is_err());
    }
}
