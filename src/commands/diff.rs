/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::collections::HashSet;
use std::iter::zip;

use color_eyre::eyre::{Error, Result, WrapErr as _, bail, eyre};
use crate::error::SprError;

use crate::{
    forge::{
        ChangeRequest, ChangeRequestState, ChangeRequestUpdate, ReviewerRequest,
    },
    git::PreparedCommit,
    git_remote::PushSpec,
    message::{MessageSection, validate_commit_message},
    output::{output, output_essential, write_commit_title},
    utils::{parse_name_list, remove_all_parens, slugify},
};
use git2::Oid;


#[derive(Debug, clap::Parser)]
pub struct DiffOptions {
    /// Create/update pull requests for the whole branch, not just the HEAD commit
    #[clap(long, short = 'a')]
    pub all: bool,

    /// Update the pull request title and description on GitHub from the local
    /// commit message
    #[clap(long)]
    pub update_message: bool,

    /// Submit any new Pull Request as a draft
    #[clap(long)]
    pub draft: bool,

    /// Labels to apply to new Pull Requests (can be specified multiple times)
    #[clap(long, short = 'l')]
    pub label: Vec<String>,

    /// Message to be used for commits updating existing pull requests (e.g.
    /// 'rebase' or 'review comments')
    #[clap(long, short = 'm')]
    pub message: Option<String>,

    /// Which commits in the branch should be created/updated. This can be a
    /// revspec such as HEAD~4..HEAD~1 or just one commit like HEAD~7.
    #[clap(long, short = 'r')]
    pub refs: Option<String>,

    /// Submit this commit as if it was cherry-picked on the default branch. Do not base it
    /// on any intermediate changes between the default branch and this commit.
    #[clap(long)]
    pub cherry_pick: bool,

    /// Maximum number of commits to process (from HEAD backwards)
    #[clap(long, short = 'n')]
    pub count: Option<usize>,
}

fn get_oids(refs: &str, repo: &git2::Repository) -> Result<HashSet<Oid>> {
    // refs might be a single (eg 012345abc or HEAD) or a range (HEAD~4..HEAD~2)
    let revspec = repo.revparse(refs)?;

    let from = revspec
        .from()
        .ok_or_else(|| eyre!("Unexpectedly no from id in range"))?
        .id();
    if revspec.mode().contains(git2::RevparseMode::SINGLE) {
        // simple case, just return the id
        return Ok(HashSet::from([from]));
    }
    let to = revspec
        .to()
        .ok_or_else(|| eyre!("Unexpectedly no to id in range"))?
        .id();

    let mut walk = repo.revwalk()?;
    walk.push(to)?;
    walk.hide(from)?;
    walk.map(|r| Ok(r?)).collect()
}

pub async fn diff(
    opts: DiffOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    if opts.count.is_some() && !opts.all {
        bail!("--count requires --all");
    }

    // Abort right here if the local Git repository is not clean
    git.check_no_uncommitted_changes()?;

    let mut result = Ok(());

    // Look up the commits on the local branch
    let remote_tip = forge.fetch_branch(config.default_branch_name())?;
    let mut prepared_commits =
        crate::forge::get_prepared_commits(git, forge, remote_tip)?;

    // The parent of the first commit in the list is the commit on the default branch that
    // the local branch is based on
    let default_branch_base_oid = if let Some(first_commit) = prepared_commits.first() {
        first_commit.parent_oid
    } else {
        output("👋", "Branch is empty - nothing to do. Good bye!")?;
        return result;
    };

    // If refs is set, we want to track which commits to run `diff` against. The
    // simple approach would be to adjust the prepared_commits Vec (as with
    // opts.all above). This does not work however, as we need to know the
    // entire list (or more specifically the list after the first update) for
    // the rewrite_commit_messages step. This is not a problem for opts.all as
    // it only ever has a single commit to update, and so nothing after it.
    let revs_to_pr = match (opts.refs.as_deref(), opts.all) {
        (Some(refs), false) => Some(get_oids(refs, git.repo())?),
        (Some(_), true) => {
            bail!("Do not use --refs with --all");
        }
        (None, true) => {
            // Operate on all commits (possibly limited by --count)
            if let Some(count) = opts.count {
                let len = prepared_commits.len();
                if count < len {
                    prepared_commits.drain(0..len - count);
                }
            }
            None
        }
        (None, false) => {
            // Only operate on the HEAD commit.
            prepared_commits.drain(0..prepared_commits.len() - 1);
            None
        }
    };

    // Fetch change requests sequentially (can't use spawn_local with &dyn ForgeApi)
    let mut pull_request_tasks: Vec<Option<Option<ChangeRequest>>> = Vec::new();
    for pc in &prepared_commits {
        if revs_to_pr
            .as_ref()
            .is_none_or(|revs| revs.contains(&pc.oid))
        {
            if let Some(number) = pc.pull_request_number {
                pull_request_tasks
                    .push(Some(forge.get_change_request(number).await?));
            } else {
                pull_request_tasks.push(Some(None));
            }
        } else {
            pull_request_tasks.push(None);
        }
    }

    let mut message_on_prompt = String::new();

    // Build stack info for PR body (only if multiple commits)
    let stack_info = if prepared_commits.len() > 1 {
        let mut lines = vec!["**Stack:**".to_string()];
        for pc in prepared_commits.iter().rev() {
            let raw_title = pc
                .message
                .get(&MessageSection::Title)
                .map_or("(untitled)", std::string::String::as_str);
            let title = crate::stack::sanitize_title(raw_title);
            if let Some(number) = pc.pull_request_number {
                lines.push(format!("- #{number} {title}"));
            } else {
                lines.push(format!("- ⏳ {title}"));
            }
        }
        Some(lines.join("\n"))
    } else {
        None
    };

    for (prepared_commit, pull_request_task) in
        zip(prepared_commits.iter_mut(), pull_request_tasks)
    {
        if result.is_err() {
            break;
        }

        // Check whether to skip this commit because we have a hashset of oids
        // to operate on, but it doesn't contain this commit oid
        if revs_to_pr
            .as_ref()
            .is_some_and(|revs| !revs.contains(&prepared_commit.oid))
        {
            continue;
        }

        let change_request = pull_request_task.flatten();

        write_commit_title(prepared_commit)?;

        // Skip WIP commits — don't create or update PRs for them
        let title = prepared_commit
            .message
            .get(&MessageSection::Title)
            .map_or("", std::string::String::as_str);
        if title.starts_with("WIP")
            || title.starts_with("wip")
            || title.starts_with("[WIP]")
            || title.starts_with("[wip]")
        {
            // Only skip if there's no existing PR for this commit
            if prepared_commit.pull_request_number.is_none() {
                output("⏭️ ", "Skipping WIP commit")?;
                continue;
            }
        }

        // The further implementation of the diff command is in a separate
        // function. This makes it easier to run the code to update the local
        // commit message with all the changes that the implementation makes at
        // the end, even if the implementation encounters an error or exits
        // early.
        result = diff_impl(
            &opts,
            &mut message_on_prompt,
            git,
            forge,
            config,
            prepared_commit,
            default_branch_base_oid,
            change_request,
            stack_info.as_deref(),
        )
        .await;
    }

    // This updates the commit message in the local Git repository (if it was
    // changed by the implementation). Skip in dry-run to avoid writing bogus
    // PR URLs (e.g. PR #0) into local commit history.
    if !forge.is_dry_run() {
        git.rewrite_commit_messages(prepared_commits.as_mut_slice(), None)?;
    }

    result
}

#[allow(clippy::too_many_arguments)]
async fn diff_impl(
    opts: &DiffOptions,
    message_on_prompt: &mut String,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
    local_commit: &mut PreparedCommit,
    default_branch_base_oid: Oid,
    change_request: Option<ChangeRequest>,
    stack_info: Option<&str>,
) -> Result<()> {
    // Parsed commit message of the local commit
    let message = &mut local_commit.message;

    // Check if the local commit is based directly on the default branch.
    let directly_based_on_default_branch = local_commit.parent_oid == default_branch_base_oid;

    // Determine the trees the Pull Request branch and the base branch should
    // have when we're done here.
    let (new_head_tree, new_base_tree) = if !opts.cherry_pick
        || directly_based_on_default_branch
    {
        // Unless the user tells us to --cherry-pick, these should be the trees
        // of the current commit and its parent.
        // If the current commit is directly based on the default branch (i.e.
        // directly_based_on_default_branch is true), then we can do this here even when
        // the user tells us to --cherry-pick, because we would cherry pick the
        // current commit onto its parent, which gives us the same tree as the
        // current commit has, and the default branch base is the same as this commit's
        // parent.
        let head_tree = git.get_tree_oid_for_commit(local_commit.oid)?;
        let base_tree = git.get_tree_oid_for_commit(local_commit.parent_oid)?;

        (head_tree, base_tree)
    } else {
        // Cherry-pick the current commit onto the default branch
        let index = git.cherrypick(local_commit.oid, default_branch_base_oid)?;

        if index.has_conflicts() {
            bail!(
                "This commit cannot be cherry-picked on {default_branch}.",
                default_branch = config.default_branch_name(),
            );
        }

        // This is the tree we are getting from cherrypicking the local commit
        // on the default branch.
        let cherry_pick_tree = git.write_index(index)?;
        let default_branch_tree = git.get_tree_oid_for_commit(default_branch_base_oid)?;

        (cherry_pick_tree, default_branch_tree)
    };

    if let Some(number) = local_commit.pull_request_number {
        output(
            "#️⃣ ",
            &format!(
                "{} #{}: {}",
                forge.change_request_term_full(),
                number,
                forge.change_request_url(number)
            ),
        )?;
    }

    if local_commit.pull_request_number.is_none() || opts.update_message {
        validate_commit_message(message, config)?;
    }

    if let Some(ref cr) = change_request {
        if cr.state == ChangeRequestState::Closed
            || cr.state == ChangeRequestState::Merged
        {
            return Err(Error::msg(format!(
                "{} is closed. If you want to open a new one, \
                 remove the 'Pull Request' section from the commit message.",
                forge.change_request_term_full()
            )));
        }

        if !opts.update_message {
            let mut updates = ChangeRequestUpdate::default();
            updates.update_message(cr, message);

            if !updates.is_empty() {
                output(
                    "⚠️",
                    &format!(
                        "The {}'s title/message differ from the \
                         local commit's message.\n\
                         Use `spr diff --update-message` to overwrite the \
                          title and message on the remote with the local message, \
                         or `spr amend` to go the other way (rewrite the local \
                         commit message with what is on the remote).",
                        forge.change_request_term_full()
                    ),
                )?;
            }
        }
    }

    // Parse "Reviewers" section, if this is a new Pull Request
    let mut requested_reviewers = ReviewerRequest::default();

    if local_commit.pull_request_number.is_none() {
        // Use commit message reviewers, or fall back to config default
        let reviewers_text = message.get(&MessageSection::Reviewers).cloned();
        let reviewers_list = if let Some(ref text) = reviewers_text {
            parse_name_list(text)
        } else if !config.default_reviewers.is_empty() {
            config.default_reviewers.clone()
        } else {
            vec![]
        };

        if !reviewers_list.is_empty() {
            let mut checked_reviewers = Vec::new();

            for reviewer in reviewers_list {
                // Teams are indicated with a leading #
                if let Some(slug) = reviewer.strip_prefix('#') {
                    if let Some(team) =
                        forge.get_team(&config.owner, slug).await?
                    {
                        requested_reviewers.teams.push(team.slug);
                        checked_reviewers.push(reviewer);
                    } else {
                        bail!(
                            "Reviewers field contains unknown team '{}'",
                            reviewer,
                        );
                    }
                } else if let Some(user) = forge.get_user(&reviewer).await? {
                    requested_reviewers.users.push(user.login);
                    if let Some(name) = user.name {
                        checked_reviewers.push(format!(
                            "{} ({})",
                            reviewer.clone(),
                            remove_all_parens(&name)
                        ));
                    } else {
                        checked_reviewers.push(reviewer);
                    }
                } else {
                    bail!(
                        "Reviewers field contains unknown user '{}'",
                        reviewer
                    );
                }
            }

            message.insert(
                MessageSection::Reviewers,
                checked_reviewers.join(", "),
            );
        }
    }

    // Get the name of the existing Pull Request branch, or constuct one if
    // there is none yet.

    let title = message.get(&MessageSection::Title).map_or("", |t| &t[..]);

    let pull_request_branch: String = match &change_request {
        Some(cr) => cr.head_ref_name.clone(),
        None => forge
            .find_unused_branch_name(&config.branch_prefix, &slugify(title))?,
    };

    // Get the tree ids of the current head of the Pull Request, as well as the
    // base, and the commit id of the default branch commit this PR is currently based
    // on.
    // If there is no pre-existing Pull Request, we fill in the equivalent
    // values.
    let (pr_head_oid, pr_head_tree, pr_base_oid, pr_base_tree, pr_default_branch_base) =
        if let Some(cr) = &change_request {
            let pr_head_tree = git.get_tree_oid_for_commit(cr.head_oid)?;

            let current_default_branch_oid =
                forge.fetch_branch(config.default_branch_name())?;
            let pr_base_oid =
                git.repo().merge_base(cr.head_oid, cr.base_oid)?;
            let pr_base_tree = git.get_tree_oid_for_commit(pr_base_oid)?;

            let pr_default_branch_base =
                git.repo().merge_base(cr.head_oid, current_default_branch_oid)?;

            (
                cr.head_oid,
                pr_head_tree,
                pr_base_oid,
                pr_base_tree,
                pr_default_branch_base,
            )
        } else {
            let default_branch_base_tree =
                git.get_tree_oid_for_commit(default_branch_base_oid)?;
            (
                default_branch_base_oid,
                default_branch_base_tree,
                default_branch_base_oid,
                default_branch_base_tree,
                default_branch_base_oid,
            )
        };
    let needs_merging_default_branch = pr_default_branch_base != default_branch_base_oid;

    // At this point we can check if we can exit early because no update to the
    // existing Pull Request is necessary
    if let Some(ref cr) = change_request {
        // So there is an existing Pull Request...
        if !needs_merging_default_branch
            && pr_head_tree == new_head_tree
            && pr_base_tree == new_base_tree
        {
            // ...and it does not need a rebase, and the trees of both Pull
            // Request branch and base are all the right ones.
            output("✅", "No update necessary")?;

            if opts.update_message {
                // However, the user requested to update the commit message on
                // GitHub

                let mut updates = ChangeRequestUpdate::default();
                updates.update_message(cr, message);

                if !updates.is_empty() {
                    // ...and there are actual changes to the message
                    forge
                        .update_change_request(cr.number, &updates, stack_info)
                        .await?;
                    output("✍", &format!("Updated {} message remotely", forge.change_request_term_full().to_lowercase()))?;
                }
            }

            return Ok(());
        }
    }

    // Check if there is a base branch on GitHub already. That's the case when
    // there is an existing Pull Request, and its base is not the default branch.
    let base_branch: Option<String> = if let Some(ref cr) = change_request {
        if config.is_default_branch(&cr.base_ref_name) {
            None
        } else {
            Some(cr.base_ref_name.clone())
        }
    } else {
        None
    };

    // We are going to construct `pr_base_parent: Option<Oid>`.
    // The value will be the commit we have to merge into the new Pull Request
    // commit to reflect changes in the parent of the local commit (by rebasing
    // or changing commits between the default branch and this one, although technically
    // that's also rebasing).
    // If it's `None`, then we will not merge anything into the new Pull Request
    // commit.
    // If we are updating an existing PR, then there are three cases here:
    // (1) the parent tree of this commit is unchanged and we do not need to
    //     merge in the default branch, which means that the local commit was amended, but
    //     not rebased. We don't need to merge anything into the Pull Request
    //     branch.
    // (2) the parent tree has changed, but the parent of the local commit is on
    //     the default branch (or we are cherry-picking) and we are not already using a base
    //     branch: in this case we can merge the default branch commit we are based on
    //     into the PR branch, without going via a base branch. Thus, we don't
    //     introduce a base branch here and the PR continues to target the
    //     default branch.
    // (3) the parent tree has changed, and we need to use a base branch (either
    //     because one was already created earlier, or we find that we are not
    //     directly based on the default branch now): we need to construct a new commit for
    //     the base branch. That new commit's tree is always that of that local
    //     commit's parent (thus making sure that the difference between base
    //     branch and pull request branch are exactly the changes made by the
    //     local commit, thus the changes we want to have reviewed). The new
    //     commit may have one or two parents. The previous base is always a
    //     parent (that's either the current commit on an existing base branch,
    //     or the previous default branch commit the PR was based on if there isn't a
    //     base branch already). In addition, if the default branch commit this commit
    //     is based on has changed, (i.e. the local commit got rebased on newer
    //     default branch in the meantime) then we have to merge in that default branch commit,
    //     which will be the second parent.
    // If we are creating a new pull request then `pr_base_tree` (the current
    // base of the PR) was set above to be the tree of the default branch commit the
    // local commit is based one, whereas `new_base_tree` is the tree of the
    // parent of the local commit. So if the local commit for this new PR is on
    // the default branch, those two are the same (and we want to apply case 1). If the
    // commit is not directly based on the default branch, we have to create this new PR
    // with a base branch, so that is case 3.

    let (pr_base_parent, base_branch) =
        if pr_base_tree == new_base_tree && !needs_merging_default_branch {
            // Case 1
            (None, base_branch)
        } else if base_branch.is_none()
            && (directly_based_on_default_branch || opts.cherry_pick)
        {
            // Case 2
            (Some(default_branch_base_oid), None)
        } else {
            // Case 3

            // We are constructing a base branch commit.
            // One parent of the new base branch commit will be the current base
            // commit, that could be either the top commit of an existing base
            // branch, or a commit on the default branch.
            let mut parents = vec![pr_base_oid];

            // If we need to rebase on the default branch, make the default branch commit also a
            // parent (except if the first parent is that same commit, we don't
            // want duplicates in `parents`).
            if needs_merging_default_branch && pr_base_oid != default_branch_base_oid {
                parents.push(default_branch_base_oid);
            }

            let new_base_branch_commit = git.create_derived_commit(
                local_commit.parent_oid,
                &format!(
                    "[𝘀𝗽𝗿] {}\n\nCreated using spr {}\n\n[skip ci]",
                    if change_request.is_some() {
                        "changes introduced through rebase".to_string()
                    } else {
                        format!(
                            "changes to {} this commit is based on",
                            config.default_branch_name()
                        )
                    },
                    env!("CARGO_PKG_VERSION"),
                ),
                new_base_tree,
                &parents[..],
            )?;

            // If `base_branch` is `None` (which means a base branch does not exist
            // yet), then create a new name for a base branch
            let base_branch = if let Some(base_branch) = base_branch {
                base_branch
            } else {
                forge.find_unused_branch_name(
                    &config.branch_prefix,
                    &format!(
                        "{}.{}",
                        config.default_branch_name(),
                        &slugify(title),
                    ),
                )?
            };

            (Some(new_base_branch_commit), Some(base_branch))
        };

    let mut github_commit_message = opts.message.clone();
    if change_request.is_some() && github_commit_message.is_none() {
        if config.non_interactive {
            github_commit_message = Some("update".to_string());
        } else {
            let input = {
                let message_on_prompt = message_on_prompt.clone();

                tokio::task::spawn_blocking(move || {
                    dialoguer::Input::<String>::new()
                        .with_prompt("Message (leave empty to abort)")
                        .with_initial_text(message_on_prompt)
                        .allow_empty(true)
                        .interact_text()
                })
                .await??
            };

            if input.is_empty() {
                Err(SprError::UserAbort)?;
            }

            message_on_prompt.clone_from(&input);
            github_commit_message = Some(input);
        }
    }

    // Construct the new commit for the Pull Request branch. First parent is the
    // current head commit of the Pull Request (we set this to the default branch base
    // commit earlier if the Pull Request does not yet exist)
    let mut pr_commit_parents = vec![pr_head_oid];

    // If we prepared a commit earlier that needs merging into the Pull Request
    // branch, then that commit is a parent of the new Pull Request commit.
    if let Some(oid) = pr_base_parent {
        // ...unless if that's the same commit as the one we added to
        // pr_commit_parents first.
        if pr_commit_parents.first() != Some(&oid) {
            pr_commit_parents.push(oid);
        }
    }

    // Create the new commit
    let pr_commit = git.create_derived_commit(
        local_commit.oid,
        &format!(
            "{}\n\nCreated using spr {}",
            github_commit_message
                .as_ref()
                .map_or("[𝘀𝗽𝗿] initial version", |s| &s[..]),
            env!("CARGO_PKG_VERSION"),
        ),
        new_head_tree,
        &pr_commit_parents[..],
    )?;

    let head_ref = format!("refs/heads/{pull_request_branch}");
    let base_branch_ref =
        base_branch.as_ref().map(|b| format!("refs/heads/{b}"));

    if let Some(cr) = change_request {
        // We are updating an existing Pull Request

        if needs_merging_default_branch {
            output(
                "⚾",
                &format!(
                    "Commit was rebased - updating {} #{}",
                    forge.change_request_term_full(),
                    cr.number
                ),
            )?;
        } else {
            output(
                "🔁",
                &format!(
                    "Commit was changed - updating {} #{}",
                    forge.change_request_term_full(),
                    cr.number
                ),
            )?;
        }

        // Things we want to update in the Pull Request on GitHub
        let mut updates = ChangeRequestUpdate::default();

        if opts.update_message {
            updates.update_message(&cr, message);
        }

        if let Some(ref base_branch) = base_branch {
            // We are using a base branch.
            let mut push_specs = vec![PushSpec {
                oid: Some(pr_commit),
                remote_ref: &head_ref,
            }];

            if let Some(base_branch_commit) = pr_base_parent {
                // ...and we prepared a new commit for it, so we need to push an
                // update of the base branch.
                push_specs.push(PushSpec {
                    oid: Some(base_branch_commit),
                    remote_ref: base_branch_ref.as_deref().unwrap(),
                });
            }

            // Push the new commit onto the Pull Request branch (and also the
            // new base commit if present).
            forge
                .push_to_remote(push_specs.as_slice())
                .context("git push failed".to_string())?;

            // If the Pull Request's base is not set to the base branch yet,
            // change that now.
            if cr.base_ref_name != *base_branch {
                updates.base = Some(base_branch.clone());
            }
        } else {
            // The Pull Request is against the default branch. In that case we
            // only need to push the update to the Pull Request branch.
            let push_specs = vec![PushSpec {
                oid: Some(pr_commit),
                remote_ref: &head_ref,
            }];
            forge
                .push_to_remote(push_specs.as_slice())
                .context("git push failed".to_string())?;
        }

        if !updates.is_empty() {
            forge
                .update_change_request(cr.number, &updates, stack_info)
                .await?;
        }
    } else {
        // We are creating a new Pull Request.
        let mut push_specs = vec![PushSpec {
            oid: Some(pr_commit),
            remote_ref: &head_ref,
        }];

        // If there's a base branch, add it to the push
        if let (Some(_), Some(base_branch_commit)) =
            (&base_branch, pr_base_parent)
        {
            push_specs.push(PushSpec {
                oid: Some(base_branch_commit),
                remote_ref: base_branch_ref.as_deref().unwrap(),
            });
        }
        // Push the pull request branch and the base branch if present
        forge
            .push_to_remote(push_specs.as_slice())
            .context("git push failed".to_string())?;

        // Then call GitHub to create the Pull Request.
        let base_ref = base_branch
            .as_deref()
            .unwrap_or(config.default_branch_name());
        let pull_request_number = forge
            .create_change_request(
                message,
                base_ref,
                &pull_request_branch,
                opts.draft || config.create_draft_prs,
                stack_info,
            )
            .await?;

        let pull_request_url = forge.change_request_url(pull_request_number);

        output(
            "✨",
            &format!(
                "Created new {} #{}: {}",
                forge.change_request_term_full(),
                pull_request_number, &pull_request_url,
            ),
        )?;
        output_essential(&format!(
            "{} {}",
            forge.short_cr_ref(pull_request_number),
            &pull_request_url,
        ))?;

        message.insert(MessageSection::PullRequest, pull_request_url);

        let result = forge
            .request_reviewers(pull_request_number, &requested_reviewers)
            .await;
        match result {
            Ok(()) => (),
            Err(report) => {
                output("⚠️", "Requesting reviewers failed")?;
                for message in report.chain() {
                    output("  ", &message.to_string())?;
                }
            }
        }

        // Apply labels
        if !opts.label.is_empty() {
            let result =
                forge.add_labels(pull_request_number, &opts.label).await;
            if let Err(report) = result {
                output("⚠️", "Adding labels failed")?;
                for message in report.chain() {
                    output("  ", &message.to_string())?;
                }
            }
        }
    }

    Ok(())
}
