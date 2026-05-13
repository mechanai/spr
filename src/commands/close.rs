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

    let remote_tip = forge.fetch_branch(config.master_branch_name())?;
    let mut prepared_commits =
        crate::forge::get_prepared_commits(git, config, remote_tip)?;

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
            output("#️⃣ ", &format!("Pull Request #{number}"))?;
            number
        } else {
            Err(SprError::ChangeRequestState("This commit does not refer to a Pull Request.".into()))?
        };

    // Load Pull Request information
    let change_request = forge
        .get_change_request(pull_request_number)
        .await?
        .ok_or_else(|| {
        color_eyre::eyre::eyre!("PR #{} not found", pull_request_number)
    })?;

    if change_request.state != crate::forge::ChangeRequestState::Open {
        Err(SprError::ChangeRequestState("This Pull Request is already closed!".into()))?;
    }

    output("📖", "Getting started...")?;

    let base_is_master = config.is_master_branch(&change_request.base_ref_name);

    let result = forge.close_change_request(pull_request_number).await;

    match result {
        Ok(()) => (),
        Err(error) => {
            output("❌", "GitHub Pull Request close failed")?;

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

    if !base_is_master {
        push_specs.push(PushSpec {
            oid: None,
            remote_ref: &base_ref,
        });
    }

    forge.push_to_remote(&push_specs)?;

    Ok(())
}
