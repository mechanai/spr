/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

// unimock generates match arms that trigger this lint on parameterless trait methods.
#![allow(clippy::ignored_unit_patterns)]

mod dry_run;
mod verbose;
pub use dry_run::DryRunForge;
pub use verbose::VerboseForge;

use std::collections::HashMap;

use async_trait::async_trait;
use color_eyre::Result;
use git2::Oid;
#[cfg(test)]
use unimock::unimock;

use crate::config::MergeMethod;
use crate::git::Git;
use crate::git_remote::PushSpec;
use crate::message::MessageSectionsMap;

/// Review status for a change request reviewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Requested,
    Approved,
    Rejected,
}

/// State of a change request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeRequestState {
    Open,
    Closed,
    Merged,
}

/// Forge-neutral representation of a change request (PR, MR, etc.).
#[derive(Debug, Clone)]
pub struct ChangeRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub base_ref_name: String,
    pub base_oid: git2::Oid,
    pub head_ref_name: String,
    pub head_oid: git2::Oid,
    pub is_draft: bool,
    pub state: ChangeRequestState,
    pub sections: MessageSectionsMap,
    pub reviewers: HashMap<String, ReviewStatus>,
    pub review_status: Option<ReviewStatus>,
    pub merge_commit: Option<git2::Oid>,
}

/// Fields to update on a change request.
#[derive(Debug, Default)]
pub struct ChangeRequestUpdate {
    pub title: Option<String>,
    pub body: Option<String>,
    pub base: Option<String>,
    pub state: Option<ChangeRequestState>,
}

impl ChangeRequestUpdate {
    /// Returns true if no fields are set for update.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.base.is_none()
            && self.state.is_none()
    }

    /// Compare a change request's current title/body with a local message
    /// and conditionally set update fields.
    pub fn update_message(
        &mut self,
        cr: &ChangeRequest,
        message: &MessageSectionsMap,
    ) {
        let new_title =
            message.get(&crate::message::MessageSection::Title).cloned();
        if new_title.as_deref() != Some(&cr.title) {
            self.title = new_title;
        }
        let new_body = crate::message::build_forge_body(message);
        if cr.body.as_deref() != Some(&new_body) {
            self.body = Some(new_body);
        }
    }
}

/// Reviewers to request on a change request.
#[derive(Debug, Default)]
pub struct ReviewerRequest {
    pub users: Vec<String>,
    pub teams: Vec<String>,
}

/// Mergeability status of a change request.
#[derive(Debug, Clone)]
pub struct Mergeability {
    pub mergeable: Option<bool>,
    pub base_ref_name: String,
    pub head_oid: git2::Oid,
    pub merge_commit: Option<git2::Oid>,
}

/// Forge-agnostic API trait for interacting with a code hosting platform.
///
/// Implementations exist for GitHub (and in the future, Forgejo, etc.).
/// Commands use `&dyn ForgeApi` so they are decoupled from any specific forge.
#[cfg_attr(test, unimock(api=ForgeApiMock))]
#[async_trait(?Send)]
pub trait ForgeApi {
    // Change request lifecycle
    async fn create_change_request(
        &self,
        message: &MessageSectionsMap,
        base: &str,
        head: &str,
        draft: bool,
        stack_info: Option<&str>,
    ) -> Result<u64>;

    async fn update_change_request(
        &self,
        number: u64,
        update: &ChangeRequestUpdate,
        stack_info: Option<&str>,
    ) -> Result<()>;

    async fn get_change_request(
        &self,
        number: u64,
    ) -> Result<Option<ChangeRequest>>;

    async fn close_change_request(&self, number: u64) -> Result<()>;

    async fn merge_change_request(
        &self,
        number: u64,
        method: MergeMethod,
        title: &str,
        message: &str,
        expected_head_oid: Oid,
    ) -> Result<()>;

    async fn get_mergeability(&self, number: u64) -> Result<Mergeability>;

    // Reviewers and labels
    async fn request_reviewers(
        &self,
        number: u64,
        reviewers: &ReviewerRequest,
    ) -> Result<()>;

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()>;

    // User/team lookup
    async fn get_user(&self, username: &str) -> Result<Option<UserInfo>>;
    async fn get_team(
        &self,
        org: &str,
        team_slug: &str,
    ) -> Result<Option<TeamInfo>>;

    // Listing and bootstrapping
    //
    // `list_open_change_requests` operates on the configured repo (implicit).
    // `get_authenticated_user` and `get_repo_default_branch` take explicit
    // params because they are used during `init`, before config exists.
    async fn list_open_change_requests(
        &self,
    ) -> Result<Vec<OpenChangeRequestSummary>>;
    async fn get_authenticated_user(&self) -> Result<UserInfo>;
    async fn get_repo_default_branch(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<String>;

    // Git remote operations
    fn push_to_remote(&self, refs: &[PushSpec<'_>]) -> Result<()>;
    fn fetch_from_remote(
        &self,
        branch_names: &[&str],
        commit_oids: &[Oid],
    ) -> Result<Vec<Option<Oid>>>;
    fn fetch_branch(&self, branch_name: &str) -> Result<Oid>;
    fn find_unused_branch_name(
        &self,
        branch_prefix: &str,
        slug: &str,
    ) -> Result<String>;
    fn get_branches(&self) -> Result<HashMap<String, Oid>>;

    // Display — forge-native terminology for user-facing output
    fn change_request_term(&self) -> &str;
    fn change_request_term_full(&self) -> &str;

    /// Whether this forge is in dry-run mode (no writes performed).
    fn is_dry_run(&self) -> bool {
        false
    }
}

/// Forge-neutral user info.
#[derive(Debug, Clone)]
pub struct UserInfo {
    pub login: String,
    pub name: Option<String>,
    pub is_collaborator: bool,
}

/// Forge-neutral team info.
#[derive(Debug, Clone)]
pub struct TeamInfo {
    pub slug: String,
    pub name: String,
}

/// Summary of an open change request for listing.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenChangeRequestSummary {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub review_status: Option<ReviewStatus>,
}

/// Prepare commits for stacking — local git only, no network I/O.
///
/// Call `forge.fetch_branch()` first to get `remote_tip`, then pass it here.
pub fn get_prepared_commits(
    git: &Git,
    config: &crate::config::Config,
    remote_tip: Oid,
) -> Result<Vec<crate::git::PreparedCommit>> {
    git.get_prepared_commits(config, remote_tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_request_state_default() {
        let cr = ChangeRequest {
            number: 1,
            title: "test".to_owned(),
            body: None,
            base_ref_name: "main".to_owned(),
            base_oid: git2::Oid::zero(),
            head_ref_name: "spr/main/test".to_owned(),
            head_oid: git2::Oid::zero(),
            is_draft: false,
            state: ChangeRequestState::Open,
            sections: std::collections::BTreeMap::default(),
            reviewers: HashMap::default(),
            review_status: None,
            merge_commit: None,
        };
        assert_eq!(cr.number, 1);
        assert_eq!(cr.state, ChangeRequestState::Open);
    }

    #[test]
    fn test_reviewer_request_empty() {
        let rr = ReviewerRequest::default();
        assert!(rr.users.is_empty());
        assert!(rr.teams.is_empty());
    }

    #[test]
    fn test_change_request_update_is_empty() {
        let update = ChangeRequestUpdate::default();
        assert!(update.is_empty());

        let update = ChangeRequestUpdate {
            title: Some("new title".to_owned()),
            ..Default::default()
        };
        assert!(!update.is_empty());
    }
}
