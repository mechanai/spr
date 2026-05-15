//! Shared test utilities: temp git repo builder and default Config factory.
//!
//! Gated behind `#[cfg(test)]` so this module is only compiled during testing.

use git2::{Oid, Repository, Signature};

use crate::config::{Config, MergeMethod};
use crate::git::Git;

/// A temporary git repository with an initial "base" commit and optional
/// working commits stacked on top.
pub struct TestRepo {
    pub dir: tempfile::TempDir,
    /// Oid of the initial commit (simulates remote default-branch tip).
    pub base_oid: Oid,
}

impl TestRepo {
    /// Create a repo with one initial commit (the "remote tip").
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = Repository::init(dir.path()).expect("init repo");

        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            let path = dir.path().join("README.md");
            std::fs::write(&path, "# test\n").unwrap();
            index.add_path(std::path::Path::new("README.md")).unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        let base_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();

        Self { dir, base_oid }
    }

    /// Add a working commit on top of HEAD with the given message.
    /// Returns the new commit's Oid.
    pub fn add_commit(&self, message: &str) -> Oid {
        let repo = Repository::open(self.dir.path()).unwrap();
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            let filename = format!("file-{}.txt", head.id());
            let path = self.dir.path().join(&filename);
            std::fs::write(&path, message).unwrap();
            index
                .add_path(std::path::Path::new(&filename))
                .unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])
            .unwrap()
    }

    /// Build a `Git` wrapper from this repo.
    pub fn git(&self) -> Git {
        let repo = Repository::open(self.dir.path()).expect("reopen repo");
        Git::new(repo)
    }
}

/// Minimal Config suitable for tests.
pub fn test_config() -> Config {
    Config::new(
        "test-owner".into(),
        "test-repo".into(),
        "main",
        "spr/main/".into(),
        false,  // require_approval
        false,  // require_test_plan
        false,  // create_draft_prs
        true,   // non_interactive
        vec![], // default_reviewers
        MergeMethod::Squash,
    )
}
