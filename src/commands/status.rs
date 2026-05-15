/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, eyre};

use crate::output::output_essential;

#[derive(Debug, clap::Parser)]
pub struct StatusOptions {
    /// Only check if the PR is ready to land (exit 0 = ready, exit 4 = not ready)
    #[clap(long)]
    pub ready: bool,

    /// Show status of all commits in the branch, not just HEAD
    #[clap(long, short = 'a')]
    pub all: bool,
}

pub async fn status(
    opts: StatusOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    let remote_tip = forge.fetch_branch(config.default_branch_name())?;
    let prepared_commits =
        crate::forge::get_prepared_commits(git, config, forge, remote_tip)?;

    if prepared_commits.is_empty() {
        return Err(eyre!("No commits on branch"));
    }

    let commits_to_check = if opts.all {
        &prepared_commits[..]
    } else {
        &prepared_commits[prepared_commits.len() - 1..]
    };

    let mut all_ready = true;

    for pc in commits_to_check {
        let Some(pr_number) = pc.pull_request_number else {
            output_essential(&format!(
                "{} | no pr | {}",
                &pc.short_id,
                pc.message
                    .get(&crate::message::MessageSection::Title)
                    .map_or("(untitled)", std::string::String::as_str)
            ))?;
            all_ready = false;
            continue;
        };

        let pr = forge
            .get_change_request(pr_number)
            .await?
            .ok_or_else(|| eyre!("PR #{} not found", pr_number))?;

        let review = match &pr.review_status {
            Some(crate::forge::ReviewStatus::Approved) => "approved",
            Some(crate::forge::ReviewStatus::Rejected) => "changes requested",
            Some(crate::forge::ReviewStatus::Requested) => "review pending",
            None => "no review",
        };

        if pr.review_status != Some(crate::forge::ReviewStatus::Approved) {
            all_ready = false;
        }

        let url = forge.change_request_url(pr_number);

        output_essential(&format!(
            "#{} | {} | {} | {}",
            pr_number, review, pr.title, url
        ))?;
    }

    if opts.ready && !all_ready {
        return Err(eyre!("not all PRs are approved"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::forge::{
        ChangeRequest, ChangeRequestState, ForgeApiMock, ReviewStatus,
    };
    use crate::test_helpers::{TestRepo, test_config};
    use unimock::*;

    fn base_clauses(base_oid: git2::Oid) -> impl Clause {
        ForgeApiMock::fetch_branch
            .some_call(matching!(_))
            .returns(Ok(base_oid))
    }

    fn make_cr(number: u64, review: Option<ReviewStatus>) -> ChangeRequest {
        ChangeRequest {
            number,
            title: format!("PR #{number}"),
            body: None,
            base_ref_name: "main".into(),
            base_oid: git2::Oid::zero(),
            head_ref_name: format!("spr/main/pr-{number}"),
            head_oid: git2::Oid::zero(),
            is_draft: false,
            state: ChangeRequestState::Open,
            sections: BTreeMap::default(),
            reviewers: std::collections::HashMap::default(),
            review_status: review,
            merge_commit: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_commits_errors() {
        let test_repo = TestRepo::new();
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = StatusOptions {
            ready: false,
            all: false,
        };
        let result = status(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No commits"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn commit_without_pr() {
        let test_repo = TestRepo::new();
        test_repo.add_commit("feat: add widget\n\nSome description");
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(base_clauses(test_repo.base_oid));

        let opts = StatusOptions {
            ready: false,
            all: false,
        };
        let result = status(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "status should succeed: {result:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ready_fails_when_not_approved() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: widget\n\nPull Request: https://github.com/test-owner/test-repo/pull/42",
        );
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new((
            base_clauses(test_repo.base_oid),
            ForgeApiMock::parse_cr_field
                .some_call(matching!(_))
                .returns(Ok(Some(42))),
            ForgeApiMock::change_request_url
                .each_call(matching!(42))
                .returns("https://github.com/test-owner/test-repo/pull/42".to_string()),
            ForgeApiMock::get_change_request
                .some_call(matching!(_))
                .returns(Ok(Some(make_cr(
                    42,
                    Some(ReviewStatus::Requested),
                )))),
        ));

        let opts = StatusOptions {
            ready: true,
            all: false,
        };
        let result = status(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not all PRs are approved"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ready_succeeds_when_approved() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: widget\n\nPull Request: https://github.com/test-owner/test-repo/pull/42",
        );
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new((
            base_clauses(test_repo.base_oid),
            ForgeApiMock::parse_cr_field
                .some_call(matching!(_))
                .returns(Ok(Some(42))),
            ForgeApiMock::change_request_url
                .each_call(matching!(42))
                .returns("https://github.com/test-owner/test-repo/pull/42".to_string()),
            ForgeApiMock::get_change_request
                .some_call(matching!(_))
                .returns(Ok(Some(make_cr(
                    42,
                    Some(ReviewStatus::Approved),
                )))),
        ));

        let opts = StatusOptions {
            ready: true,
            all: false,
        };
        let result = status(opts, &git, &forge, &config).await;
        assert!(
            result.is_ok(),
            "status --ready should succeed when approved: {result:?}"
        );
    }
}
