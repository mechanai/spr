/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, eyre};

/// Forge-neutral git branch reference.
///
/// Normalizes branch names and full refs (e.g. `refs/heads/foo`).
#[derive(Debug, Clone)]
pub struct ForgeBranch {
    full_ref: String,
    is_default: bool,
}

impl ForgeBranch {
    /// Create from a full git ref (e.g. `refs/heads/foo`) or bare branch name.
    pub fn from_ref(git_ref: &str, default_branch: &str) -> Result<Self> {
        let full_ref = if git_ref.starts_with("refs/heads/") {
            git_ref.to_string()
        } else if git_ref.starts_with("refs/") {
            return Err(eyre!("Ref '{git_ref}' does not refer to a branch"));
        } else {
            format!("refs/heads/{git_ref}")
        };

        let branch_name = full_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&full_ref);
        let is_default = branch_name == default_branch;

        Ok(Self { full_ref, is_default })
    }

    /// Create from a bare branch name.
    #[must_use]
    pub fn from_branch_name(name: &str, default_branch: &str) -> Self {
        Self {
            full_ref: format!("refs/heads/{name}"),
            is_default: name == default_branch,
        }
    }

    #[must_use]
    pub fn full_ref(&self) -> &str {
        &self.full_ref
    }

    #[must_use]
    pub fn branch_name(&self) -> &str {
        self.full_ref.strip_prefix("refs/heads/").unwrap_or(&self.full_ref)
    }

    #[must_use]
    pub fn is_default(&self) -> bool {
        self.is_default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_ref_bare_name() {
        let b = ForgeBranch::from_ref("foo", "main").unwrap();
        assert_eq!(b.full_ref(), "refs/heads/foo");
        assert_eq!(b.branch_name(), "foo");
        assert!(!b.is_default());
    }

    #[test]
    fn from_ref_full_ref() {
        let b = ForgeBranch::from_ref("refs/heads/foo", "main").unwrap();
        assert_eq!(b.full_ref(), "refs/heads/foo");
        assert_eq!(b.branch_name(), "foo");
    }

    #[test]
    fn from_ref_default_branch() {
        let b = ForgeBranch::from_ref("main", "main").unwrap();
        assert!(b.is_default());
    }

    #[test]
    fn from_ref_non_branch_ref_errors() {
        assert!(ForgeBranch::from_ref("refs/tags/v1", "main").is_err());
    }

    #[test]
    fn from_branch_name_basic() {
        let b = ForgeBranch::from_branch_name("feature", "main");
        assert_eq!(b.full_ref(), "refs/heads/feature");
        assert!(!b.is_default());
    }

    #[test]
    fn from_branch_name_default() {
        let b = ForgeBranch::from_branch_name("main", "main");
        assert!(b.is_default());
    }

    #[test]
    fn from_ref_nested_refs_heads() {
        let b = ForgeBranch::from_ref("refs/heads/refs/heads/foo", "main").unwrap();
        assert_eq!(b.full_ref(), "refs/heads/refs/heads/foo");
        assert_eq!(b.branch_name(), "refs/heads/foo");
        assert!(!b.is_default());
    }
}
