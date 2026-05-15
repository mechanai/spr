/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, eyre};

use crate::{
    message::validate_commit_message,
    output::{output, write_commit_title},
};

#[derive(Debug, clap::Parser)]
pub struct AmendOptions {
    /// Amend all commits in branch, not just HEAD
    #[clap(long, short = 'a')]
    all: bool,
}

pub async fn amend(
    opts: AmendOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    let remote_tip = forge.fetch_branch(config.default_branch_name())?;
    let mut pc = crate::forge::get_prepared_commits(git, config, forge, remote_tip)?;

    let len = pc.len();
    if len == 0 {
        output("👋", "Branch is empty - nothing to do. Good bye!")?;
        return Ok(());
    }

    // The slice of prepared commits we want to operate on.
    let slice = if opts.all {
        &mut pc[..]
    } else {
        &mut pc[len - 1..]
    };

    // Request the Pull Request information for each commit (well, those that
    // declare to have Pull Requests). This list is in reverse order, so that
    // below we can pop from the vector as we iterate.
    let mut pull_requests: Vec<Option<Option<crate::forge::ChangeRequest>>> =
        Vec::new();
    for pc in slice.iter().rev() {
        if let Some(number) = pc.pull_request_number {
            pull_requests.push(Some(forge.get_change_request(number).await?));
        } else {
            pull_requests.push(None);
        }
    }

    let mut failure = false;

    for commit in slice.iter_mut() {
        write_commit_title(commit)?;
        let pull_request = pull_requests.pop().flatten();
        if let Some(Some(change_request)) = pull_request {
            commit.message = change_request.sections;
        }
        failure = validate_commit_message(&commit.message, config).is_err()
            || failure;
    }
    git.rewrite_commit_messages(slice, None)?;

    if failure {
        Err(eyre!("amend failed"))
    } else {
        Ok(())
    }
}
