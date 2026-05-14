/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::Result;

use crate::{
    message::{MessageSection, build_commit_message},
    output::output,
};

#[derive(Debug, clap::Parser)]
pub struct PatchOptions {
    /// Pull Request number
    pull_request: u64,

    /// Name of the branch to be created. Defaults to `PR-<number>`
    #[clap(long)]
    branch_name: Option<String>,

    /// If given, create new branch but do not check out
    #[clap(long)]
    no_checkout: bool,
}

pub async fn patch(
    opts: PatchOptions,
    git: &crate::git::Git,
    forge: &dyn crate::forge::ForgeApi,
    config: &crate::config::Config,
) -> Result<()> {
    let pr = forge
        .get_change_request(opts.pull_request)
        .await?
        .ok_or_else(|| color_eyre::eyre::eyre!("{} not found", forge.change_request_term()))?;
    output(
        "#️⃣ ",
        &format!(
            "{} #{}: {}",
            forge.change_request_term_full(),
            pr.number,
            pr.sections
                .get(&MessageSection::Title)
                .map_or("(no title)", |s| &s[..])
        ),
    )?;

    let branch_name = if let Some(name) = opts.branch_name {
        name
    } else {
        git.get_pr_patch_branch_name(pr.number)?
    };

    let patch_branch_oid = if let Some(oid) = pr.merge_commit {
        output("❗", &format!("{} has been merged", forge.change_request_term_full()))?;

        oid
    } else {
        // Current oid of the default branch
        let current_default_branch_oid =
            forge.fetch_branch(config.default_branch_name())?;

        // The parent commit to base the new PR branch on shall be the default
        // branch commit this PR is based on
        let mut pr_default_branch_oid =
            git.repo().merge_base(pr.head_oid, current_default_branch_oid)?;

        // The PR may be against the default branch or some base branch.
        // `pr.base_oid` indicates what the PR base is, but might point to the
        // latest commit of the target (i.e. base) branch, and especially if
        // the target branch is the default branch, might be different from the
        // commit the PR is actually based on. But the merge base of the given
        // `pr.base_oid` and the PR head is the right commit.
        let pr_base_oid = git.repo().merge_base(pr.head_oid, pr.base_oid)?;

        if pr_base_oid != pr_default_branch_oid {
            // So the commit the PR is based on is not the same as the default
            // branch commit it's based on. This means there must be a base
            // branch that contains additional commits. We want to squash those
            // changes into one commit that we then title "Base of Pull
            // Reqeust #x".
            // Oh, one more thing. The base commit might not be on the default
            // branch, but if it, for whatever reason, contains the same tree
            // as the default branch base, the base commit we construct here
            // would turn out to be empty. No point in creating an empty
            // commit, so let's first check whether base tree and default
            // branch tree are different.
            let pr_base_tree = git.get_tree_oid_for_commit(pr.base_oid)?;
            let default_branch_tree = git.get_tree_oid_for_commit(pr_default_branch_oid)?;

            if pr_base_tree != default_branch_tree {
                // The base of this PR is not on the default branch. We need to
                // create two commits on the new branch we are making. First, a
                // commit that represents the base of the PR. And then second,
                // the commit that represents the contents of the PR.

                pr_default_branch_oid = git.create_derived_commit(
                    pr_base_oid,
                    &format!("[𝘀𝗽𝗿] Base of {} #{}", forge.change_request_term_full(), pr.number),
                    pr_base_tree,
                    &[pr_default_branch_oid],
                )?;
            }
        }

        // Create the main commit for the patch branch. This is based on a
        // default branch commit, or, if the PR can't be based on the default
        // branch directly, on the commit we created above to prepare the base
        // of this commit.
        git.create_derived_commit(
            pr.head_oid,
            &build_commit_message(&pr.sections),
            git.get_tree_oid_for_commit(pr.head_oid)?,
            &[pr_default_branch_oid],
        )?
    };

    let repo = git.repo();
    let patch_branch_commit = repo.find_commit(patch_branch_oid)?;

    // Create the new branch, now that we know the commit it shall point to
    repo.branch(&branch_name, &patch_branch_commit, true)?;

    output("🌱", &format!("Created new branch: {}", &branch_name))?;

    if !opts.no_checkout {
        // Check out the new branch
        repo.checkout_tree(patch_branch_commit.as_object(), None)?;
        repo.set_head(&format!("refs/heads/{branch_name}"))?;
        output("✅", "Checked out")?;
    }

    Ok(())
}
