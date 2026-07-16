use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct RepoInfo {
    pub path: String,
    pub name: String,
}

#[derive(Serialize)]
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
