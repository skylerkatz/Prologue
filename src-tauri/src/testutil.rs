use git2::build::CheckoutBuilder;
use git2::{Commit, Oid, Repository, RepositoryInitOptions, Signature};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Throwaway git repository in a temp dir for exercising branch and diff logic.
pub struct FixtureRepo {
    pub dir: TempDir,
    pub repo: Repository,
}

impl FixtureRepo {
    pub fn new() -> Self {
        Self::with_initial_head("main")
    }

    pub fn with_initial_head(branch: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = RepositoryInitOptions::new();
        opts.initial_head(branch);
        let repo = Repository::init_opts(dir.path(), &opts).unwrap();
        Self { dir, repo }
    }

    pub fn path(&self) -> String {
        self.dir.path().to_string_lossy().into_owned()
    }

    pub fn write(&self, rel: &str, content: &str) {
        let full = self.dir.path().join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    pub fn stage(&self, paths: &[&str]) {
        let mut index = self.repo.index().unwrap();
        for path in paths {
            index.add_path(Path::new(path)).unwrap();
        }
        index.write().unwrap();
    }

    pub fn stage_removal(&self, paths: &[&str]) {
        let mut index = self.repo.index().unwrap();
        for path in paths {
            index.remove_path(Path::new(path)).unwrap();
            fs::remove_file(self.dir.path().join(path)).unwrap();
        }
        index.write().unwrap();
    }

    /// Commit whatever is currently staged in the index.
    pub fn commit(&self, message: &str) -> Oid {
        let mut index = self.repo.index().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = self.repo.find_tree(tree_id).unwrap();
        let sig = Signature::now("Fixture", "fixture@example.com").unwrap();
        let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&Commit> = parent.as_ref().into_iter().collect();
        self.repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap()
    }

    /// Convenience: write + stage + commit a single file.
    pub fn commit_file(&self, rel: &str, content: &str, message: &str) -> Oid {
        self.write(rel, content);
        self.stage(&[rel]);
        self.commit(message)
    }

    /// Create `name` at the current HEAD commit and check it out.
    pub fn create_branch(&self, name: &str) {
        let head = self.repo.head().unwrap().peel_to_commit().unwrap();
        self.repo.branch(name, &head, false).unwrap();
        self.checkout(name);
    }

    pub fn checkout(&self, name: &str) {
        self.repo.set_head(&format!("refs/heads/{name}")).unwrap();
        self.repo
            .checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
    }

    /// Merge `branch` into `into` with a true merge commit (both parents),
    /// without touching HEAD or the working tree.
    pub fn merge_into(&self, into: &str, branch: &str) -> Oid {
        let into_commit = self
            .repo
            .revparse_single(into)
            .and_then(|o| o.peel_to_commit())
            .unwrap();
        let branch_commit = self
            .repo
            .revparse_single(branch)
            .and_then(|o| o.peel_to_commit())
            .unwrap();
        let tree = branch_commit.tree().unwrap();
        let sig = Signature::now("Fixture", "fixture@example.com").unwrap();
        self.repo
            .commit(
                Some(&format!("refs/heads/{into}")),
                &sig,
                &sig,
                &format!("merge {branch} into {into}"),
                &tree,
                &[&into_commit, &branch_commit],
            )
            .unwrap()
    }

    pub fn delete_branch(&self, name: &str) {
        let mut branch = self.repo.find_branch(name, git2::BranchType::Local).unwrap();
        branch.delete().unwrap();
    }

    /// Simulate a fetched remote-tracking branch pointing at `target`.
    pub fn add_remote_branch(&self, name: &str, target: &str) {
        let commit = self.repo.revparse_single(target).unwrap().id();
        self.repo
            .reference(
                &format!("refs/remotes/origin/{name}"),
                commit,
                false,
                "fixture",
            )
            .unwrap();
    }
}
