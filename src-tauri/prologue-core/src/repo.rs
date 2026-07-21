use git2::{BranchType, Repository};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Debug)]
pub struct RepoInfo {
    pub path: String,
    pub name: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BranchList {
    pub branches: Vec<String>,
    pub current: String,
    pub default_base: String,
}

/// Open the repository at exactly `path` (no upward discovery), with
/// user-facing error messages.
pub fn open_git_repo(path: &str) -> Result<Repository, String> {
    let repo_path = Path::new(path);
    if !repo_path.is_dir() {
        return Err(format!("Not a directory: {path}"));
    }
    Repository::open(repo_path).map_err(|_| format!("Not a git repository: {path}"))
}

/// Validate that `path` points to a local git repository and return its identity.
pub fn open_repo(path: String) -> Result<RepoInfo, String> {
    open_git_repo(&path)?;
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    Ok(RepoInfo { path, name })
}

/// List local and remote-tracking branches, the checked-out branch, and the
/// auto-detected default base ref.
pub fn list_branches(repo_path: String) -> Result<BranchList, String> {
    let repo = open_git_repo(&repo_path)?;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    let iter = repo
        .branches(None)
        .map_err(|e| format!("Failed to list branches: {}", e.message()))?;
    for entry in iter {
        let (branch, kind) = entry.map_err(|e| format!("Failed to read branch: {}", e.message()))?;
        let Some(name) = branch.name().ok().flatten().map(str::to_owned) else {
            continue;
        };
        match kind {
            BranchType::Local => local.push(name),
            // `origin/HEAD` is a symbolic pointer, not a reviewable branch.
            BranchType::Remote if !name.ends_with("/HEAD") => remote.push(name),
            BranchType::Remote => {}
        }
    }
    local.sort();
    remote.sort();

    let current = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(str::to_owned))
        .unwrap_or_else(|| "HEAD".to_owned());
    let default_base = detect_default_base(&local, &remote, &current);

    let mut branches = local;
    branches.extend(remote);
    Ok(BranchList {
        branches,
        current,
        default_base,
    })
}

/// Remote-tracking branches are preferred over locals — a local `main` is
/// often behind what the remote would diff against.
fn detect_default_base(local: &[String], remote: &[String], current: &str) -> String {
    const PREFERRED: [&str; 3] = ["main", "master", "production"];
    for name in PREFERRED {
        let remote_name = format!("origin/{name}");
        if remote.contains(&remote_name) {
            return remote_name;
        }
    }
    for name in PREFERRED {
        if local.iter().any(|b| b == name) {
            return name.to_owned();
        }
    }
    current.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FixtureRepo;

    #[test]
    fn open_repo_accepts_a_git_repository() {
        // A fixture repo, not this checkout: the project may be checked out
        // under any directory name (e.g. a git worktree).
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "hello\n", "init");
        let info = open_repo(fixture.path()).unwrap();
        let expected_name = Path::new(&fixture.path())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(info.name, expected_name);
        assert_eq!(info.path, fixture.path());
    }

    #[test]
    fn open_repo_rejects_a_non_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        let err = open_repo(dir.path().to_string_lossy().into_owned()).unwrap_err();
        assert!(err.starts_with("Not a git repository"), "{err}");
    }

    #[test]
    fn open_repo_rejects_a_missing_path() {
        let err = open_repo("/nonexistent/path".into()).unwrap_err();
        assert!(err.starts_with("Not a directory"), "{err}");
    }

    #[test]
    fn list_branches_returns_local_and_remote_tracking_branches() {
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.create_branch("feature/x");
        fixture.add_remote_branch("main", "main");

        let list = list_branches(fixture.path()).unwrap();
        assert_eq!(list.branches, vec!["feature/x", "main", "origin/main"]);
        assert_eq!(list.current, "feature/x");
        assert_eq!(list.default_base, "origin/main");
    }

    #[test]
    fn default_base_prefers_origin_main_over_other_remotes() {
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.add_remote_branch("production", "main");
        fixture.add_remote_branch("master", "main");
        fixture.add_remote_branch("main", "main");

        let list = list_branches(fixture.path()).unwrap();
        assert_eq!(list.default_base, "origin/main");
    }

    #[test]
    fn default_base_falls_back_through_remote_candidates() {
        let fixture = FixtureRepo::with_initial_head("master");
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.add_remote_branch("production", "master");
        fixture.add_remote_branch("master", "master");

        let list = list_branches(fixture.path()).unwrap();
        assert_eq!(list.default_base, "origin/master");
    }

    #[test]
    fn default_base_uses_local_main_when_no_remotes_exist() {
        let fixture = FixtureRepo::new();
        fixture.commit_file("a.txt", "one\n", "initial");
        fixture.create_branch("feature/x");

        let list = list_branches(fixture.path()).unwrap();
        assert_eq!(list.default_base, "main");
    }

    #[test]
    fn default_base_uses_local_master_when_main_is_absent() {
        let fixture = FixtureRepo::with_initial_head("master");
        fixture.commit_file("a.txt", "one\n", "initial");

        let list = list_branches(fixture.path()).unwrap();
        assert_eq!(list.default_base, "master");
    }

    #[test]
    fn default_base_falls_back_to_current_branch() {
        let fixture = FixtureRepo::with_initial_head("trunk");
        fixture.commit_file("a.txt", "one\n", "initial");

        let list = list_branches(fixture.path()).unwrap();
        assert_eq!(list.default_base, "trunk");
    }

    #[test]
    fn list_branches_rejects_a_non_git_directory() {
        assert!(list_branches("/nonexistent/path".into()).is_err());
    }
}
