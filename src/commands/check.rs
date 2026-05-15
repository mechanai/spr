/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, eyre};

use crate::{error::SprError, message::validate_commit_message, output::output_essential};

#[derive(Debug, clap::Parser)]
pub struct CheckOptions {
    /// Also verify cherry-pick would succeed (requires fetch)
    #[clap(long)]
    pub cherry_pick: bool,
}

#[allow(clippy::unused_async)]
pub async fn check(
    opts: CheckOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    // Check for uncommitted changes
    git.check_no_uncommitted_changes()?;

    let remote_tip = forge.fetch_branch(config.default_branch_name())?;
    let prepared_commits =
        crate::forge::get_prepared_commits(git, forge, remote_tip)?;

    let head_commit = prepared_commits
        .last()
        .ok_or_else(|| eyre!("No commits on branch"))?;

    // Validate commit message
    let message = head_commit.message.clone();
    validate_commit_message(&message, config)?;
    output_essential("message: ok")?;

    // Check if PR already exists
    if let Some(number) = head_commit.pull_request_number {
        output_essential(&format!("pr: #{number}"))?;
    } else {
        output_essential("pr: new")?;
    }

    // Cherry-pick conflict check
    if opts.cherry_pick {
        let default_branch_oid = if let Some(first) = prepared_commits.first() {
            first.parent_oid
        } else {
            return Ok(());
        };

        let index = git.cherrypick(head_commit.oid, default_branch_oid)?;
        if index.has_conflicts() {
            output_essential("cherry-pick: conflict")?;
            return Err(SprError::Conflict("Cherry-pick would conflict".into()).into());
        }
        output_essential("cherry-pick: clean")?;
    }

    output_essential("check: pass")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::ForgeApiMock;
    use crate::test_helpers::{TestRepo, test_config};
    use unimock::*;

    fn base_clauses(base_oid: git2::Oid) -> impl Clause {
        ForgeApiMock::fetch_branch
            .some_call(matching!(_))
            .returns(Ok(base_oid))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_commits_errors() {
        let test_repo = TestRepo::new();
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = CheckOptions { cherry_pick: false };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No commits"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn valid_commit_passes() {
        let test_repo = TestRepo::new();
        test_repo.add_commit("feat: add widget\n\nSome summary here");
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = CheckOptions { cherry_pick: false };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "check should pass: {result:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_title_fails() {
        let test_repo = TestRepo::new();
        // git2 allows empty commit messages; parse_message("") → empty Title
        test_repo.add_commit("");
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = CheckOptions { cherry_pick: false };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("title"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_test_plan_fails_when_required() {
        let test_repo = TestRepo::new();
        test_repo.add_commit("feat: add widget\n\nSummary here");
        let git = test_repo.git();
        let mut config = test_config();
        config.require_test_plan = true;

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = CheckOptions { cherry_pick: false };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Test Plan"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn with_test_plan_passes_when_required() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: add widget\n\nSummary here\n\nTest Plan: run cargo test",
        );
        let git = test_repo.git();
        let mut config = test_config();
        config.require_test_plan = true;

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = CheckOptions { cherry_pick: false };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "check should pass: {result:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reports_existing_pr() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: widget\n\nPull Request: https://github.com/test-owner/test-repo/pull/99",
        );
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new((
            base_clauses(test_repo.base_oid),
            ForgeApiMock::parse_cr_field
                .some_call(matching!(_))
                .returns(Ok(Some(99))),
            ForgeApiMock::change_request_url
                .some_call(matching!(99))
                .returns("https://github.com/test-owner/test-repo/pull/99".to_string()),
        ));

        let opts = CheckOptions { cherry_pick: false };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "check should pass: {result:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cherry_pick_clean() {
        let test_repo = TestRepo::new();
        test_repo.add_commit("feat: add widget\n\nClean change");
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = CheckOptions { cherry_pick: true };
        let result = check(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "cherry-pick should be clean: {result:?}");
    }
}
