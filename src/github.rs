/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::eyre::{Error, Result, WrapErr as _, eyre};
use graphql_client::{GraphQLQuery, Response};

use crate::{
    git::PreparedCommit,
    git_remote::GitRemote,
    message::{
        MessageSection, MessageSectionsMap, build_forge_body, parse_message,
    },
};
use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

static OCTOCRAB_INIT: OnceLock<()> = OnceLock::new();

use crate::forge::{
    ChangeRequest, ChangeRequestState, ChangeRequestUpdate, ForgeApi,
    Mergeability, OpenChangeRequestSummary, ReviewStatus, ReviewerRequest,
    TeamInfo, UserInfo,
};

#[allow(clippy::upper_case_acronyms)]
type URI = String;

pub struct GitHub {
    config: crate::config::Config,
    git: crate::git::Git,
    git_remote: crate::git_remote::GitRemote,
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
        auth_token: SecretString,
    ) -> Self {
        OCTOCRAB_INIT.get_or_init(|| {
            octocrab::initialise(
                octocrab::Octocrab::builder()
                    .personal_token(auth_token.expose_secret().to_owned())
                    .build()
                    .expect("Failed to build octocrab client"),
            );
        });

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
        let default_branch_oid = self
            .git_remote
            .fetch_branch(self.config.default_branch_name())?;
        self.git.get_prepared_commits(self, default_branch_oid)
    }

    pub async fn fetch_user(&self, login: &str) -> Result<UserInfo> {
        /// Deserialization wrapper for GitHub GET /users/{login} endpoint.
        /// Forge-neutral `UserInfo` intentionally omits serde derives.
        #[derive(serde::Deserialize)]
        struct GitHubUser {
            login: String,
            name: Option<String>,
            #[serde(default)]
            is_collaborator: bool,
        }

        let u: GitHubUser = octocrab::instance()
            .get(format!("/users/{login}"), None::<&()>)
            .await
            .map_err(Error::from)?;
        Ok(UserInfo {
            login: u.login,
            name: u.name,
            is_collaborator: u.is_collaborator,
        })
    }

    pub async fn fetch_team(
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

    pub async fn fetch_change_request(
        &self,
        number: u64,
    ) -> Result<ChangeRequest> {
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
                .fold(error, |err, e| err.wrap_err(e.to_string()));
        }

        let pr = response_body
            .data
            .ok_or_else(|| eyre!("failed to fetch PR"))?
            .repository
            .ok_or_else(|| eyre!("failed to find repository"))?
            .pull_request
            .ok_or_else(|| eyre!("failed to find PR"))?;

        let base = self.config.new_branch_from_ref(&pr.base_ref_name)?;
        let head = self.config.new_branch_from_ref(&pr.head_ref_name)?;

        let branch_names: Vec<_> =
            [&base, &head].iter().map(|&b| b.branch_name()).collect();

        let [base_oid, head_oid] =
            self.git_remote.fetch_from_remote(&branch_names, &[])?[0..2]
        else {
            unreachable!();
        };

        let base_oid = base_oid.ok_or_else(|| {
            eyre!("{} not found on GitHub", base.full_ref())
        })?;
        let head_oid = head_oid.ok_or_else(|| {
            eyre!("{} not found on GitHub", head.full_ref())
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
            self.change_request_url(number),
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

        Ok(ChangeRequest {
            #[allow(clippy::cast_sign_loss)]
            number: pr.number as u64,
            state: match pr.state {
                pull_request_query::PullRequestState::OPEN => {
                    ChangeRequestState::Open
                }
                pull_request_query::PullRequestState::MERGED => {
                    ChangeRequestState::Merged
                }
                _ => ChangeRequestState::Closed,
            },
            title: pr.title,
            body: Some(pr.body),
            is_draft: pr.is_draft,
            base_ref_name: base.branch_name().to_owned(),
            head_ref_name: head.branch_name().to_owned(),
            sections,
            base_oid,
            head_oid,
            reviewers,
            review_status,
            merge_commit: pr
                .merge_commit
                .and_then(|sha| git2::Oid::from_str(&sha.oid).ok()),
        })
    }

    pub async fn create_change_request_impl(
        &self,
        message: &MessageSectionsMap,
        base_ref_name: String,
        head_ref_name: String,
        draft: bool,
        stack_info: Option<&str>,
    ) -> Result<u64> {
        let mut body = build_forge_body(message);
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

    async fn patch_change_request(
        &self,
        number: u64,
        updates: &PatchBody<'_>,
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

    async fn post_reviewer_request(
        &self,
        number: u64,
        reviewers: &RequestReviewersBody<'_>,
    ) -> Result<()> {
        #[derive(serde::Deserialize)]
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

    pub async fn fetch_mergeability(
        &self,
        number: u64,
    ) -> Result<Mergeability> {
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

        let base = self.config.new_branch_from_ref(&pr.base_ref_name)?;

        Ok(Mergeability {
            base_ref_name: base.branch_name().to_owned(),
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

    pub async fn close_change_request_impl(&self, number: u64) -> Result<()> {
        let updates = PatchBody {
            state: Some("closed"),
            ..Default::default()
        };
        self.patch_change_request(number, &updates).await
    }

    pub async fn merge_change_request_impl(
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
                "{} merge failed: {}",
                self.change_request_term_full(),
                merge.message.unwrap_or_default()
            ))
        }
    }

    async fn list_open_change_requests_impl(
        &self,
    ) -> Result<Vec<OpenChangeRequestSummary>> {
        #[derive(graphql_client::GraphQLQuery)]
        #[graphql(
            schema_path = "src/gql/schema.docs.graphql",
            query_path = "src/gql/open_reviews.graphql",
            response_derives = "Debug"
        )]
        struct SearchQuery;

        let variables = search_query::Variables {
            query: format!(
                "repo:{}/{} is:open is:pr author:@me archived:false",
                self.config.owner, self.config.repo
            ),
        };
        let request_body = SearchQuery::build_query(variables);
        let response: graphql_client::Response<search_query::ResponseData> =
            octocrab::instance()
                .post("/graphql", Some(&request_body))
                .await
                .wrap_err("Searching for open change requests")?;

        let mut summaries = Vec::new();
        if let Some(data) = response.data
            && let Some(nodes) = data.search.nodes
        {
            for node in nodes.into_iter().flatten() {
                if let search_query::SearchQuerySearchNodes::PullRequest(
                    pr,
                ) = node
                {
                    let review_status = match pr.review_decision {
                        Some(
                            search_query::PullRequestReviewDecision::APPROVED,
                        ) => Some(crate::forge::ReviewStatus::Approved),
                        Some(
                            search_query::PullRequestReviewDecision::CHANGES_REQUESTED,
                        ) => Some(crate::forge::ReviewStatus::Rejected),
                        Some(
                            search_query::PullRequestReviewDecision::REVIEW_REQUIRED,
                        ) => Some(crate::forge::ReviewStatus::Requested),
                        _ => None,
                    };
                    summaries.push(OpenChangeRequestSummary {
                        number: pr.number.cast_unsigned(),
                        title: pr.title,
                        url: pr.url,
                        review_status,
                    });
                }
            }
        }
        Ok(summaries)
    }

    /// Fetch the currently authenticated user.
    ///
    /// Note: `is_collaborator` is not checked here — the `/user` endpoint
    /// doesn't reveal collaborator status for a specific repo. Callers
    /// needing that should use a separate collaborator check.
    async fn get_authenticated_user_impl(&self) -> Result<UserInfo> {
        let user = octocrab::instance().current().user().await
            .wrap_err("Fetching authenticated user")?;
        Ok(UserInfo {
            login: user.login,
            name: user.name,
            is_collaborator: false,
        })
    }

    async fn get_repo_default_branch_impl(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<String> {
        fn is_valid_slug(s: &str) -> bool {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        }
        if !is_valid_slug(owner) || !is_valid_slug(repo) {
            return Err(eyre!(
                "Invalid owner/repo: {owner}/{repo}"
            ));
        }

        let repo_info = octocrab::instance()
            .get::<octocrab::models::Repository, _, _>(
                format!("/repos/{owner}/{repo}"),
                None::<&()>,
            )
            .await
            .wrap_err(format!(
                "Fetching repository info for {owner}/{repo}"
            ))?;
        Ok(repo_info
            .default_branch
            .unwrap_or_else(|| "main".to_owned()))
    }

}

/// Serialization wrapper for GitHub PATCH /pulls/{number} endpoint.
/// Forge-neutral `ChangeRequestUpdate` intentionally omits serde derives
/// to stay forge-agnostic; this struct bridges to the GitHub REST API.
#[derive(serde::Serialize, Default)]
struct PatchBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
}

/// Serialization wrapper for GitHub `POST /pulls/{number}/requested_reviewers`.
/// Forge-neutral `ReviewerRequest` intentionally omits serde derives.
#[derive(serde::Serialize)]
struct RequestReviewersBody<'a> {
    reviewers: &'a [String],
    team_reviewers: &'a [String],
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
        self.create_change_request_impl(
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
        let state_str: Option<String> =
            update.state.as_ref().map(|s| match s {
                ChangeRequestState::Open => "open".to_owned(),
                ChangeRequestState::Closed | ChangeRequestState::Merged => {
                    "closed".to_owned()
                }
            });

        let body_value: Option<String> = match (&update.body, stack_info) {
            (Some(body), Some(info)) => {
                Some(crate::stack::update_body_with_stack(body, info))
            }
            (Some(body), None) => Some(body.clone()),
            (None, Some(info)) => {
                let current_body = self
                    .fetch_change_request(number)
                    .await
                    .ok()
                    .and_then(|cr| cr.body)
                    .unwrap_or_default();
                Some(crate::stack::update_body_with_stack(&current_body, info))
            }
            (None, None) => None,
        };

        let patch = PatchBody {
            title: update.title.as_deref(),
            body: body_value.as_deref(),
            base: update.base.as_deref(),
            state: state_str.as_deref(),
        };

        self.patch_change_request(number, &patch).await
    }

    async fn get_change_request(
        &self,
        number: u64,
    ) -> Result<Option<ChangeRequest>> {
        match self.fetch_change_request(number).await {
            Ok(cr) => Ok(Some(cr)),
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
        self.close_change_request_impl(number).await
    }

    async fn merge_change_request(
        &self,
        number: u64,
        method: crate::config::MergeMethod,
        title: &str,
        message: &str,
        expected_head_oid: git2::Oid,
    ) -> Result<()> {
        self.merge_change_request_impl(
            number,
            method,
            title,
            message,
            expected_head_oid,
        )
        .await
    }

    async fn get_mergeability(&self, number: u64) -> Result<Mergeability> {
        self.fetch_mergeability(number).await
    }

    async fn request_reviewers(
        &self,
        number: u64,
        reviewers: &ReviewerRequest,
    ) -> Result<()> {
        let body = RequestReviewersBody {
            reviewers: &reviewers.users,
            team_reviewers: &reviewers.teams,
        };
        self.post_reviewer_request(number, &body).await
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        GitHub::add_labels(self, number, labels).await
    }

    async fn get_user(&self, username: &str) -> Result<Option<UserInfo>> {
        match self.fetch_user(username).await {
            Ok(u) => Ok(Some(u)),
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
        match self.fetch_team(org, team_slug).await {
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

    async fn list_open_change_requests(
        &self,
    ) -> Result<Vec<OpenChangeRequestSummary>> {
        self.list_open_change_requests_impl().await
    }

    async fn get_authenticated_user(&self) -> Result<UserInfo> {
        self.get_authenticated_user_impl().await
    }

    async fn get_repo_default_branch(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<String> {
        self.get_repo_default_branch_impl(owner, repo).await
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

    fn change_request_term_full(&self) -> &'static str {
        "Pull Request"
    }

    fn change_request_url(&self, number: u64) -> String {
        format!(
            "https://github.com/{}/{}/pull/{}",
            self.config.owner, self.config.repo, number
        )
    }

    fn short_cr_ref(&self, number: u64) -> String {
        format!("{}/{}#{}", self.config.owner, self.config.repo, number)
    }

    fn parse_cr_field(&self, text: &str) -> Result<Option<u64>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }

        // GitHub URL pattern
        let url_regex = lazy_regex::regex!(
            r#"^https?://github\.com/([\w\-\.]+)/([\w\-\.]+)/pull/(\d+)([/?#].*)?$"#
        );
        if let Some(caps) = url_regex.captures(text)
            && self.config.owner == caps.get(1).unwrap().as_str()
            && self.config.repo == caps.get(2).unwrap().as_str()
        {
            return Ok(Some(caps.get(3).unwrap().as_str().parse()?));
        }

        // Bare #NNN or NNN
        let bare_regex = lazy_regex::regex!(r#"^#?\s*(\d+)$"#);
        if let Some(caps) = bare_regex.captures(text) {
            return Ok(Some(caps.get(1).unwrap().as_str().parse()?));
        }

        Ok(None)
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
    use color_eyre::eyre::Result;

    fn parse_cr_field_test_helper(owner: &str, repo: &str, text: &str) -> Result<Option<u64>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let url_regex = lazy_regex::regex!(
            r#"^https?://github\.com/([\w\-\.]+)/([\w\-\.]+)/pull/(\d+)([/?#].*)?$"#
        );
        if let Some(caps) = url_regex.captures(text)
            && owner == caps.get(1).unwrap().as_str()
            && repo == caps.get(2).unwrap().as_str()
        {
            return Ok(Some(caps.get(3).unwrap().as_str().parse()?));
        }
        let bare_regex = lazy_regex::regex!(r#"^#?\s*(\d+)$"#);
        if let Some(caps) = bare_regex.captures(text) {
            return Ok(Some(caps.get(1).unwrap().as_str().parse()?));
        }
        Ok(None)
    }

    #[test]
    fn test_parse_cr_field_empty() {
        assert_eq!(parse_cr_field_test_helper("acme", "codez", "").unwrap(), None);
        assert_eq!(parse_cr_field_test_helper("acme", "codez", "   ").unwrap(), None);
    }

    #[test]
    fn test_parse_cr_field_bare_number() {
        assert_eq!(parse_cr_field_test_helper("acme", "codez", "123").unwrap(), Some(123));
        assert_eq!(parse_cr_field_test_helper("acme", "codez", "   123 ").unwrap(), Some(123));
        assert_eq!(parse_cr_field_test_helper("acme", "codez", "#123").unwrap(), Some(123));
        assert_eq!(parse_cr_field_test_helper("acme", "codez", " # 123").unwrap(), Some(123));
    }

    #[test]
    fn test_parse_cr_field_github_url() {
        assert_eq!(
            parse_cr_field_test_helper("acme", "codez", "https://github.com/acme/codez/pull/123").unwrap(),
            Some(123)
        );
        assert_eq!(
            parse_cr_field_test_helper("acme", "codez", "https://github.com/acme/codez/pull/123/").unwrap(),
            Some(123)
        );
        assert_eq!(
            parse_cr_field_test_helper("acme", "codez", "https://github.com/acme/codez/pull/123?x=a").unwrap(),
            Some(123)
        );
        assert_eq!(
            parse_cr_field_test_helper("acme", "codez", "https://github.com/acme/codez/pull/123#abc").unwrap(),
            Some(123)
        );
    }

    #[test]
    fn test_parse_cr_field_wrong_repo() {
        assert_eq!(
            parse_cr_field_test_helper("acme", "codez", "https://github.com/other/repo/pull/123").unwrap(),
            None
        );
    }
}
