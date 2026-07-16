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

/// Validate that `path` points to a local git repository and return its identity.
#[tauri::command]
pub fn open_repo(path: String) -> Result<RepoInfo, String> {
    let repo_path = Path::new(&path);
    if !repo_path.is_dir() {
        return Err(format!("Not a directory: {path}"));
    }
    // `.git` is a directory in a normal clone and a file in worktrees/submodules.
    if !repo_path.join(".git").exists() {
        return Err(format!("Not a git repository: {path}"));
    }
    let name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    Ok(RepoInfo { path, name })
}

/// M1 placeholder: returns canned branch data. M2 replaces the body with git2
/// (local + remote-tracking branches, remote-preferred default base) behind
/// the same command signature.
#[tauri::command]
pub fn list_branches(repo_path: String) -> Result<BranchList, String> {
    open_repo(repo_path)?;
    Ok(BranchList {
        branches: vec![
            "main".into(),
            "origin/main".into(),
            "feature/example-branch".into(),
            "fix/placeholder".into(),
        ],
        current: "feature/example-branch".into(),
        default_base: "origin/main".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn this_repo() -> String {
        // CARGO_MANIFEST_DIR is src-tauri; the git repo root is its parent.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn open_repo_accepts_a_git_repository() {
        let info = open_repo(this_repo()).unwrap();
        assert_eq!(info.name, "diff-viewer");
        assert_eq!(info.path, this_repo());
    }

    #[test]
    fn open_repo_rejects_a_non_git_directory() {
        let err = open_repo(std::env::temp_dir().to_string_lossy().into_owned()).unwrap_err();
        assert!(err.starts_with("Not a git repository"), "{err}");
    }

    #[test]
    fn open_repo_rejects_a_missing_path() {
        let err = open_repo("/nonexistent/path".into()).unwrap_err();
        assert!(err.starts_with("Not a directory"), "{err}");
    }

    #[test]
    fn list_branches_returns_placeholder_data_for_a_valid_repo() {
        let list = list_branches(this_repo()).unwrap();
        assert!(!list.branches.is_empty());
        assert!(list.branches.contains(&list.current));
        assert!(list.branches.contains(&list.default_base));
    }

    #[test]
    fn list_branches_rejects_a_non_git_directory() {
        assert!(list_branches("/nonexistent/path".into()).is_err());
    }
}
