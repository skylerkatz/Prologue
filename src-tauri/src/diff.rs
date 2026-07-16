use git2::{Delta, Diff, DiffFindOptions, DiffLineType, DiffOptions, Oid, Patch, Repository};
use serde::{Deserialize, Serialize};

use crate::repo::open_git_repo;

/// Which working-tree state is diffed against the merge base.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
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

    Ok(FileDiff {
        path: file_path,
        old_path,
        status,
        binary,
        hunks,
    })
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

fn resolve_commit<'r>(repo: &'r Repository, refname: &str) -> Result<git2::Commit<'r>, String> {
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

    #[test]
    fn committed_diff_matches_git_cli_three_dot_numstat() {
        let fixture = branch_fixture();
        let summary = summary_for(&fixture, DiffMode::Committed);

        let output = Command::new("git")
            .args(["-C", &fixture.path(), "diff", "main...feature", "--numstat"])
            .output()
            .expect("git CLI available");
        assert!(output.status.success());

        // numstat lines: "<adds>\t<dels>\t<path>"; renames render the path as
        // "old => new" or "prefix{old => new}suffix".
        let stdout = String::from_utf8(output.stdout).unwrap();
        let mut expected: Vec<(usize, usize, String)> = stdout
            .lines()
            .map(|line| {
                let mut parts = line.splitn(3, '\t');
                let adds = parts.next().unwrap().parse().unwrap();
                let dels = parts.next().unwrap().parse().unwrap();
                let raw_path = parts.next().unwrap();
                let path = match raw_path.split_once(" => ") {
                    Some((_, new)) => new.trim_end_matches('}').to_owned(),
                    None => raw_path.to_owned(),
                };
                (adds, dels, path)
            })
            .collect();
        expected.sort_by(|x, y| x.2.cmp(&y.2));

        let mut actual: Vec<(usize, usize, String)> = summary
            .files
            .iter()
            .map(|f| (f.additions, f.deletions, f.path.clone()))
            .collect();
        actual.sort_by(|x, y| x.2.cmp(&y.2));

        assert_eq!(actual, expected);
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
