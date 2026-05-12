/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::Result;

use crate::github::GitHubBranch;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MergeMethod {
    #[default]
    Squash,
    Rebase,
    Merge,
}

impl MergeMethod {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "rebase" => Self::Rebase,
            "merge" => Self::Merge,
            _ => Self::Squash,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub owner: String,
    pub repo: String,
    pub master_branch: String,
    pub master_ref: GitHubBranch,
    pub branch_prefix: String,
    pub auth_token: String,
    pub require_approval: bool,
    pub require_test_plan: bool,
    pub create_draft_prs: bool,
    pub non_interactive: bool,
    pub default_reviewers: Vec<String>,
    pub merge_method: MergeMethod,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("owner", &self.owner)
            .field("repo", &self.repo)
            .field("master_branch", &self.master_branch)
            .field("master_ref", &self.master_ref)
            .field("branch_prefix", &self.branch_prefix)
            .field("auth_token", &"[REDACTED]")
            .field("require_approval", &self.require_approval)
            .field("require_test_plan", &self.require_test_plan)
            .field("create_draft_prs", &self.create_draft_prs)
            .field("non_interactive", &self.non_interactive)
            .field("default_reviewers", &self.default_reviewers)
            .field("merge_method", &self.merge_method)
            .finish()
    }
}

impl Config {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        owner: String,
        repo: String,
        master_branch: &str,
        branch_prefix: String,
        auth_token: String,
        require_approval: bool,
        require_test_plan: bool,
        create_draft_prs: bool,
        non_interactive: bool,
        default_reviewers: Vec<String>,
        merge_method: MergeMethod,
    ) -> Self {
        let master_ref =
            GitHubBranch::new_from_branch_name(master_branch, master_branch);
        Self {
            owner,
            repo,
            master_branch: master_branch.to_owned(),
            master_ref,
            branch_prefix,
            auth_token,
            require_approval,
            require_test_plan,
            create_draft_prs,
            non_interactive,
            default_reviewers,
            merge_method,
        }
    }

    #[must_use]
    pub fn master_branch_name(&self) -> &str {
        &self.master_branch
    }

    #[must_use]
    pub fn is_master_branch(&self, branch: &str) -> bool {
        let name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
        name == self.master_branch
    }

    #[must_use]
    pub fn pull_request_url(&self, number: u64) -> String {
        format!(
            "https://github.com/{owner}/{repo}/pull/{number}",
            owner = &self.owner,
            repo = &self.repo
        )
    }

    #[must_use]
    pub fn short_pr_ref(&self, number: u64) -> String {
        format!("{}/{}#{}", &self.owner, &self.repo, number)
    }

    #[must_use]
    pub fn parse_pull_request_field(&self, text: &str) -> Option<u64> {
        if text.is_empty() {
            return None;
        }

        let regex = lazy_regex::regex!(r#"^\s*#?\s*(\d+)\s*$"#);
        let m = regex.captures(text);
        if let Some(caps) = m {
            return Some(caps.get(1).unwrap().as_str().parse().unwrap());
        }

        let regex = lazy_regex::regex!(
            r#"^\s*https?://github.com/([\w\-\.]+)/([\w\-\.]+)/pull/(\d+)([/?#].*)?\s*$"#
        );
        let m = regex.captures(text);
        if let Some(caps) = m
            && self.owner == caps.get(1).unwrap().as_str()
            && self.repo == caps.get(2).unwrap().as_str()
        {
            return Some(caps.get(3).unwrap().as_str().parse().unwrap());
        }

        None
    }

    pub fn new_github_branch_from_ref(
        &self,
        ghref: &str,
    ) -> Result<GitHubBranch> {
        GitHubBranch::new_from_ref(ghref, self.master_ref.branch_name())
    }

    #[must_use]
    pub fn new_github_branch(&self, branch_name: &str) -> GitHubBranch {
        GitHubBranch::new_from_branch_name(
            branch_name,
            self.master_ref.branch_name(),
        )
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    fn config_factory() -> Config {
        crate::config::Config::new(
            "acme".into(),
            "codez".into(),
            "master",
            "spr/foo/".into(),
            "xyz".into(),
            false,
            true,
            false,
            false,
            vec![],
            MergeMethod::Squash,
        )
    }

    #[test]
    fn test_pull_request_url() {
        let gh = config_factory();

        assert_eq!(
            &gh.pull_request_url(123),
            "https://github.com/acme/codez/pull/123"
        );
    }

    #[test]
    fn test_parse_pull_request_field_empty() {
        let gh = config_factory();

        assert_eq!(gh.parse_pull_request_field(""), None);
        assert_eq!(gh.parse_pull_request_field("   "), None);
        assert_eq!(gh.parse_pull_request_field("\n"), None);
    }

    #[test]
    fn test_parse_pull_request_field_number() {
        let gh = config_factory();

        assert_eq!(gh.parse_pull_request_field("123"), Some(123));
        assert_eq!(gh.parse_pull_request_field("   123 "), Some(123));
        assert_eq!(gh.parse_pull_request_field("#123"), Some(123));
        assert_eq!(gh.parse_pull_request_field(" # 123"), Some(123));
    }

    #[test]
    fn test_parse_pull_request_field_url() {
        let gh = config_factory();

        assert_eq!(
            gh.parse_pull_request_field(
                "https://github.com/acme/codez/pull/123"
            ),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field(
                "  https://github.com/acme/codez/pull/123  "
            ),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field(
                "https://github.com/acme/codez/pull/123/"
            ),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field(
                "https://github.com/acme/codez/pull/123?x=a"
            ),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field(
                "https://github.com/acme/codez/pull/123/foo"
            ),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field(
                "https://github.com/acme/codez/pull/123#abc"
            ),
            Some(123)
        );
    }

    #[test]
    fn test_short_pr_ref() {
        let gh = config_factory();
        assert_eq!(gh.short_pr_ref(42), "acme/codez#42");
    }

    #[test]
    fn test_merge_method_parse() {
        assert_eq!(MergeMethod::parse("squash"), MergeMethod::Squash);
        assert_eq!(MergeMethod::parse("rebase"), MergeMethod::Rebase);
        assert_eq!(MergeMethod::parse("merge"), MergeMethod::Merge);
        assert_eq!(MergeMethod::parse("SQUASH"), MergeMethod::Squash);
        assert_eq!(MergeMethod::parse("Rebase"), MergeMethod::Rebase);
        assert_eq!(MergeMethod::parse("unknown"), MergeMethod::Squash);
        assert_eq!(MergeMethod::parse(""), MergeMethod::Squash);
    }

    #[test]
    fn test_is_master_branch() {
        let config = config_factory();
        assert!(config.is_master_branch("master"));
        assert!(config.is_master_branch("refs/heads/master"));
        assert!(!config.is_master_branch("develop"));
        assert!(!config.is_master_branch("spr/main/foo"));
    }

    #[test]
    fn test_master_branch_name() {
        let config = config_factory();
        assert_eq!(config.master_branch_name(), "master");
    }
}
