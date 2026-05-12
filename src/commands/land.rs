/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Error, Result, WrapErr as _, bail, eyre};
use indoc::formatdoc;
use std::time::Duration;

use crate::{
    forge::{ChangeRequestState, ChangeRequestUpdate, ReviewStatus},
    git_remote::PushSpec,
    message::build_github_body_for_merging,
    output::{output, write_commit_title},
};

#[derive(Debug, clap::Parser)]
pub struct LandOptions {
    /// Merge a Pull Request that was created or updated with spr diff
    /// --cherry-pick
    #[clap(long)]
    cherry_pick: bool,

    /// Land all approved PRs in the branch from bottom to top, stopping at
    /// the first that cannot be landed.
    #[clap(long, short = 'a')]
    all: bool,
}

pub async fn land(
    opts: LandOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    if !opts.all {
        return land_one(opts.cherry_pick, git, forge, config).await;
    }

    // --all: land commits from bottom to top
    let mut landed = 0u32;
    loop {
        let remote_tip = forge.fetch_branch(config.master_branch_name())?;
        let prepared_commits =
            crate::forge::get_prepared_commits(git, config, remote_tip)?;
        if prepared_commits.is_empty() {
            break;
        }

        // Find the first (bottom) commit that has a PR
        let first_with_pr = prepared_commits
            .iter()
            .find(|pc| pc.pull_request_number.is_some());

        if first_with_pr.is_none() {
            break;
        }

        // cherry_pick must be true when landing --all because each iteration
        // lands HEAD which has unlanded commits below it. land_one would reject
        // non-cherry-pick mode when based_on_unlanded_commits is true.
        match land_one(true, git, forge, config).await {
            Ok(()) => {
                landed += 1;
            }
            Err(e) => {
                if landed == 0 {
                    return Err(e);
                }
                // Landed some but hit a blocker — that's fine
                crate::output::output_essential(&format!(
                    "landed {landed} PR(s), stopped: {e}"
                ))?;
                return Ok(());
            }
        }
    }

    if landed > 0 {
        crate::output::output_essential(&format!("landed {landed} PR(s)"))?;
    } else {
        crate::output::output_essential("nothing to land")?;
    }
    Ok(())
}

async fn land_one(
    cherry_pick: bool,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    git.check_no_uncommitted_changes()?;
    let remote_tip = forge.fetch_branch(config.master_branch_name())?;
    let mut prepared_commits =
        crate::forge::get_prepared_commits(git, config, remote_tip)?;

    let based_on_unlanded_commits = prepared_commits.len() > 1;

    if based_on_unlanded_commits && !cherry_pick {
        return Err(Error::msg(formatdoc!(
            "Cannot land a commit whose parent is not on {master}. To land \
             this commit, rebase it so that it is a direct child of {master}.
             Alternatively, if you used the `--cherry-pick` option with `spr \
             diff`, then you can pass it to `spr land`, too.",
            master = &config.master_branch_name(),
        )));
    }

    let Some(prepared_commit) = prepared_commits.last_mut() else {
        output("👋", "Branch is empty - nothing to do. Good bye!")?;
        return Ok(());
    };

    write_commit_title(prepared_commit)?;

    let pull_request_number =
        if let Some(number) = prepared_commit.pull_request_number {
            output("#️⃣ ", &format!("Pull Request #{number}"))?;
            number
        } else {
            bail!("This commit does not refer to a Pull Request.");
        };

    // Load Pull Request information
    let change_request = forge
        .get_change_request(pull_request_number)
        .await?
        .ok_or_else(|| {
        eyre!("Pull Request #{} not found", pull_request_number)
    })?;

    if change_request.state != ChangeRequestState::Open {
        bail!("This Pull Request is already closed!");
    }

    if config.require_approval
        && change_request.review_status != Some(ReviewStatus::Approved)
    {
        bail!("This Pull Request has not been approved on GitHub.");
    }

    output("🛫", "Getting started...")?;

    // Fetch current master from GitHub.
    let current_master = forge.fetch_branch(config.master_branch_name())?;

    let base_is_master = config.is_master_branch(&change_request.base_ref_name);
    let index = git.cherrypick(prepared_commit.oid, current_master)?;

    if index.has_conflicts() {
        return Err(Error::msg(formatdoc!(
            "This commit cannot be applied on top of the '{master}' branch.
             Please rebase this commit.{unlanded}",
            master = &config.master_branch_name(),
            unlanded = if based_on_unlanded_commits {
                " You may also have to land commits that this commit depends on first."
            } else {
                ""
            },
        )));
    }

    // This is the tree we are getting from cherrypicking the local commit
    // on the selected base (master or stacked-on Pull Request).
    let our_tree_oid = git.write_index(index)?;

    // Now let's predict what merging the PR into the master branch would
    // produce.
    let merge_index = {
        let repo = git.repo();
        let current_master = repo.find_commit(current_master)?;
        let pr_head = repo.find_commit(change_request.head_oid)?;
        repo.merge_commits(&current_master, &pr_head, None)
    }?;

    let merge_matches_cherrypick = if merge_index.has_conflicts() {
        false
    } else {
        let merge_tree_oid = git.write_index(merge_index)?;
        merge_tree_oid == our_tree_oid
    };

    if !merge_matches_cherrypick {
        return Err(Error::msg(formatdoc!(
            "This commit has been updated and/or rebased since the pull \
             request was last updated. Please run `spr diff` to update the \
             pull request and then try `spr land` again!"
        )));
    }

    // Okay, we are confident now that the PR can be merged and the result of
    // that merge would be a master commit with the same tree as if we
    // cherry-picked the commit onto master.
    let mut pr_head_oid = change_request.head_oid;

    if !base_is_master {
        // The base of the Pull Request on GitHub is not set to master. This
        // means the Pull Request uses a base branch. We tested above that
        // merging the Pull Request branch into the master branch produces the
        // intended result (the same as cherry-picking the local commit onto
        // master), so what we want to do is actually merge the Pull Request as
        // it is into master. Hence, we change the base to the master branch.
        //
        // Before we do that, there is one more edge case to look out for: if
        // the base branch contains changes that have since been landed on
        // master, then Git might be able to figure out that these changes
        // appear both in the pull request branch (via the merge branch) and in
        // master, but are identical in those two so it is not a merge conflict
        // but can go ahead. The result of this in master if we merge now is
        // correct, but there is one problem: when looking at the Pull Request
        // in GitHub after merging, it will show these change as part of the
        // Pull Request. So when you look at the changed files of the Pull
        // Request, you will see both changes in this commit (great!) and those
        // in the base branch (a previous commit that has already been landed on
        // master - not great!). This is because the changes shown are the ones
        // that happened on this Pull Request branch (now including the base
        // branch) since it branched off master. This can include changes in the
        // base branch that are already on master, but were added to master
        // after the Pull Request branch branched from master.
        // The solution is to merge current master into the Pull Request branch.
        // Doing that now means that the final changes done by this Pull Request
        // are only the changes that are not yet in master. That's what we want.
        // This final merge never introduces any changes to the Pull Request. In
        // fact, the tree that we use for the merge commit is the one we got
        // above from the cherry-picking of this commit on master.

        // The commit on the base branch that the PR branch is currently based on
        let pr_base_oid = git
            .repo()
            .merge_base(pr_head_oid, change_request.base_oid)?;
        let pr_base_tree = git.get_tree_oid_for_commit(pr_base_oid)?;

        let pr_master_base =
            git.repo().merge_base(pr_base_oid, current_master)?;
        let pr_master_base_tree =
            git.get_tree_oid_for_commit(pr_master_base)?;

        if pr_base_tree != pr_master_base_tree {
            // So the current file contents of the base branch are not the same
            // as those of the master branch commit that the base branch is
            // based on. In other words, the base branch is currently not
            // "empty". Or, the base branch has changes in them. These changes
            // must all have been landed on master in the meantime (after this
            // base branch was branched off) or otherwise we would have aborted
            // this whole operation further above. But in order not to show them
            // as part of this Pull Request after landing, we have to make clear
            // those are changes in master, not in this Pull Request.
            // Here comes the additional merge-in-master commit on the Pull
            // Request branch that achieves that!

            pr_head_oid = git.create_derived_commit(
                pr_head_oid,
                &format!(
                    "[𝘀𝗽𝗿] landed version\n\nCreated using spr {}",
                    env!("CARGO_PKG_VERSION"),
                ),
                our_tree_oid,
                &[pr_head_oid, current_master],
            )?;

            forge
                .push_to_remote(&[PushSpec {
                    oid: Some(pr_head_oid),
                    remote_ref: &format!(
                        "refs/heads/{}",
                        change_request.head_ref_name
                    ),
                }])
                .wrap_err("git push failed")?;
        }

        forge
            .update_change_request(
                pull_request_number,
                &ChangeRequestUpdate {
                    base: Some(config.master_branch_name().to_string()),
                    ..Default::default()
                },
                None,
            )
            .await?;
    }

    // Check whether GitHub says this PR is mergeable. This happens in a
    // retry-loop because recent changes to the Pull Request can mean that
    // GitHub has not finished the mergeability check yet.
    let mut attempts = 0;
    let result = loop {
        attempts += 1;

        let mergeability = forge.get_mergeability(pull_request_number).await?;

        if mergeability.head_oid != pr_head_oid {
            break Err(eyre!(
                "The Pull Request seems to have been updated externally. Please try again!"
            ));
        }

        if config.is_master_branch(&mergeability.base_ref_name)
            && mergeability.mergeable.is_some()
        {
            if mergeability.mergeable != Some(true) {
                break Err(Error::msg(formatdoc!(
                    "GitHub concluded the Pull Request is not mergeable at \
                    this point. Please rebase your changes and try again!"
                )));
            }

            if let Some(merge_commit) = mergeability.merge_commit {
                forge.fetch_from_remote(&[], &[merge_commit])?;

                if git.get_tree_oid_for_commit(merge_commit)? != our_tree_oid {
                    return Err(Error::msg(formatdoc!(
                    "This commit has been updated and/or rebased since the pull
                     request was last updated. Please run `spr diff` to update the pull
                     request and then try `spr land` again!"
                )));
                }
            }

            break Ok(());
        }

        if attempts >= 10 {
            // After ten failed attempts we give up.
            break Err(eyre!(
                "GitHub Pull Request did not update. Please try again!"
            ));
        }

        // Wait one second before retrying
        tokio::time::sleep(Duration::from_secs(1)).await;
    };

    let result = match result {
        Ok(()) => {
            forge
                .merge_change_request(
                    pull_request_number,
                    config.merge_method,
                    &change_request.title,
                    &build_github_body_for_merging(&change_request.sections),
                    pr_head_oid,
                )
                .await
        }
        Err(err) => Err(err),
    };

    match result {
        Ok(()) => {
            output("🛬", "Landed!")?;

            // Fetch updated master to rebase on
            let new_master = forge.fetch_branch(config.master_branch_name())?;
            git.rebase_commits(&mut prepared_commits[..], new_master)
                .context(
                    "The automatic rebase failed - please rebase manually!"
                        .to_string(),
                )?;
        }
        Err(mut error) => {
            output("❌", "GitHub Pull Request merge failed")?;

            // If we changed the target branch of the Pull Request earlier,
            // then undo this change now.
            if !base_is_master {
                let result = forge
                    .update_change_request(
                        pull_request_number,
                        &ChangeRequestUpdate {
                            base: Some(change_request.base_ref_name.clone()),
                            ..Default::default()
                        },
                        None,
                    )
                    .await;
                if let Err(e) = result {
                    error = error.wrap_err(e);
                }
            }

            return Err(error);
        }
    }

    let head_remote_ref =
        format!("refs/heads/{}", change_request.head_ref_name);
    let mut push_specs = vec![PushSpec {
        oid: None,
        remote_ref: &head_remote_ref,
    }];

    let base_remote_ref =
        format!("refs/heads/{}", change_request.base_ref_name);
    if !base_is_master {
        push_specs.push(PushSpec {
            oid: None,
            remote_ref: &base_remote_ref,
        });
    }

    forge.push_to_remote(&push_specs)?;

    Ok(())
}
