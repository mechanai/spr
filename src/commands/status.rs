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
        crate::forge::get_prepared_commits(git, config, remote_tip)?;

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

        let url = config.pull_request_url(pr_number);

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
