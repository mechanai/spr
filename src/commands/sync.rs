/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Result, WrapErr as _};

use crate::output::{output, output_essential};

#[derive(Debug, clap::Parser)]
pub struct SyncOptions {
    /// Also create/update PRs after rebasing (equivalent to running
    /// spr diff --all after the rebase)
    #[clap(long)]
    pub update: bool,

    /// Message to use for update commits on existing PRs
    #[clap(long, short = 'm')]
    pub message: Option<String>,
}

pub async fn sync(
    opts: SyncOptions,
    git: &crate::git::Git,
    gh: &mut crate::github::GitHub,
    config: &crate::config::Config,
) -> Result<()> {
    git.check_no_uncommitted_changes()?;

    // Fetch current master from upstream
    output("🔄", &format!("Fetching {}", config.master_ref.branch_name()))?;
    let new_master_oid =
        gh.remote().fetch_branch(config.master_ref.branch_name())?;

    // Get the prepared commits (these are the local commits above master)
    let mut prepared_commits = gh.get_prepared_commits()?;

    if prepared_commits.is_empty() {
        output_essential("already up to date, no local commits")?;
        return Ok(());
    }

    // Check if we're already based on the latest master
    let current_base = prepared_commits.first().unwrap().parent_oid;
    if current_base == new_master_oid {
        output_essential("already up to date")?;
    } else {
        output(
            "⚾",
            &format!(
                "Rebasing {} commit(s) onto {}",
                prepared_commits.len(),
                config.master_ref.branch_name()
            ),
        )?;

        git.rebase_commits(&mut prepared_commits, new_master_oid)
            .wrap_err("Rebase failed — please rebase manually and run spr diff --all")?;

        output_essential(&format!(
            "rebased {} commit(s)",
            prepared_commits.len()
        ))?;
    }

    // Optionally update PRs
    if opts.update {
        output("📤", "Updating pull requests")?;

        let diff_opts = crate::commands::diff::DiffOptions {
            all: true,
            update_message: false,
            draft: false,
            message: opts.message,
            refs: None,
            cherry_pick: false,
            label: vec![],
            count: None,
        };

        crate::commands::diff::diff(diff_opts, git, gh, config).await?;
    }

    Ok(())
}
