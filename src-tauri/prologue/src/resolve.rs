//! Turning "which review?" into a review row: explicit id, `repo@branch`,
//! or nothing at all (cwd's repository + checked-out branch).

use prologue_core::review::{self, Review};
use prologue_core::rusqlite::Connection;
use std::path::{Path, PathBuf};

/// An explicit REVIEW argument: a numeric id, or `repo@branch` where repo is
/// a path (contains `/`) or a repository name (`my-app@main`).
pub enum ReviewRef {
    Id(i64),
    RepoBranch { repo: String, branch: String },
}

impl ReviewRef {
    pub fn parse(arg: &str) -> Result<Self, String> {
        if let Ok(id) = arg.parse::<i64>() {
            return Ok(ReviewRef::Id(id));
        }
        // Split at the LAST '@' so paths containing '@' still parse.
        match arg.rsplit_once('@') {
            Some((repo, branch)) if !repo.is_empty() && !branch.is_empty() => {
                Ok(ReviewRef::RepoBranch { repo: repo.to_owned(), branch: branch.to_owned() })
            }
            _ => Err(format!(
                "Cannot parse review '{arg}' — expected a numeric id or repo@branch"
            )),
        }
    }
}

/// The repository containing `start` (upward `.git` discovery) and its
/// checked-out branch.
pub fn repo_and_branch(start: &Path) -> Result<(PathBuf, String), String> {
    let repo = git2::Repository::discover(start)
        .map_err(|_| format!("Not inside a git repository: {}", start.display()))?;
    let workdir = repo
        .workdir()
        .ok_or("This is a bare repository — no working tree to review")?
        .to_path_buf();
    let head = repo.head().map_err(|_| {
        "The repository has no commits yet — nothing to review".to_owned()
    })?;
    if !head.is_branch() {
        return Err("Detached HEAD — pass a review id or repo@branch explicitly".to_owned());
    }
    let branch = head
        .shorthand()
        .ok_or("Cannot read the current branch name")?
        .to_owned();
    Ok((workdir, branch))
}

/// Whether `stored` (a repo path as the app recorded it) names the same
/// directory as `query`: string equality first, then canonicalized equality
/// so trailing slashes and symlinks (e.g. /tmp vs /private/tmp) don't
/// produce false negatives.
fn paths_match(stored: &str, query: &Path) -> bool {
    let stored_path = Path::new(stored);
    if stored_path == query {
        return true;
    }
    match (stored_path.canonicalize(), query.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The working tree of the repository containing `start`, if any. Unlike
/// `repo_and_branch`, an empty repository or a detached HEAD still counts —
/// listing needs no branch. Bare repositories and paths outside any
/// repository yield None.
pub fn containing_workdir(start: &Path) -> Option<PathBuf> {
    git2::Repository::discover(start).ok()?.workdir().map(Path::to_path_buf)
}

/// Whether a review belongs in a `reviews` listing: an explicit `--repo`
/// query wins, otherwise the repository containing the cwd (when there is
/// one), otherwise everything is in scope.
pub fn review_in_scope(review: &Review, repo: Option<&str>, cwd_repo: Option<&Path>) -> bool {
    match (repo, cwd_repo) {
        (Some(query), _) => repo_matches(review, query),
        (None, Some(workdir)) => paths_match(&review.repo_path, workdir),
        (None, None) => true,
    }
}

/// Whether a review belongs to the repo named by `query`: a path (contains
/// `/`) matches the stored path, a bare name matches the stored path's
/// final component.
pub fn repo_matches(review: &Review, query: &str) -> bool {
    if query.contains('/') {
        paths_match(&review.repo_path, Path::new(query))
    } else {
        Path::new(&review.repo_path).file_name().is_some_and(|n| n.to_string_lossy() == query)
    }
}

/// Resolve an optional REVIEW argument to one review row, read-only. With
/// no argument the review is inferred from `cwd`'s repository and its
/// checked-out branch.
pub fn resolve_review(conn: &Connection, arg: Option<&str>, cwd: &Path) -> Result<Review, String> {
    match arg {
        Some(arg) => match ReviewRef::parse(arg)? {
            ReviewRef::Id(id) => {
                review::find_review(conn, id)?.ok_or(format!("No review with id {id}"))
            }
            ReviewRef::RepoBranch { repo, branch } => {
                let matches: Vec<Review> = review::list_reviews_impl(conn, None, true)?
                    .into_iter()
                    .filter(|r| r.branch == branch && repo_matches(r, &repo))
                    .collect();
                pick_one(matches, &format!("{repo}@{branch}"))
            }
        },
        None => {
            let (workdir, branch) = repo_and_branch(cwd)?;
            let matches: Vec<Review> = review::list_reviews_impl(conn, None, false)?
                .into_iter()
                .filter(|r| r.branch == branch && paths_match(&r.repo_path, &workdir))
                .collect();
            if matches.is_empty() {
                return Err(format!(
                    "No active review for {}@{branch} — open one in the Prologue app, \
                     or pass a review id (see `prologue reviews`)",
                    workdir.display()
                ));
            }
            pick_one(matches, &branch)
        }
    }
}

/// Exactly one match or a useful error. Active reviews shadow archived ones
/// with the same name (only one active per repo+branch can exist).
fn pick_one(mut matches: Vec<Review>, wanted: &str) -> Result<Review, String> {
    let active: Vec<Review> =
        matches.iter().filter(|r| r.status == "active").cloned().collect();
    if active.len() == 1 {
        return Ok(active.into_iter().next().unwrap());
    }
    if !active.is_empty() {
        matches = active;
    }
    match matches.len() {
        0 => Err(format!("No review matches {wanted} (see `prologue reviews --all --archived`)")),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            let ids: Vec<String> = matches
                .iter()
                .map(|r| format!("{} ({} @ {}, {})", r.id, r.repo_path, r.branch, r.status))
                .collect();
            Err(format!(
                "Review '{wanted}' is ambiguous — pass an id instead:\n  {}",
                ids.join("\n  ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prologue_core::diff::DiffMode;
    use prologue_core::review::open_review_impl;
    use prologue_core::testutil::FixtureRepo;

    fn test_db(dir: &tempfile::TempDir) -> Connection {
        prologue_core::db::open(&dir.path().join("reviews.db")).unwrap()
    }

    #[test]
    fn review_ref_parses_ids_and_repo_at_branch() {
        assert!(matches!(ReviewRef::parse("42"), Ok(ReviewRef::Id(42))));
        match ReviewRef::parse("my-app@main").unwrap() {
            ReviewRef::RepoBranch { repo, branch } => {
                assert_eq!(repo, "my-app");
                assert_eq!(branch, "main");
            }
            ReviewRef::Id(_) => panic!("parsed as id"),
        }
        // The LAST '@' splits, so paths containing '@' survive.
        match ReviewRef::parse("/Users/x@home/repo@feature").unwrap() {
            ReviewRef::RepoBranch { repo, branch } => {
                assert_eq!(repo, "/Users/x@home/repo");
                assert_eq!(branch, "feature");
            }
            ReviewRef::Id(_) => panic!("parsed as id"),
        }
        assert!(ReviewRef::parse("no-at-sign").is_err());
        assert!(ReviewRef::parse("@branch").is_err());
        assert!(ReviewRef::parse("repo@").is_err());
    }

    #[test]
    fn repo_and_branch_walks_up_to_the_repository_root() {
        let fixture = FixtureRepo::new();
        fixture.commit_file("src/deep/file.txt", "content\n", "initial");

        let nested = fixture.dir.path().join("src/deep");
        let (workdir, branch) = repo_and_branch(&nested).unwrap();
        assert_eq!(workdir.canonicalize().unwrap(), fixture.dir.path().canonicalize().unwrap());
        assert_eq!(branch, "main");

        let outside = tempfile::tempdir().unwrap();
        assert!(repo_and_branch(outside.path()).is_err());
    }

    #[test]
    fn repo_and_branch_reports_the_checked_out_branch() {
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.create_branch("feature/x");
        let (_, branch) = repo_and_branch(fixture.dir.path()).unwrap();
        assert_eq!(branch, "feature/x");
    }

    #[test]
    fn resolves_by_id_name_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        let repo_path = fixture.dir.path().to_string_lossy().into_owned();
        let review =
            open_review_impl(&conn, &repo_path, "main", "origin/main", DiffMode::Committed)
                .unwrap();

        let cwd = Path::new("/");
        assert_eq!(
            resolve_review(&conn, Some(&review.id.to_string()), cwd).unwrap().id,
            review.id
        );
        assert!(resolve_review(&conn, Some("999"), cwd).is_err());

        // By full path and by repository name (temp dir's final component).
        let by_path = format!("{repo_path}@main");
        assert_eq!(resolve_review(&conn, Some(&by_path), cwd).unwrap().id, review.id);
        let name = fixture.dir.path().file_name().unwrap().to_string_lossy();
        let by_name = format!("{name}@main");
        assert_eq!(resolve_review(&conn, Some(&by_name), cwd).unwrap().id, review.id);

        let err = resolve_review(&conn, Some("no-such-repo@main"), cwd).unwrap_err();
        assert!(err.contains("`prologue reviews --all --archived`"), "{err}");
    }

    #[test]
    fn containing_workdir_tolerates_head_states_repo_and_branch_rejects() {
        let fixture = FixtureRepo::new();
        let head = fixture.commit_file("src/deep/f.txt", "x\n", "initial");

        // Nested directories resolve to the repository root.
        let nested = fixture.dir.path().join("src/deep");
        let workdir = containing_workdir(&nested).unwrap();
        assert_eq!(workdir.canonicalize().unwrap(), fixture.dir.path().canonicalize().unwrap());

        // Outside any repository: no scope, not an error.
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(containing_workdir(outside.path()), None);

        // An empty repository still scopes (repo_and_branch would error).
        let empty = FixtureRepo::new();
        assert!(containing_workdir(empty.dir.path()).is_some());
        assert!(repo_and_branch(empty.dir.path()).is_err());

        // A detached HEAD still scopes (repo_and_branch would error).
        fixture.repo.set_head_detached(head).unwrap();
        assert!(containing_workdir(fixture.dir.path()).is_some());
        assert!(repo_and_branch(fixture.dir.path()).is_err());
    }

    #[test]
    fn review_in_scope_prefers_explicit_repo_then_cwd_repo() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture_a = FixtureRepo::new();
        fixture_a.commit_file("a.txt", "one\n", "initial");
        let fixture_b = FixtureRepo::new();
        fixture_b.commit_file("b.txt", "one\n", "initial");
        let review_a = open_review_impl(
            &conn,
            &fixture_a.dir.path().to_string_lossy(),
            "main",
            "origin/main",
            DiffMode::Committed,
        )
        .unwrap();
        let review_b = open_review_impl(
            &conn,
            &fixture_b.dir.path().to_string_lossy(),
            "main",
            "origin/main",
            DiffMode::Committed,
        )
        .unwrap();

        // An explicit --repo query wins even when the cwd is another repo.
        let name_a = fixture_a.dir.path().file_name().unwrap().to_string_lossy().into_owned();
        assert!(review_in_scope(&review_a, Some(&name_a), Some(fixture_b.dir.path())));
        assert!(!review_in_scope(&review_b, Some(&name_a), Some(fixture_b.dir.path())));

        // The cwd's repo scopes when no --repo is given, including through
        // canonicalization (e.g. /tmp vs /private/tmp).
        assert!(review_in_scope(&review_a, None, Some(fixture_a.dir.path())));
        assert!(!review_in_scope(&review_b, None, Some(fixture_a.dir.path())));
        let canonical = fixture_a.dir.path().canonicalize().unwrap();
        assert!(review_in_scope(&review_a, None, Some(&canonical)));

        // No query and no cwd repo: everything is in scope.
        assert!(review_in_scope(&review_a, None, None));
        assert!(review_in_scope(&review_b, None, None));
    }

    #[test]
    fn bare_invocation_resolves_via_cwd_and_current_branch() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.create_branch("feature");
        let repo_path = fixture.dir.path().to_string_lossy().into_owned();
        let review =
            open_review_impl(&conn, &repo_path, "feature", "main", DiffMode::Committed).unwrap();
        // A review of another branch must not be picked up.
        open_review_impl(&conn, &repo_path, "other", "main", DiffMode::Committed).unwrap();

        // From a nested directory of the checked-out repo.
        fixture.write("src/deep/f.txt", "x\n");
        let nested = fixture.dir.path().join("src/deep");
        assert_eq!(resolve_review(&conn, None, &nested).unwrap().id, review.id);

        // Outside any repository: a helpful error.
        let outside = tempfile::tempdir().unwrap();
        assert!(resolve_review(&conn, None, outside.path()).is_err());
    }

    #[test]
    fn stored_paths_match_canonicalized_queries() {
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        let stored = fixture.dir.path().to_string_lossy().into_owned();
        // Trailing slash and canonical form (e.g. /tmp vs /private/tmp on
        // macOS) both match.
        assert!(paths_match(&stored, &fixture.dir.path().join("")));
        assert!(paths_match(&stored, &fixture.dir.path().canonicalize().unwrap()));
        assert!(!paths_match(&stored, Path::new("/somewhere/else")));
    }
}
