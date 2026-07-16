use git2::{Delta, Diff, DiffFindOptions, DiffLineType, DiffOptions, Oid, Patch, Repository};
use serde::{Deserialize, Serialize};

use crate::repo::open_git_repo;

/// Which working-tree state is diffed against the merge base.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiffMode {
    /// Branch tip only (default).
    Committed,
    /// Index: committed changes plus whatever is staged.
    Staged,
    /// Working directory: committed + staged + unstaged, with untracked
    /// files as new files (`.gitignore` respected).
    All,
}

impl DiffMode {
    /// Stable text form used in the reviews database.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            DiffMode::Committed => "committed",
            DiffMode::Staged => "staged",
            DiffMode::All => "all",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, String> {
        match s {
            "committed" => Ok(DiffMode::Committed),
            "staged" => Ok(DiffMode::Staged),
            "all" => Ok(DiffMode::All),
            other => Err(format!("Unknown working-tree mode: {other}")),
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileSummary {
    pub path: String,
    /// Previous path; present only for renames.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub base_ref: String,
    pub head_ref: String,
    /// SHA of merge-base(base, head) — the commit the diff is computed against.
    pub merge_base: String,
    pub files: Vec<FileSummary>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    Context,
    Addition,
    Deletion,
}

/// One diff line, side-aware: `old_lineno`/`new_lineno` carry the position on
/// each side (absent on the side the line doesn't exist on), so a split view
/// can be built later without reshaping the data.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub content: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
    /// Total line count of the new side, bounding expand-context below the
    /// last hunk; `None` for deleted or binary files (nothing to expand).
    pub new_total_lines: Option<u32>,
}

/// Merge-base (three-dot) file summary: diff(merge-base(base, head), head),
/// where "head" is the branch tip, index, or working tree depending on `mode`.
#[tauri::command]
pub fn get_diff_summary(
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
) -> Result<DiffSummary, String> {
    let repo = open_git_repo(&repo_path)?;
    let (diff, merge_base) = build_diff(&repo, &base, &head, mode)?;

    let mut files = Vec::new();
    let mut total_additions = 0;
    let mut total_deletions = 0;
    for (idx, delta) in diff.deltas().enumerate() {
        let Some(status) = file_status(delta.status()) else {
            continue;
        };
        let (additions, deletions, binary) =
            match Patch::from_diff(&diff, idx).map_err(git_err("Failed to read file patch"))? {
                Some(patch) => {
                    let (_context, adds, dels) = patch
                        .line_stats()
                        .map_err(git_err("Failed to compute line stats"))?;
                    (adds, dels, delta.flags().is_binary())
                }
                // Binary deltas produce no text patch.
                None => (0, 0, true),
            };
        total_additions += additions;
        total_deletions += deletions;
        files.push(FileSummary {
            path: new_path(&delta)?,
            old_path: rename_old_path(&delta, status),
            status,
            additions,
            deletions,
            binary,
        });
    }

    Ok(DiffSummary {
        base_ref: base,
        head_ref: head,
        merge_base: merge_base.to_string(),
        files,
        total_additions,
        total_deletions,
    })
}

/// Hunks for a single file from the same diff `get_diff_summary` computes;
/// fetched on demand so only the summary crosses IPC up front.
#[tauri::command]
pub fn get_file_diff(
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    path: String,
) -> Result<FileDiff, String> {
    let repo = open_git_repo(&repo_path)?;
    let (diff, _) = build_diff(&repo, &base, &head, mode)?;

    let (idx, status) = diff
        .deltas()
        .enumerate()
        .find_map(|(idx, delta)| {
            let matches = delta_path_matches(&delta, &path);
            file_status(delta.status()).filter(|_| matches).map(|s| (idx, s))
        })
        .ok_or_else(|| format!("No changes for file: {path}"))?;
    let delta = diff
        .get_delta(idx)
        .ok_or_else(|| format!("No changes for file: {path}"))?;
    let file_path = new_path(&delta)?;
    let old_path = rename_old_path(&delta, status);
    let binary = delta.flags().is_binary();

    let mut hunks = Vec::new();
    if let Some(patch) =
        Patch::from_diff(&diff, idx).map_err(git_err("Failed to read file patch"))?
    {
        for h in 0..patch.num_hunks() {
            let (header, old_start, old_lines, new_start, new_lines, line_count) = {
                let (hunk, line_count) = patch
                    .hunk(h)
                    .map_err(git_err("Failed to read diff hunk"))?;
                (
                    text_of(hunk.header()).trim_end().to_owned(),
                    hunk.old_start(),
                    hunk.old_lines(),
                    hunk.new_start(),
                    hunk.new_lines(),
                    line_count,
                )
            };
            let mut lines = Vec::with_capacity(line_count);
            for l in 0..line_count {
                let line = patch
                    .line_in_hunk(h, l)
                    .map_err(git_err("Failed to read diff line"))?;
                let kind = match line.origin_value() {
                    DiffLineType::Context => LineKind::Context,
                    DiffLineType::Addition => LineKind::Addition,
                    DiffLineType::Deletion => LineKind::Deletion,
                    // EOF-newline markers and file/hunk headers are not
                    // content lines.
                    _ => continue,
                };
                lines.push(DiffLine {
                    kind,
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                    content: line_text(line.content()),
                });
            }
            hunks.push(Hunk {
                header,
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines,
            });
        }
    }

    let new_total_lines = if binary || status == FileStatus::Deleted {
        None
    } else {
        new_side_content(&repo, &head, mode, &file_path)
            .ok()
            .map(|content| u32::try_from(text_of(&content).lines().count()).unwrap_or(u32::MAX))
    };

    Ok(FileDiff {
        path: file_path,
        old_path,
        status,
        binary,
        hunks,
        new_total_lines,
    })
}

/// Unchanged lines around hunks, fetched on expand-context clicks so they
/// never cross IPC up front.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ContextLines {
    /// 1-based new-side line number of the first returned line.
    pub start: u32,
    pub lines: Vec<String>,
    /// Total line count of the new-side file, so the frontend knows how far
    /// the gap after the last hunk extends.
    pub total_lines: u32,
}

/// Lines `start..=end` (1-based, clamped) of the file's new side — head tree,
/// index, or working tree depending on `mode`. Old-side line numbers are
/// derivable on the frontend: within a gap between hunks both sides advance
/// together, offset by the surrounding hunk's old/new starts.
#[tauri::command]
pub fn get_context_lines(
    repo_path: String,
    head: String,
    mode: DiffMode,
    path: String,
    start: u32,
    end: u32,
) -> Result<ContextLines, String> {
    let repo = open_git_repo(&repo_path)?;
    let content = new_side_content(&repo, &head, mode, &path)?;
    if content.contains(&0) {
        return Err(format!("Cannot expand context in a binary file: {path}"));
    }

    let text = text_of(&content);
    // `str::lines` strips the `\n` and any `\r` before it, matching how
    // `line_text` normalizes diff lines.
    let all: Vec<&str> = text.lines().collect();
    let total_lines = u32::try_from(all.len()).unwrap_or(u32::MAX);
    let start = start.max(1);
    let end = end.min(total_lines);
    let lines = if start > end {
        Vec::new()
    } else {
        all[(start - 1) as usize..end as usize]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    };
    Ok(ContextLines {
        start,
        lines,
        total_lines,
    })
}

/// The new-side file content for `mode`: branch tip blob, index blob, or
/// working-tree file.
fn new_side_content(
    repo: &Repository,
    head: &str,
    mode: DiffMode,
    path: &str,
) -> Result<Vec<u8>, String> {
    let not_found = || format!("File not found on the new side: {path}");
    match mode {
        DiffMode::Committed => {
            let tree = resolve_commit(repo, head)?
                .tree()
                .map_err(git_err("Failed to load branch tree"))?;
            let entry = tree.get_path(std::path::Path::new(path)).map_err(|_| not_found())?;
            let blob = entry
                .to_object(repo)
                .and_then(|obj| obj.peel_to_blob())
                .map_err(|_| not_found())?;
            Ok(blob.content().to_vec())
        }
        DiffMode::Staged => {
            let index = repo.index().map_err(git_err("Failed to read index"))?;
            let entry = index
                .get_path(std::path::Path::new(path), 0)
                .ok_or_else(not_found)?;
            let blob = repo.find_blob(entry.id).map_err(|_| not_found())?;
            Ok(blob.content().to_vec())
        }
        DiffMode::All => {
            let workdir = repo
                .workdir()
                .ok_or_else(|| "Repository has no working directory".to_owned())?;
            std::fs::read(workdir.join(path)).map_err(|_| not_found())
        }
    }
}

/// Compute the three-dot diff for `mode`, with rename detection (matching
/// `git diff`'s default behavior).
fn build_diff<'r>(
    repo: &'r Repository,
    base: &str,
    head: &str,
    mode: DiffMode,
) -> Result<(Diff<'r>, Oid), String> {
    let base_commit = resolve_commit(repo, base)?;
    let head_commit = resolve_commit(repo, head)?;
    let merge_base = repo
        .merge_base(base_commit.id(), head_commit.id())
        .map_err(|_| format!("No merge base between '{base}' and '{head}'"))?;
    let merge_base_tree = repo
        .find_commit(merge_base)
        .and_then(|c| c.tree())
        .map_err(git_err("Failed to load merge-base tree"))?;

    let mut opts = DiffOptions::new();
    let mut diff = match mode {
        DiffMode::Committed => {
            let head_tree = head_commit
                .tree()
                .map_err(git_err("Failed to load branch tree"))?;
            repo.diff_tree_to_tree(Some(&merge_base_tree), Some(&head_tree), Some(&mut opts))
        }
        DiffMode::Staged => {
            let index = repo.index().map_err(git_err("Failed to read index"))?;
            repo.diff_tree_to_index(Some(&merge_base_tree), Some(&index), Some(&mut opts))
        }
        DiffMode::All => {
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
            repo.diff_tree_to_workdir_with_index(Some(&merge_base_tree), Some(&mut opts))
        }
    }
    .map_err(git_err("Failed to compute diff"))?;

    diff.find_similar(Some(DiffFindOptions::new().renames(true)))
        .map_err(git_err("Failed to detect renames"))?;
    Ok((diff, merge_base))
}

pub(crate) fn resolve_commit<'r>(
    repo: &'r Repository,
    refname: &str,
) -> Result<git2::Commit<'r>, String> {
    repo.revparse_single(refname)
        .and_then(|obj| obj.peel_to_commit())
        .map_err(|_| format!("Cannot resolve ref: {refname}"))
}

fn file_status(delta: Delta) -> Option<FileStatus> {
    match delta {
        Delta::Added | Delta::Untracked | Delta::Copied => Some(FileStatus::Added),
        Delta::Deleted => Some(FileStatus::Deleted),
        Delta::Renamed => Some(FileStatus::Renamed),
        Delta::Modified | Delta::Typechange | Delta::Conflicted => Some(FileStatus::Modified),
        _ => None,
    }
}

/// The file's current path — the new side, falling back to the old side for
/// deletions.
fn new_path(delta: &git2::DiffDelta) -> Result<String, String> {
    delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| "Diff entry has no file path".to_owned())
}

fn rename_old_path(delta: &git2::DiffDelta, status: FileStatus) -> Option<String> {
    if status != FileStatus::Renamed {
        return None;
    }
    delta
        .old_file()
        .path()
        .map(|p| p.to_string_lossy().into_owned())
}

fn delta_path_matches(delta: &git2::DiffDelta, path: &str) -> bool {
    let matches = |f: git2::DiffFile| f.path().is_some_and(|p| p.to_string_lossy() == path);
    matches(delta.new_file()) || matches(delta.old_file())
}

fn git_err(context: &'static str) -> impl Fn(git2::Error) -> String {
    move |e| format!("{context}: {}", e.message())
}

fn text_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Line content without its trailing newline (the newline is implied by the
/// line structure).
fn line_text(bytes: &[u8]) -> String {
    let mut text = text_of(bytes);
    if text.ends_with('\n') {
        text.pop();
    }
    if text.ends_with('\r') {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FixtureRepo;
    use std::process::Command;

    /// main has a.txt/b.txt/c.txt; feature modifies a.txt, adds new.txt,
    /// deletes b.txt, renames c.txt; main then drifts with an extra commit
    /// that a three-dot diff must ignore.
    fn branch_fixture() -> FixtureRepo {
        let fixture = FixtureRepo::new();
        fixture.write("a.txt", "one\ntwo\nthree\n");
        fixture.write("b.txt", "gone\n");
        fixture.write("c.txt", "same content\nacross the rename\nkept identical\n");
        fixture.stage(&["a.txt", "b.txt", "c.txt"]);
        fixture.commit("initial");

        fixture.create_branch("feature");
        fixture.write("a.txt", "one\ntwo\nthree\nfour\n");
        fixture.write("new.txt", "hello\nworld\n");
        fixture.write(
            "renamed.txt",
            "same content\nacross the rename\nkept identical\n",
        );
        fixture.stage(&["a.txt", "new.txt", "renamed.txt"]);
        fixture.stage_removal(&["b.txt", "c.txt"]);
        fixture.commit("feature work");

        // Drift on main after the branch point.
        fixture.checkout("main");
        fixture.commit_file("drift.txt", "base drift\n", "drift");
        fixture.checkout("feature");
        fixture
    }

    fn summary_for(fixture: &FixtureRepo, mode: DiffMode) -> DiffSummary {
        get_diff_summary(
            fixture.path(),
            "main".into(),
            "feature".into(),
            mode,
        )
        .unwrap()
    }

    fn file<'s>(summary: &'s DiffSummary, path: &str) -> &'s FileSummary {
        summary
            .files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} missing from {:?}", summary.files))
    }

    #[test]
    fn committed_diff_uses_the_merge_base_and_ignores_base_drift() {
        let fixture = branch_fixture();
        let summary = summary_for(&fixture, DiffMode::Committed);

        let paths: Vec<&str> = summary.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["a.txt", "b.txt", "new.txt", "renamed.txt"]);
        assert!(
            !paths.contains(&"drift.txt"),
            "three-dot diff must ignore base drift"
        );

        let a = file(&summary, "a.txt");
        assert_eq!(
            (a.status, a.additions, a.deletions),
            (FileStatus::Modified, 1, 0)
        );
        let b = file(&summary, "b.txt");
        assert_eq!(
            (b.status, b.additions, b.deletions),
            (FileStatus::Deleted, 0, 1)
        );
        let new = file(&summary, "new.txt");
        assert_eq!(
            (new.status, new.additions, new.deletions),
            (FileStatus::Added, 2, 0)
        );
        let renamed = file(&summary, "renamed.txt");
        assert_eq!(renamed.status, FileStatus::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some("c.txt"));
        assert_eq!((renamed.additions, renamed.deletions), (0, 0));

        assert_eq!(summary.total_additions, 3);
        assert_eq!(summary.total_deletions, 1);
    }

    /// The new-side path from a numstat entry; renames render as
    /// "old => new" or "prefix/{old => new}/suffix".
    fn numstat_new_path(raw: &str) -> String {
        if let (Some(open), Some(close)) = (raw.find('{'), raw.find('}')) {
            let inside = &raw[open + 1..close];
            let new = inside.split(" => ").nth(1).unwrap_or(inside);
            let mut path = format!("{}{}{}", &raw[..open], new, &raw[close + 1..]);
            // An empty old/new side leaves a doubled separator behind.
            while path.contains("//") {
                path = path.replace("//", "/");
            }
            path
        } else if let Some((_, new)) = raw.split_once(" => ") {
            new.to_owned()
        } else {
            raw.to_owned()
        }
    }

    /// `git diff <base>...<head> --numstat` as sorted (adds, dels, path) rows.
    fn git_cli_numstat(repo_path: &str, base: &str, head: &str) -> Vec<(usize, usize, String)> {
        let range = format!("{base}...{head}");
        let output = Command::new("git")
            .args(["-C", repo_path, "diff", &range, "--numstat"])
            .output()
            .expect("git CLI available");
        assert!(output.status.success());

        let stdout = String::from_utf8(output.stdout).unwrap();
        let mut rows: Vec<(usize, usize, String)> = stdout
            .lines()
            .map(|line| {
                let mut parts = line.splitn(3, '\t');
                let adds = parts.next().unwrap();
                let dels = parts.next().unwrap();
                let raw_path = parts.next().unwrap();
                (
                    // Binary entries report "-"; our summary reports 0.
                    adds.parse().unwrap_or(0),
                    dels.parse().unwrap_or(0),
                    numstat_new_path(raw_path),
                )
            })
            .collect();
        rows.sort_by(|x, y| x.2.cmp(&y.2));
        rows
    }

    fn summary_rows(summary: &DiffSummary) -> Vec<(usize, usize, String)> {
        let mut rows: Vec<(usize, usize, String)> = summary
            .files
            .iter()
            .map(|f| (f.additions, f.deletions, f.path.clone()))
            .collect();
        rows.sort_by(|x, y| x.2.cmp(&y.2));
        rows
    }

    #[test]
    fn committed_diff_matches_git_cli_three_dot_numstat() {
        let fixture = branch_fixture();
        let summary = summary_for(&fixture, DiffMode::Committed);
        let expected = git_cli_numstat(&fixture.path(), "main", "feature");
        assert!(!expected.is_empty());
        assert_eq!(summary_rows(&summary), expected);
    }

    #[test]
    fn committed_diff_matches_git_cli_on_this_real_repo() {
        // CARGO_MANIFEST_DIR is src-tauri; the git repo root is its parent.
        let repo_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let summary = get_diff_summary(
            repo_path.clone(),
            "main".into(),
            "HEAD".into(),
            DiffMode::Committed,
        )
        .unwrap();
        let expected = git_cli_numstat(&repo_path, "main", "HEAD");
        assert_eq!(summary_rows(&summary), expected);
    }

    /// Fixture for mode tests: distinct files change at each layer.
    fn mode_fixture() -> FixtureRepo {
        let fixture = FixtureRepo::new();
        fixture.write(".gitignore", "ignored.txt\n");
        fixture.write("committed.txt", "original\n");
        fixture.write("unstaged.txt", "untouched\n");
        fixture.stage(&[".gitignore", "committed.txt", "unstaged.txt"]);
        fixture.commit("initial");

        fixture.create_branch("feature");
        fixture.commit_file("committed.txt", "original\nchanged on branch\n", "commit");

        fixture.write("staged.txt", "staged only\n");
        fixture.stage(&["staged.txt"]);
        fixture.write("unstaged.txt", "untouched\nedited but not staged\n");
        fixture.write("untracked.txt", "brand new\n");
        fixture.write("ignored.txt", "must not appear\n");
        fixture
    }

    fn paths(summary: &DiffSummary) -> Vec<&str> {
        summary.files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn committed_mode_shows_only_committed_changes() {
        let fixture = mode_fixture();
        let summary = summary_for(&fixture, DiffMode::Committed);
        assert_eq!(paths(&summary), vec!["committed.txt"]);
    }

    #[test]
    fn staged_mode_adds_staged_changes() {
        let fixture = mode_fixture();
        let summary = summary_for(&fixture, DiffMode::Staged);
        assert_eq!(paths(&summary), vec!["committed.txt", "staged.txt"]);
        let staged = file(&summary, "staged.txt");
        assert_eq!(
            (staged.status, staged.additions, staged.deletions),
            (FileStatus::Added, 1, 0)
        );
    }

    #[test]
    fn all_mode_adds_unstaged_edits_and_untracked_files_respecting_gitignore() {
        let fixture = mode_fixture();
        let summary = summary_for(&fixture, DiffMode::All);
        assert_eq!(
            paths(&summary),
            vec!["committed.txt", "staged.txt", "unstaged.txt", "untracked.txt"]
        );

        let unstaged = file(&summary, "unstaged.txt");
        assert_eq!(
            (unstaged.status, unstaged.additions),
            (FileStatus::Modified, 1)
        );
        let untracked = file(&summary, "untracked.txt");
        assert_eq!(
            (untracked.status, untracked.additions, untracked.deletions),
            (FileStatus::Added, 1, 0)
        );
    }

    #[test]
    fn recomputing_picks_up_new_commits_and_working_tree_edits() {
        let fixture = branch_fixture();
        let before = summary_for(&fixture, DiffMode::Committed);
        assert!(!paths(&before).contains(&"later.txt"));

        // A new commit and a fresh working-tree edit, as a refresh would see.
        fixture.commit_file("later.txt", "added later\n", "second commit");
        fixture.write("fresh.txt", "fresh edit\n");

        let after = summary_for(&fixture, DiffMode::Committed);
        assert!(paths(&after).contains(&"later.txt"));

        let after_all = summary_for(&fixture, DiffMode::All);
        assert!(paths(&after_all).contains(&"fresh.txt"));
    }

    #[test]
    fn file_diff_returns_side_aware_hunks() {
        let fixture = branch_fixture();
        let diff = get_file_diff(
            fixture.path(),
            "main".into(),
            "feature".into(),
            DiffMode::Committed,
            "a.txt".into(),
        )
        .unwrap();

        assert_eq!(diff.path, "a.txt");
        assert_eq!(diff.status, FileStatus::Modified);
        assert!(!diff.binary);
        assert_eq!(diff.new_total_lines, Some(4));
        assert_eq!(diff.hunks.len(), 1);

        let hunk = &diff.hunks[0];
        assert!(hunk.header.starts_with("@@ -1,3 +1,4 @@"), "{}", hunk.header);
        assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
        assert_eq!((hunk.new_start, hunk.new_lines), (1, 4));

        let added: Vec<&DiffLine> = hunk
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Addition)
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].content, "four");
        assert_eq!(added[0].old_lineno, None);
        assert_eq!(added[0].new_lineno, Some(4));

        let context: Vec<&DiffLine> = hunk
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Context)
            .collect();
        assert_eq!(context.len(), 3);
        assert_eq!(context[0].old_lineno, Some(1));
        assert_eq!(context[0].new_lineno, Some(1));
    }

    #[test]
    fn file_diff_locates_deleted_files_by_old_path() {
        let fixture = branch_fixture();
        let diff = get_file_diff(
            fixture.path(),
            "main".into(),
            "feature".into(),
            DiffMode::Committed,
            "b.txt".into(),
        )
        .unwrap();

        assert_eq!(diff.status, FileStatus::Deleted);
        assert_eq!(diff.new_total_lines, None);
        assert_eq!(diff.hunks.len(), 1);
        let line = &diff.hunks[0].lines[0];
        assert_eq!(line.kind, LineKind::Deletion);
        assert_eq!(line.content, "gone");
        assert_eq!(line.old_lineno, Some(1));
        assert_eq!(line.new_lineno, None);
    }

    #[test]
    fn file_diff_reports_untracked_files_in_all_mode() {
        let fixture = mode_fixture();
        let diff = get_file_diff(
            fixture.path(),
            "main".into(),
            "feature".into(),
            DiffMode::All,
            "untracked.txt".into(),
        )
        .unwrap();

        assert_eq!(diff.status, FileStatus::Added);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].lines[0].content, "brand new");
    }

    #[test]
    fn file_diff_rejects_a_path_with_no_changes() {
        let fixture = branch_fixture();
        let err = get_file_diff(
            fixture.path(),
            "main".into(),
            "feature".into(),
            DiffMode::Committed,
            "nope.txt".into(),
        )
        .unwrap_err();
        assert!(err.contains("No changes for file"), "{err}");
    }

    /// Fixture for context expansion: feature edits line 10 of a 20-line file,
    /// leaving unchanged lines above and below the hunk.
    fn context_fixture() -> FixtureRepo {
        let fixture = FixtureRepo::new();
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        fixture.write("big.txt", &(lines.join("\n") + "\n"));
        fixture.stage(&["big.txt"]);
        fixture.commit("initial");

        fixture.create_branch("feature");
        let mut changed = lines.clone();
        changed[9] = "line 10 CHANGED".to_owned();
        fixture.commit_file("big.txt", &(changed.join("\n") + "\n"), "edit line 10");
        fixture
    }

    #[test]
    fn context_lines_returns_the_requested_new_side_range() {
        let fixture = context_fixture();
        let ctx = get_context_lines(
            fixture.path(),
            "feature".into(),
            DiffMode::Committed,
            "big.txt".into(),
            1,
            6,
        )
        .unwrap();

        assert_eq!(ctx.start, 1);
        assert_eq!(ctx.total_lines, 20);
        assert_eq!(ctx.lines.len(), 6);
        assert_eq!(ctx.lines[0], "line 1");
        assert_eq!(ctx.lines[5], "line 6");
    }

    #[test]
    fn context_lines_clamps_past_the_end_of_the_file() {
        let fixture = context_fixture();
        let ctx = get_context_lines(
            fixture.path(),
            "feature".into(),
            DiffMode::Committed,
            "big.txt".into(),
            18,
            50,
        )
        .unwrap();

        assert_eq!(ctx.start, 18);
        assert_eq!(ctx.lines, vec!["line 18", "line 19", "line 20"]);
    }

    #[test]
    fn context_lines_reads_the_branch_tip_not_the_working_tree_in_committed_mode() {
        let fixture = context_fixture();
        // Working-tree edit that committed mode must not see.
        fixture.write("big.txt", "workdir only\n");

        let committed = get_context_lines(
            fixture.path(),
            "feature".into(),
            DiffMode::Committed,
            "big.txt".into(),
            1,
            1,
        )
        .unwrap();
        assert_eq!(committed.lines, vec!["line 1"]);
        assert_eq!(committed.total_lines, 20);

        let workdir = get_context_lines(
            fixture.path(),
            "feature".into(),
            DiffMode::All,
            "big.txt".into(),
            1,
            1,
        )
        .unwrap();
        assert_eq!(workdir.lines, vec!["workdir only"]);
        assert_eq!(workdir.total_lines, 1);
    }

    #[test]
    fn context_lines_reads_the_index_in_staged_mode() {
        let fixture = context_fixture();
        fixture.write("big.txt", "staged content\n");
        fixture.stage(&["big.txt"]);
        fixture.write("big.txt", "unstaged after\n");

        let ctx = get_context_lines(
            fixture.path(),
            "feature".into(),
            DiffMode::Staged,
            "big.txt".into(),
            1,
            5,
        )
        .unwrap();
        assert_eq!(ctx.lines, vec!["staged content"]);
        assert_eq!(ctx.total_lines, 1);
    }

    #[test]
    fn context_lines_rejects_a_missing_path() {
        let fixture = context_fixture();
        let err = get_context_lines(
            fixture.path(),
            "feature".into(),
            DiffMode::Committed,
            "nope.txt".into(),
            1,
            5,
        )
        .unwrap_err();
        assert!(err.contains("File not found"), "{err}");
    }

    #[test]
    fn diff_rejects_unresolvable_refs() {
        let fixture = branch_fixture();
        let err = get_diff_summary(
            fixture.path(),
            "does-not-exist".into(),
            "feature".into(),
            DiffMode::Committed,
        )
        .unwrap_err();
        assert!(err.contains("Cannot resolve ref"), "{err}");
    }
}
