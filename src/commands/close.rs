/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::Result;

use crate::{
    error::SprError,
    git::PreparedCommit,
    git_remote::PushSpec,
    message::MessageSection,
    output::{output, write_commit_title},
};

#[derive(Debug, clap::Parser)]
pub struct CloseOptions {
    /// Close Pull Requests for the whole branch, not just the HEAD commit
    #[clap(long, short = 'a')]
    all: bool,
}

pub async fn close(
    opts: CloseOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    let mut result = Ok(());

    let remote_tip = forge.fetch_branch(config.default_branch_name())?;
    let mut prepared_commits =
        crate::forge::get_prepared_commits(git, config, forge, remote_tip)?;

    if prepared_commits.is_empty() {
        output("👋", "Branch is empty - nothing to do. Good bye!")?;
        return result;
    }

    if !opts.all {
        // Remove all prepared commits from the vector but the last. So, if
        // `--all` is not given, we only operate on the HEAD commit.
        prepared_commits.drain(0..prepared_commits.len() - 1);
    }

    for prepared_commit in &mut prepared_commits {
        if result.is_err() {
            break;
        }

        write_commit_title(prepared_commit)?;

        // The further implementation of the close command is in a separate function.
        // This makes it easier to run the code to update the local commit message
        // with all the changes that the implementation makes at the end, even if
        // the implementation encounters an error or exits early.
        result = close_impl(forge, config, prepared_commit).await;
    }

    // This updates the commit message in the local Git repository (if it was
    // changed by the implementation)
    git.rewrite_commit_messages(prepared_commits.as_mut_slice(), None)?;

    result
}

async fn close_impl(
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
    prepared_commit: &mut PreparedCommit,
) -> Result<()> {
    let pull_request_number =
        if let Some(number) = prepared_commit.pull_request_number {
            output("#️⃣ ", &format!("{} #{number}", forge.change_request_term_full()))?;
            number
        } else {
            Err(SprError::ChangeRequestState(format!("This commit does not refer to a {}.", forge.change_request_term_full())))?
        };

    // Load Pull Request information
    let change_request = forge
        .get_change_request(pull_request_number)
        .await?
        .ok_or_else(|| {
        color_eyre::eyre::eyre!("{} #{} not found", forge.change_request_term(), pull_request_number)
    })?;

    if change_request.state != crate::forge::ChangeRequestState::Open {
        Err(SprError::ChangeRequestState(format!("This {} is already closed!", forge.change_request_term_full())))?;
    }

    output("📖", "Getting started...")?;

    let base_is_default_branch = config.is_default_branch(&change_request.base_ref_name);

    let result = forge.close_change_request(pull_request_number).await;

    match result {
        Ok(()) => (),
        Err(error) => {
            output("❌", &format!("{} close failed", forge.change_request_term_full()))?;

            return Err(error);
        }
    }

    output("📕", "Closed!")?;

    // Remove sections from commit that are not relevant after closing.
    prepared_commit.message.remove(&MessageSection::PullRequest);
    prepared_commit.message.remove(&MessageSection::ReviewedBy);

    let head_ref = format!("refs/heads/{}", change_request.head_ref_name);
    let base_ref = format!("refs/heads/{}", change_request.base_ref_name);

    let mut push_specs = vec![PushSpec {
        oid: None,
        remote_ref: &head_ref,
    }];

    if !base_is_default_branch {
        push_specs.push(PushSpec {
            oid: None,
            remote_ref: &base_ref,
        });
    }

    forge.push_to_remote(&push_specs)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::forge::{
        ChangeRequest, ChangeRequestState, ForgeApiMock,
    };
    use crate::test_helpers::{TestRepo, test_config};
    use unimock::*;

    fn make_open_cr(number: u64, head: &str, base: &str) -> ChangeRequest {
        ChangeRequest {
            number,
            title: format!("PR #{number}"),
            body: None,
            base_ref_name: base.into(),
            base_oid: git2::Oid::zero(),
            head_ref_name: head.into(),
            head_oid: git2::Oid::zero(),
            is_draft: false,
            state: ChangeRequestState::Open,
            sections: BTreeMap::default(),
            reviewers: std::collections::HashMap::default(),
            review_status: None,
            merge_commit: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_empty_branch_succeeds() {
        let test_repo = TestRepo::new();
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new(
            ForgeApiMock::fetch_branch
                .some_call(matching!(_))
                .returns(Ok(test_repo.base_oid)),
        );

        let opts = CloseOptions { all: false };
        let result = close(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "close on empty branch should succeed: {result:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_commit_without_pr_errors() {
        let test_repo = TestRepo::new();
        test_repo.add_commit("feat: no pr here\n\nJust a commit");
        let git = test_repo.git();
        let config = test_config();

        let forge = Unimock::new((
            ForgeApiMock::fetch_branch
                .some_call(matching!(_))
                .returns(Ok(test_repo.base_oid)),
            ForgeApiMock::change_request_term_full
                .some_call(matching!())
                .returns("Pull Request"),
        ));

        let opts = CloseOptions { all: false };
        let result = close(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not refer to a Pull Request"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_already_closed_errors() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: widget\n\nPull Request: https://github.com/test-owner/test-repo/pull/10",
        );
        let git = test_repo.git();
        let config = test_config();

        let mut cr = make_open_cr(10, "spr/main/widget", "main");
        cr.state = ChangeRequestState::Closed;

        let forge = Unimock::new((
            ForgeApiMock::fetch_branch
                .some_call(matching!(_))
                .returns(Ok(test_repo.base_oid)),
            ForgeApiMock::parse_cr_field
                .some_call(matching!(_))
                .returns(Ok(Some(10))),
            ForgeApiMock::change_request_url
                .some_call(matching!(10))
                .returns("https://github.com/test-owner/test-repo/pull/10".to_string()),
            ForgeApiMock::get_change_request
                .some_call(matching!(_))
                .returns(Ok(Some(cr))),
            ForgeApiMock::change_request_term_full
                .each_call(matching!())
                .returns("Pull Request"),
        ));

        let opts = CloseOptions { all: false };
        let result = close(opts, &git, &forge, &config).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("already closed"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_open_pr_succeeds() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: widget\n\nPull Request: https://github.com/test-owner/test-repo/pull/10",
        );
        let git = test_repo.git();
        let config = test_config();

        let cr = make_open_cr(10, "spr/main/widget", "main");

        let forge = Unimock::new((
            ForgeApiMock::fetch_branch
                .some_call(matching!(_))
                .returns(Ok(test_repo.base_oid)),
            ForgeApiMock::parse_cr_field
                .some_call(matching!(_))
                .returns(Ok(Some(10))),
            ForgeApiMock::change_request_url
                .some_call(matching!(10))
                .returns("https://github.com/test-owner/test-repo/pull/10".to_string()),
            ForgeApiMock::get_change_request
                .some_call(matching!(_))
                .returns(Ok(Some(cr))),
            ForgeApiMock::close_change_request
                .some_call(matching!(_))
                .returns(Ok(())),
            ForgeApiMock::push_to_remote
                .some_call(matching!(_))
                .returns(Ok(())),
            ForgeApiMock::change_request_term_full
                .some_call(matching!())
                .returns("Pull Request"),
        ));

        let opts = CloseOptions { all: false };
        let result = close(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "close should succeed: {result:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_removes_pr_section_from_commit() {
        let test_repo = TestRepo::new();
        test_repo.add_commit(
            "feat: widget\n\nSome summary\n\nPull Request: https://github.com/test-owner/test-repo/pull/10",
        );
        let git = test_repo.git();
        let config = test_config();

        let cr = make_open_cr(10, "spr/main/widget", "main");

        let forge = Unimock::new((
            ForgeApiMock::fetch_branch
                .some_call(matching!(_))
                .returns(Ok(test_repo.base_oid)),
            ForgeApiMock::parse_cr_field
                .some_call(matching!(_))
                .returns(Ok(Some(10))),
            ForgeApiMock::change_request_url
                .some_call(matching!(10))
                .returns("https://github.com/test-owner/test-repo/pull/10".to_string()),
            ForgeApiMock::get_change_request
                .some_call(matching!(_))
                .returns(Ok(Some(cr))),
            ForgeApiMock::close_change_request
                .some_call(matching!(_))
                .returns(Ok(())),
            ForgeApiMock::push_to_remote
                .some_call(matching!(_))
                .returns(Ok(())),
            ForgeApiMock::change_request_term_full
                .some_call(matching!())
                .returns("Pull Request"),
        ));

        let opts = CloseOptions { all: false };
        let result = close(opts, &git, &forge, &config).await;
        assert!(result.is_ok(), "close should succeed: {result:?}");

        // Verify the commit message no longer contains the PR section
        let repo = git2::Repository::open(test_repo.dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let msg = head.message().unwrap();
        assert!(
            !msg.contains("Pull Request:"),
            "commit message should not contain Pull Request section after close, got: {msg}"
        );
        assert!(
            msg.contains("feat: widget"),
            "commit message should still contain title, got: {msg}"
        );
    }
}
