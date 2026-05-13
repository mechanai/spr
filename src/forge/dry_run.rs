/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::collections::HashMap;

use async_trait::async_trait;
use color_eyre::Result;
use git2::Oid;

use crate::config::MergeMethod;
use crate::forge::{
    ChangeRequest, ChangeRequestUpdate, ForgeApi, Mergeability,
    ReviewerRequest, TeamInfo, UserInfo,
};
use crate::git_remote::PushSpec;
use crate::message::MessageSectionsMap;
use crate::output::output_essential;

/// A [`ForgeApi`] wrapper that delegates reads to an inner forge but logs
/// writes without performing them. If no inner forge is provided
/// (tests only), reads return empty/None results.
pub struct DryRunForge {
    inner: Option<Box<dyn ForgeApi>>,
    verbose: bool,
    cr_term: String,
}

impl DryRunForge {
    #[must_use]
    pub fn new(inner: Box<dyn ForgeApi>, verbose: bool) -> Self {
        let cr_term = inner.change_request_term().to_owned();
        Self {
            inner: Some(inner),
            verbose,
            cr_term,
        }
    }

    /// For tests only — no inner forge, reads return None/empty.
    #[cfg(test)]
    #[must_use]
    pub fn without_inner(verbose: bool, cr_term: &str) -> Self {
        Self {
            inner: None,
            verbose,
            cr_term: cr_term.to_owned(),
        }
    }

    #[allow(clippy::unused_self)]
    fn log(&self, action: &str) {
        let _ = output_essential(&format!("[dry-run] {action}"));
    }

    fn log_verbose(&self, action: &str) {
        if self.verbose {
            let _ = output_essential(&format!("[dry-run]   {action}"));
        }
    }
}

#[async_trait(?Send)]
impl ForgeApi for DryRunForge {
    async fn create_change_request(
        &self,
        message: &MessageSectionsMap,
        base: &str,
        head: &str,
        draft: bool,
        _stack_info: Option<&str>,
    ) -> Result<u64> {
        let title = message
            .get(&crate::message::MessageSection::Title)
            .map_or("(untitled)", |s| s.as_str());
        self.log(&format!("would create {} \"{}\"", self.cr_term, title));
        self.log_verbose(&format!("targeting {base}"));
        self.log_verbose(&format!("head: {head}"));
        self.log_verbose(&format!("draft: {draft}"));
        Ok(0)
    }

    async fn update_change_request(
        &self,
        number: u64,
        update: &ChangeRequestUpdate,
        _stack_info: Option<&str>,
    ) -> Result<()> {
        if let Some(base) = &update.base {
            self.log(&format!(
                "would update {} #{} base to {}",
                self.cr_term, number, base
            ));
        }
        if update.title.is_some() || update.body.is_some() {
            self.log(&format!(
                "would update {} #{} message",
                self.cr_term, number
            ));
        }
        if update.state.is_some() {
            self.log(&format!(
                "would update {} #{} state",
                self.cr_term, number
            ));
        }
        Ok(())
    }

    // --- Reads delegate to inner forge ---

    async fn get_change_request(
        &self,
        number: u64,
    ) -> Result<Option<ChangeRequest>> {
        match &self.inner {
            Some(inner) => inner.get_change_request(number).await,
            None => Ok(None),
        }
    }

    async fn close_change_request(&self, number: u64) -> Result<()> {
        self.log(&format!("would close {} #{}", self.cr_term, number));
        Ok(())
    }

    async fn merge_change_request(
        &self,
        number: u64,
        method: MergeMethod,
        title: &str,
        _message: &str,
        _expected_head_oid: Oid,
    ) -> Result<()> {
        self.log(&format!(
            "would merge {} #{} ({:?})",
            self.cr_term, number, method
        ));
        self.log_verbose(&format!("title: {title}"));
        Ok(())
    }

    async fn get_mergeability(&self, number: u64) -> Result<Mergeability> {
        match &self.inner {
            Some(inner) => inner.get_mergeability(number).await,
            None => Ok(Mergeability {
                mergeable: Some(true),
                base_ref_name: String::new(),
                head_oid: Oid::zero(),
                merge_commit: None,
            }),
        }
    }

    async fn request_reviewers(
        &self,
        _number: u64,
        reviewers: &ReviewerRequest,
    ) -> Result<()> {
        if !reviewers.users.is_empty() || !reviewers.teams.is_empty() {
            let all: Vec<&str> = reviewers
                .users
                .iter()
                .chain(reviewers.teams.iter())
                .map(String::as_str)
                .collect();
            self.log(&format!("would request reviewers: {}", all.join(", ")));
        }
        Ok(())
    }

    async fn add_labels(&self, _number: u64, labels: &[String]) -> Result<()> {
        if !labels.is_empty() {
            self.log(&format!("would add labels: {}", labels.join(", ")));
        }
        Ok(())
    }

    async fn get_user(&self, username: &str) -> Result<Option<UserInfo>> {
        match &self.inner {
            Some(inner) => inner.get_user(username).await,
            None => Ok(None),
        }
    }

    async fn get_team(
        &self,
        org: &str,
        team_slug: &str,
    ) -> Result<Option<TeamInfo>> {
        match &self.inner {
            Some(inner) => inner.get_team(org, team_slug).await,
            None => Ok(None),
        }
    }

    fn push_to_remote(&self, refs: &[PushSpec<'_>]) -> Result<()> {
        for r in refs {
            self.log(&format!("would push {r}"));
        }
        Ok(())
    }

    fn fetch_from_remote(
        &self,
        branch_names: &[&str],
        commit_oids: &[Oid],
    ) -> Result<Vec<Option<Oid>>> {
        match &self.inner {
            Some(inner) => inner.fetch_from_remote(branch_names, commit_oids),
            None => Ok(vec![None; branch_names.len() + commit_oids.len()]),
        }
    }

    fn fetch_branch(&self, branch_name: &str) -> Result<Oid> {
        if let Some(inner) = &self.inner {
            inner.fetch_branch(branch_name)
        } else {
            self.log_verbose(&format!("(no inner forge for {branch_name})"));
            Ok(Oid::zero())
        }
    }

    fn find_unused_branch_name(
        &self,
        branch_prefix: &str,
        slug: &str,
    ) -> Result<String> {
        match &self.inner {
            Some(inner) => inner.find_unused_branch_name(branch_prefix, slug),
            None => Ok(format!("{branch_prefix}{slug}")),
        }
    }

    fn get_branches(&self) -> Result<HashMap<String, Oid>> {
        match &self.inner {
            Some(inner) => inner.get_branches(),
            None => Ok(HashMap::new()),
        }
    }

    fn change_request_term(&self) -> &str {
        &self.cr_term
    }

    fn is_dry_run(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn test_dry_run_create_change_request() {
        let forge = DryRunForge::without_inner(false, "PR");
        let message = crate::message::MessageSectionsMap::new();
        let result = forge
            .create_change_request(
                &message,
                "main",
                "spr/main/test",
                false,
                None,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_dry_run_change_request_term() {
        let forge = DryRunForge::without_inner(false, "PR");
        assert_eq!(forge.change_request_term(), "PR");
    }
}
