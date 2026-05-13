/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Error, Result, WrapErr as _, eyre};
use graphql_client::{GraphQLQuery, Response};
use serde::Deserialize;

use crate::{
    git::PreparedCommit,
    git_remote::GitRemote,
    message::{
        MessageSection, MessageSectionsMap, build_github_body, parse_message,
    },
};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

use crate::forge::{
    ChangeRequest, ChangeRequestState, ChangeRequestUpdate, ForgeApi,
    Mergeability, ReviewStatus as ForgeReviewStatus, ReviewerRequest, TeamInfo,
    UserInfo,
};

#[derive(Clone)]
pub struct GitHub {
    config: crate::config::Config,
    git: crate::git::Git,
    git_remote: crate::git_remote::GitRemote,
}

#[derive(Debug, Clone)]
pub struct PullRequest {
    pub number: u64,
    pub state: PullRequestState,
    pub title: String,
    pub body: Option<String>,
    pub sections: MessageSectionsMap,
    pub base: GitHubBranch,
    pub head: GitHubBranch,
    pub base_oid: git2::Oid,
    pub head_oid: git2::Oid,
    pub merge_commit: Option<git2::Oid>,
    pub reviewers: HashMap<String, ReviewStatus>,
    pub review_status: Option<ReviewStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Requested,
    Approved,
    Rejected,
}

#[derive(serde::Serialize, Default, Debug)]
pub struct PullRequestUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<PullRequestState>,
}

impl PullRequestUpdate {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.base.is_none()
            && self.state.is_none()
    }

    pub fn update_message(
        &mut self,
        pull_request: &PullRequest,
        message: &MessageSectionsMap,
    ) {
        let title = message.get(&MessageSection::Title);
        if title.is_some() && title != Some(&pull_request.title) {
            self.title = title.cloned();
        }

        let body = build_github_body(message);
        if pull_request.body.as_ref() != Some(&body) {
            self.body = Some(body);
        }
    }
}

#[derive(serde::Serialize, Default, Debug)]
pub struct PullRequestRequestReviewers {
    pub reviewers: Vec<String>,
    pub team_reviewers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestState {
    Open,
    Closed,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct UserWithName {
    pub login: String,
    pub name: Option<String>,
    #[serde(default)]
    pub is_collaborator: bool,
}

#[derive(Debug, Clone)]
pub struct PullRequestMergeability {
    pub base: GitHubBranch,
    pub head_oid: git2::Oid,
    pub mergeable: Option<bool>,
    pub merge_commit: Option<git2::Oid>,
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/pullrequest_query.graphql",
    response_derives = "Debug"
)]
pub struct PullRequestQuery;
type GitObjectID = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/pullrequest_mergeability_query.graphql",
    response_derives = "Debug"
)]
pub struct PullRequestMergeabilityQuery;

impl GitHub {
    #[must_use]
    pub fn new(
        config: crate::config::Config,
        git: crate::git::Git,
        auth_token: String,
    ) -> Self {
        let git_remote = GitRemote::new(
            git.repo().clone(),
            format!(
                "https://github.com/{}/{}.git",
                &config.owner, &config.repo,
            ),
            auth_token,
        );
        Self {
            config,
            git,
            git_remote,
        }
    }

    #[must_use]
    pub fn remote(&self) -> &GitRemote {
        &self.git_remote
    }

    pub fn get_prepared_commits(&self) -> Result<Vec<PreparedCommit>> {
        let master_oid = self
            .git_remote
            .fetch_branch(self.config.master_ref.branch_name())?;
        self.git.get_prepared_commits(&self.config, master_oid)
    }

    pub async fn get_github_user(&self, login: &str) -> Result<UserWithName> {
        octocrab::instance()
            .get::<UserWithName, _, _>(format!("/users/{login}"), None::<&()>)
            .await
            .map_err(Error::from)
    }

    pub async fn get_github_team(
        &self,
        owner: &str,
        team: &str,
    ) -> Result<octocrab::models::teams::Team> {
        octocrab::instance()
            .teams(owner)
            .get(team)
            .await
            .map_err(Error::from)
    }

    pub async fn get_pull_request(&self, number: u64) -> Result<PullRequest> {
        let variables = pull_request_query::Variables {
            name: self.config.repo.clone(),
            owner: self.config.owner.clone(),
            #[allow(clippy::cast_possible_wrap)]
            number: number as i64,
        };
        let request_body = PullRequestQuery::build_query(variables);
        let response_body: Response<pull_request_query::ResponseData> =
            octocrab::instance()
                .post("/graphql", Some(&request_body))
                .await?;

        if let Some(errors) = response_body.errors {
            let error = Err(eyre!("fetching PR #{number} failed"));
            return errors
                .into_iter()
                .fold(error, |err, e| err.context(e.to_string()));
        }

        let pr = response_body
            .data
            .ok_or_else(|| eyre!("failed to fetch PR"))?
            .repository
            .ok_or_else(|| eyre!("failed to find repository"))?
            .pull_request
            .ok_or_else(|| eyre!("failed to find PR"))?;

        let base = self.config.new_github_branch_from_ref(&pr.base_ref_name)?;
        let head = self.config.new_github_branch_from_ref(&pr.head_ref_name)?;

        let branch_names: Vec<_> =
            [&base, &head].iter().map(|&b| b.branch_name()).collect();

        let [base_oid, head_oid] =
            self.git_remote.fetch_from_remote(&branch_names, &[])?[0..2]
        else {
            unreachable!();
        };

        let base_oid = base_oid.ok_or_else(|| {
            eyre!("{} not found on GitHub", &base.ref_on_github)
        })?;
        let head_oid = head_oid.ok_or_else(|| {
            eyre!("{} not found on GitHub", &head.ref_on_github)
        })?;

        let mut sections = parse_message(&pr.body, MessageSection::Summary);

        let title = pr.title.trim().to_string();
        sections.insert(
            MessageSection::Title,
            if title.is_empty() {
                String::from("(untitled)")
            } else {
                title
            },
        );

        sections.insert(
            MessageSection::PullRequest,
            self.config.pull_request_url(number),
        );

        let reviewers: HashMap<String, ReviewStatus> = pr
            .latest_opinionated_reviews
            .iter()
            .flat_map(|all_reviews| &all_reviews.nodes)
            .flatten()
            .flatten()
            .filter_map(|review| {
                let user_name = review.author.as_ref()?.login.clone();
                let status = match review.state {
                    pull_request_query::PullRequestReviewState::APPROVED => ReviewStatus::Approved,
                    pull_request_query::PullRequestReviewState::CHANGES_REQUESTED => ReviewStatus::Rejected,
                    _ => ReviewStatus::Requested,
                };
                Some((user_name, status))
            })
            .collect();

        let review_status = match pr.review_decision {
            Some(pull_request_query::PullRequestReviewDecision::APPROVED) => Some(ReviewStatus::Approved),
            Some(pull_request_query::PullRequestReviewDecision::CHANGES_REQUESTED) => Some(ReviewStatus::Rejected),
            Some(pull_request_query::PullRequestReviewDecision::REVIEW_REQUIRED) => Some(ReviewStatus::Requested),
            _ => None,
        };

        let requested_reviewers: Vec<String> = pr.review_requests
            .iter()
            .flat_map(|x| &x.nodes)
            .flatten()
            .flatten()
            .flat_map(|x| &x.requested_reviewer)
            .filter_map(|reviewer| {
              type UserType = pull_request_query::PullRequestQueryRepositoryPullRequestReviewRequestsNodesRequestedReviewer;
              match reviewer {
                UserType::User(user) => Some(user.login.clone()),
                UserType::Team(team) => Some(format!("#{}", team.slug)),
                _ => None,
              }
            })
            .chain(reviewers.keys().cloned())
            .collect::<HashSet<String>>() // de-duplicate
            .into_iter()
            .collect();

        sections
            .insert(MessageSection::Reviewers, requested_reviewers.join(", "));

        if review_status == Some(ReviewStatus::Approved) {
            sections.insert(
                MessageSection::ReviewedBy,
                reviewers
                    .iter()
                    .filter_map(|(k, v)| {
                        (v == &ReviewStatus::Approved).then_some(k.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }

        Ok::<_, Error>(PullRequest {
            #[allow(clippy::cast_sign_loss)]
            number: pr.number as u64,
            state: match pr.state {
                pull_request_query::PullRequestState::OPEN => {
                    PullRequestState::Open
                }
                _ => PullRequestState::Closed,
            },
            title: pr.title,
            body: Some(pr.body),
            sections,
            base,
            head,
            base_oid,
            head_oid,
            reviewers,
            review_status,
            merge_commit: pr
                .merge_commit
                .and_then(|sha| git2::Oid::from_str(&sha.oid).ok()),
        })
    }

    pub async fn create_pull_request(
        &self,
        message: &MessageSectionsMap,
        base_ref_name: String,
        head_ref_name: String,
        draft: bool,
        stack_info: Option<&str>,
    ) -> Result<u64> {
        let mut body = build_github_body(message);
        if let Some(info) = stack_info {
            body.push_str("\n\n");
            body.push_str(&crate::stack::wrap_with_markers(info));
        }
        let number = octocrab::instance()
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .create(
                message
                    .get(&MessageSection::Title)
                    .unwrap_or(&String::new()),
                head_ref_name,
                base_ref_name,
            )
            .body(body)
            .draft(Some(draft))
            .send()
            .await?
            .number;

        Ok(number)
    }

    pub async fn update_pull_request(
        &self,
        number: u64,
        updates: PullRequestUpdate,
    ) -> Result<()> {
        octocrab::instance()
            .patch::<octocrab::models::pulls::PullRequest, _, _>(
                format!(
                    "/repos/{}/{}/pulls/{}",
                    self.config.owner, self.config.repo, number
                ),
                Some(&updates),
            )
            .await?;

        Ok(())
    }

    pub async fn request_reviewers(
        &self,
        number: u64,
        reviewers: PullRequestRequestReviewers,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Ignore {}
        let _: Ignore = octocrab::instance()
            .post(
                format!(
                    "/repos/{}/{}/pulls/{}/requested_reviewers",
                    self.config.owner, self.config.repo, number
                ),
                Some(&reviewers),
            )
            .await?;

        Ok(())
    }

    pub async fn add_labels(
        &self,
        number: u64,
        labels: &[String],
    ) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        octocrab::instance()
            .issues(&self.config.owner, &self.config.repo)
            .add_labels(number, labels)
            .await?;
        Ok(())
    }

    pub async fn get_pull_request_mergeability(
        &self,
        number: u64,
    ) -> Result<PullRequestMergeability> {
        let variables = pull_request_mergeability_query::Variables {
            name: self.config.repo.clone(),
            owner: self.config.owner.clone(),
            #[allow(clippy::cast_possible_wrap)]
            number: number as i64,
        };
        let request_body = PullRequestMergeabilityQuery::build_query(variables);
        let response_body: Response<
            pull_request_mergeability_query::ResponseData,
        > = octocrab::instance()
            .post("/graphql", Some(&request_body))
            .await?;

        if let Some(errors) = response_body.errors {
            let error = Err(eyre!("querying PR #{number} mergeability failed"));
            return errors.into_iter().fold(error, |err, e| err.wrap_err(e));
        }

        let pr = response_body
            .data
            .ok_or_else(|| eyre!("failed to fetch PR"))?
            .repository
            .ok_or_else(|| eyre!("failed to find repository"))?
            .pull_request
            .ok_or_else(|| eyre!("failed to find PR"))?;

        Ok::<_, Error>(PullRequestMergeability {
            base: self.config.new_github_branch_from_ref(&pr.base_ref_name)?,
            head_oid: git2::Oid::from_str(&pr.head_ref_oid)?,
            mergeable: match pr.mergeable {
                pull_request_mergeability_query::MergeableState::CONFLICTING => Some(false),
                pull_request_mergeability_query::MergeableState::MERGEABLE => Some(true),
                pull_request_mergeability_query::MergeableState::UNKNOWN
                | pull_request_mergeability_query::MergeableState::Other(_) => None,
            },
            merge_commit: pr
            .merge_commit
            .and_then(|sha| git2::Oid::from_str(&sha.oid).ok()),
        })
    }

    pub async fn close_pull_request(&self, number: u64) -> Result<()> {
        let updates = PullRequestUpdate {
            state: Some(PullRequestState::Closed),
            ..Default::default()
        };
        self.update_pull_request(number, updates).await
    }

    pub async fn merge_pull_request(
        &self,
        number: u64,
        method: crate::config::MergeMethod,
        title: &str,
        message: &str,
        expected_head_oid: git2::Oid,
    ) -> Result<()> {
        let octocrab_method = match method {
            crate::config::MergeMethod::Squash => {
                octocrab::params::pulls::MergeMethod::Squash
            }
            crate::config::MergeMethod::Rebase => {
                octocrab::params::pulls::MergeMethod::Rebase
            }
            crate::config::MergeMethod::Merge => {
                octocrab::params::pulls::MergeMethod::Merge
            }
        };
        let merge = octocrab::instance()
            .pulls(&self.config.owner, &self.config.repo)
            .merge(number)
            .method(octocrab_method)
            .title(title)
            .message(message)
            .sha(format!("{expected_head_oid}"))
            .send()
            .await?;
        if merge.merged {
            Ok(())
        } else {
            Err(eyre!(
                "Pull Request merge failed: {}",
                merge.message.unwrap_or_default()
            ))
        }
    }

    fn pull_request_to_change_request(pr: PullRequest) -> ChangeRequest {
        ChangeRequest {
            number: pr.number,
            title: pr.title,
            body: pr.body,
            base_ref_name: pr.base.branch_name().to_owned(),
            base_oid: pr.base_oid,
            head_ref_name: pr.head.branch_name().to_owned(),
            head_oid: pr.head_oid,
            is_draft: false,
            state: match pr.state {
                PullRequestState::Open => ChangeRequestState::Open,
                PullRequestState::Closed => {
                    if pr.merge_commit.is_some() {
                        ChangeRequestState::Merged
                    } else {
                        ChangeRequestState::Closed
                    }
                }
            },
            sections: pr.sections,
            reviewers: pr
                .reviewers
                .into_iter()
                .map(|(k, v)| {
                    let forge_status = match v {
                        ReviewStatus::Requested => ForgeReviewStatus::Requested,
                        ReviewStatus::Approved => ForgeReviewStatus::Approved,
                        ReviewStatus::Rejected => ForgeReviewStatus::Rejected,
                    };
                    (k, forge_status)
                })
                .collect(),
            review_status: pr.review_status.map(|s| match s {
                ReviewStatus::Requested => ForgeReviewStatus::Requested,
                ReviewStatus::Approved => ForgeReviewStatus::Approved,
                ReviewStatus::Rejected => ForgeReviewStatus::Rejected,
            }),
            merge_commit: pr.merge_commit,
        }
    }
}

#[async_trait(?Send)]
impl ForgeApi for GitHub {
    async fn create_change_request(
        &self,
        message: &MessageSectionsMap,
        base: &str,
        head: &str,
        draft: bool,
        stack_info: Option<&str>,
    ) -> Result<u64> {
        self.create_pull_request(
            message,
            base.to_owned(),
            head.to_owned(),
            draft,
            stack_info,
        )
        .await
    }

    async fn update_change_request(
        &self,
        number: u64,
        update: &ChangeRequestUpdate,
        stack_info: Option<&str>,
    ) -> Result<()> {
        let mut pr_update = PullRequestUpdate {
            title: update.title.clone(),
            body: update.body.clone(),
            base: update.base.clone(),
            state: update.state.as_ref().map(|s| match s {
                ChangeRequestState::Open => PullRequestState::Open,
                ChangeRequestState::Closed | ChangeRequestState::Merged => {
                    PullRequestState::Closed
                }
            }),
        };

        if let Some(info) = stack_info {
            // If the update doesn't include a body, fetch the current PR body
            // so stack markers are applied to the real body, not an empty string.
            let current_body = match pr_update.body.take() {
                Some(body) => body,
                None => self
                    .get_pull_request(number)
                    .await
                    .ok()
                    .and_then(|pr| pr.body)
                    .unwrap_or_default(),
            };
            pr_update.body =
                Some(crate::stack::update_body_with_stack(&current_body, info));
        }

        self.update_pull_request(number, pr_update).await
    }

    async fn get_change_request(
        &self,
        number: u64,
    ) -> Result<Option<ChangeRequest>> {
        match self.get_pull_request(number).await {
            Ok(pr) => Ok(Some(Self::pull_request_to_change_request(pr))),
            Err(e) => {
                if e.downcast_ref::<octocrab::Error>()
                    .is_some_and(is_not_found)
                {
                    return Ok(None);
                }
                Err(e)
            }
        }
    }

    async fn close_change_request(&self, number: u64) -> Result<()> {
        self.close_pull_request(number).await
    }

    async fn merge_change_request(
        &self,
        number: u64,
        method: crate::config::MergeMethod,
        title: &str,
        message: &str,
        expected_head_oid: git2::Oid,
    ) -> Result<()> {
        self.merge_pull_request(
            number,
            method,
            title,
            message,
            expected_head_oid,
        )
        .await
    }

    async fn get_mergeability(&self, number: u64) -> Result<Mergeability> {
        let m = self.get_pull_request_mergeability(number).await?;
        Ok(Mergeability {
            mergeable: m.mergeable,
            base_ref_name: m.base.branch_name().to_owned(),
            head_oid: m.head_oid,
            merge_commit: m.merge_commit,
        })
    }

    async fn request_reviewers(
        &self,
        number: u64,
        reviewers: &ReviewerRequest,
    ) -> Result<()> {
        let pr_reviewers = PullRequestRequestReviewers {
            reviewers: reviewers.users.clone(),
            team_reviewers: reviewers.teams.clone(),
        };
        GitHub::request_reviewers(self, number, pr_reviewers).await
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        GitHub::add_labels(self, number, labels).await
    }

    async fn get_user(&self, username: &str) -> Result<Option<UserInfo>> {
        match self.get_github_user(username).await {
            Ok(u) => Ok(Some(UserInfo {
                login: u.login,
                name: u.name,
                is_collaborator: u.is_collaborator,
            })),
            Err(e) => {
                if e.downcast_ref::<octocrab::Error>()
                    .is_some_and(is_not_found)
                {
                    return Ok(None);
                }
                Err(e)
            }
        }
    }

    async fn get_team(
        &self,
        org: &str,
        team_slug: &str,
    ) -> Result<Option<TeamInfo>> {
        match self.get_github_team(org, team_slug).await {
            Ok(t) => Ok(Some(TeamInfo {
                name: t.name.clone(),
                slug: t.slug,
            })),
            Err(e) => {
                if e.downcast_ref::<octocrab::Error>()
                    .is_some_and(is_not_found)
                {
                    return Ok(None);
                }
                Err(e)
            }
        }
    }

    fn push_to_remote(
        &self,
        refs: &[crate::git_remote::PushSpec<'_>],
    ) -> Result<()> {
        self.git_remote.push_to_remote(refs)
    }

    fn fetch_from_remote(
        &self,
        branch_names: &[&str],
        commit_oids: &[git2::Oid],
    ) -> Result<Vec<Option<git2::Oid>>> {
        self.git_remote.fetch_from_remote(branch_names, commit_oids)
    }

    fn fetch_branch(&self, branch_name: &str) -> Result<git2::Oid> {
        self.git_remote.fetch_branch(branch_name)
    }

    fn find_unused_branch_name(
        &self,
        branch_prefix: &str,
        slug: &str,
    ) -> Result<String> {
        self.git_remote.find_unused_branch_name(branch_prefix, slug)
    }

    fn get_branches(
        &self,
    ) -> Result<std::collections::HashMap<String, git2::Oid>> {
        self.git_remote.get_branches()
    }

    fn change_request_term(&self) -> &'static str {
        "PR"
    }
}

#[derive(Debug, Clone)]
pub struct GitHubBranch {
    ref_on_github: String,
    is_master_branch: bool,
}

impl GitHubBranch {
    pub fn new_from_ref(ghref: &str, master_branch_name: &str) -> Result<Self> {
        let ref_on_github = if ghref.starts_with("refs/heads/") {
            ghref.to_string()
        } else if ghref.starts_with("refs/") {
            return Err(eyre!("Ref '{ghref}' does not refer to a branch"));
        } else {
            format!("refs/heads/{ghref}")
        };

        // The branch name is `ref_on_github` with the `refs/heads/` prefix
        // (length 11) removed
        let branch_name = &ref_on_github[11..];
        let is_master_branch = branch_name == master_branch_name;

        Ok(Self {
            ref_on_github,
            is_master_branch,
        })
    }

    #[must_use]
    pub fn new_from_branch_name(
        branch_name: &str,
        master_branch_name: &str,
    ) -> Self {
        Self {
            ref_on_github: format!("refs/heads/{branch_name}"),
            is_master_branch: branch_name == master_branch_name,
        }
    }

    #[must_use]
    pub fn on_github(&self) -> &str {
        &self.ref_on_github
    }

    #[must_use]
    pub fn is_master_branch(&self) -> bool {
        self.is_master_branch
    }

    #[must_use]
    pub fn branch_name(&self) -> &str {
        // The branch name is `ref_on_github` with the `refs/heads/` prefix
        // (length 11) removed
        &self.ref_on_github[11..]
    }
}

/// Check if an octocrab error is a 404 Not Found response.
fn is_not_found(err: &octocrab::Error) -> bool {
    matches!(
        err,
        octocrab::Error::GitHub {
            source,
            ..
        } if source.status_code == http::StatusCode::NOT_FOUND
    )
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_new_from_ref_with_branch_name() {
        let r = GitHubBranch::new_from_ref("foo", "masterbranch").unwrap();
        assert_eq!(r.on_github(), "refs/heads/foo");
        assert_eq!(r.branch_name(), "foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_master_branch_name() {
        let r =
            GitHubBranch::new_from_ref("masterbranch", "masterbranch").unwrap();
        assert_eq!(r.on_github(), "refs/heads/masterbranch");
        assert_eq!(r.branch_name(), "masterbranch");
        assert!(r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_ref_name() {
        let r = GitHubBranch::new_from_ref("refs/heads/foo", "masterbranch")
            .unwrap();
        assert_eq!(r.on_github(), "refs/heads/foo");
        assert_eq!(r.branch_name(), "foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_master_ref_name() {
        let r = GitHubBranch::new_from_ref(
            "refs/heads/masterbranch",
            "masterbranch",
        )
        .unwrap();
        assert_eq!(r.on_github(), "refs/heads/masterbranch");
        assert_eq!(r.branch_name(), "masterbranch");
        assert!(r.is_master_branch());
    }

    #[test]
    fn test_new_from_branch_name() {
        let r = GitHubBranch::new_from_branch_name("foo", "masterbranch");
        assert_eq!(r.on_github(), "refs/heads/foo");
        assert_eq!(r.branch_name(), "foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_master_branch_name() {
        let r =
            GitHubBranch::new_from_branch_name("masterbranch", "masterbranch");
        assert_eq!(r.on_github(), "refs/heads/masterbranch");
        assert_eq!(r.branch_name(), "masterbranch");
        assert!(r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_edge_case_ref_name() {
        let r = GitHubBranch::new_from_ref(
            "refs/heads/refs/heads/foo",
            "masterbranch",
        )
        .unwrap();
        assert_eq!(r.on_github(), "refs/heads/refs/heads/foo");
        assert_eq!(r.branch_name(), "refs/heads/foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_edge_case_branch_name() {
        let r = GitHubBranch::new_from_branch_name(
            "refs/heads/foo",
            "masterbranch",
        );
        assert_eq!(r.on_github(), "refs/heads/refs/heads/foo");
        assert_eq!(r.branch_name(), "refs/heads/foo");
        assert!(!r.is_master_branch());
    }
}
