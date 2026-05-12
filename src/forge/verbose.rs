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
use crate::output::output;

/// A [`ForgeApi`] decorator that logs each action before delegating.
pub struct VerboseForge<T> {
    inner: T,
}

impl<T: ForgeApi> VerboseForge<T> {
    #[must_use]
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

#[async_trait(?Send)]
impl<T: ForgeApi> ForgeApi for VerboseForge<T> {
    async fn create_change_request(
        &self,
        message: &MessageSectionsMap,
        base: &str,
        head: &str,
        draft: bool,
        stack_info: Option<&str>,
    ) -> Result<u64> {
        let term = self.inner.change_request_term();
        let title = message
            .get(&crate::message::MessageSection::Title)
            .map_or("(untitled)", |s| s.as_str());
        let _ = output(
            "\u{1f528}",
            &format!(
                "create {term} \"{title}\" targeting {base} (draft: {draft})"
            ),
        );
        self.inner
            .create_change_request(message, base, head, draft, stack_info)
            .await
    }

    async fn update_change_request(
        &self,
        number: u64,
        update: &ChangeRequestUpdate,
        stack_info: Option<&str>,
    ) -> Result<()> {
        let term = self.inner.change_request_term();
        let _ = output("\u{1f528}", &format!("update {term} #{number}"));
        self.inner
            .update_change_request(number, update, stack_info)
            .await
    }

    async fn get_change_request(
        &self,
        number: u64,
    ) -> Result<Option<ChangeRequest>> {
        self.inner.get_change_request(number).await
    }

    async fn close_change_request(&self, number: u64) -> Result<()> {
        let term = self.inner.change_request_term();
        let _ = output("\u{1f528}", &format!("close {term} #{number}"));
        self.inner.close_change_request(number).await
    }

    async fn merge_change_request(
        &self,
        number: u64,
        method: MergeMethod,
        title: &str,
        message: &str,
        expected_head_oid: Oid,
    ) -> Result<()> {
        let term = self.inner.change_request_term();
        let _ = output(
            "\u{1f528}",
            &format!("merge {term} #{number} ({method:?})"),
        );
        self.inner
            .merge_change_request(
                number,
                method,
                title,
                message,
                expected_head_oid,
            )
            .await
    }

    async fn get_mergeability(&self, number: u64) -> Result<Mergeability> {
        self.inner.get_mergeability(number).await
    }

    async fn request_reviewers(
        &self,
        number: u64,
        reviewers: &ReviewerRequest,
    ) -> Result<()> {
        let all: Vec<&str> = reviewers
            .users
            .iter()
            .chain(reviewers.teams.iter())
            .map(String::as_str)
            .collect();
        let _ = output(
            "\u{1f528}",
            &format!("request reviewers: {}", all.join(", ")),
        );
        self.inner.request_reviewers(number, reviewers).await
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        let _ =
            output("\u{1f528}", &format!("add labels: {}", labels.join(", ")));
        self.inner.add_labels(number, labels).await
    }

    async fn get_user(&self, username: &str) -> Result<Option<UserInfo>> {
        self.inner.get_user(username).await
    }

    async fn get_team(
        &self,
        org: &str,
        team_slug: &str,
    ) -> Result<Option<TeamInfo>> {
        self.inner.get_team(org, team_slug).await
    }

    fn push_to_remote(&self, refs: &[PushSpec<'_>]) -> Result<()> {
        for r in refs {
            let _ = output("\u{1f528}", &format!("push {r}"));
        }
        self.inner.push_to_remote(refs)
    }

    fn fetch_from_remote(
        &self,
        branch_names: &[&str],
        commit_oids: &[Oid],
    ) -> Result<Vec<Option<Oid>>> {
        self.inner.fetch_from_remote(branch_names, commit_oids)
    }

    fn fetch_branch(&self, branch_name: &str) -> Result<Oid> {
        self.inner.fetch_branch(branch_name)
    }

    fn find_unused_branch_name(
        &self,
        branch_prefix: &str,
        slug: &str,
    ) -> Result<String> {
        self.inner.find_unused_branch_name(branch_prefix, slug)
    }

    fn get_branches(&self) -> Result<HashMap<String, Oid>> {
        self.inner.get_branches()
    }

    fn change_request_term(&self) -> &str {
        self.inner.change_request_term()
    }

    fn is_dry_run(&self) -> bool {
        self.inner.is_dry_run()
    }
}
