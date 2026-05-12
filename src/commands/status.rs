/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, eyre};

use crate::{
    github::{GitHub, ReviewStatus},
    output::output_essential,
};

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
    _git: &crate::git::Git,
    gh: &mut GitHub,
    config: &crate::config::Config,
) -> Result<()> {
    let prepared_commits = gh.get_prepared_commits()?;

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

        let pr = gh.get_pull_request(pr_number).await?;

        let review = match &pr.review_status {
            Some(ReviewStatus::Approved) => "approved",
            Some(ReviewStatus::Rejected) => "changes requested",
            Some(ReviewStatus::Requested) => "review pending",
            None => "no review",
        };

        if pr.review_status != Some(ReviewStatus::Approved) {
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
