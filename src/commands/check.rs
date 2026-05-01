/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, eyre};

use crate::{
    message::validate_commit_message,
    output::output_essential,
};

#[derive(Debug, clap::Parser)]
pub struct CheckOptions {
    /// Also verify cherry-pick would succeed (requires fetch)
    #[clap(long)]
    pub cherry_pick: bool,
}

pub async fn check(
    opts: CheckOptions,
    git: &crate::git::Git,
    gh: &mut crate::github::GitHub,
    config: &crate::config::Config,
) -> Result<()> {
    // Check for uncommitted changes
    git.check_no_uncommitted_changes()?;

    let prepared_commits = gh.get_prepared_commits()?;

    let head_commit = prepared_commits
        .last()
        .ok_or_else(|| eyre!("No commits on branch"))?;

    // Validate commit message
    let mut message = head_commit.message.clone();
    validate_commit_message(&mut message, config)?;
    output_essential("message: ok")?;

    // Check if PR already exists
    if let Some(number) = head_commit.pull_request_number {
        output_essential(&format!("pr: #{}", number))?;
    } else {
        output_essential("pr: new")?;
    }

    // Cherry-pick conflict check
    if opts.cherry_pick {
        let master_oid = if let Some(first) = prepared_commits.first() {
            first.parent_oid
        } else {
            return Ok(());
        };

        let index = git.cherrypick(head_commit.oid, master_oid)?;
        if index.has_conflicts() {
            output_essential("cherry-pick: conflict")?;
            return Err(eyre!("Cherry-pick would conflict"));
        } else {
            output_essential("cherry-pick: clean")?;
        }
    }

    output_essential("check: pass")?;
    Ok(())
}
