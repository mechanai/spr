/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use color_eyre::Result;

use crate::forge::{ForgeApi, ReviewStatus};

pub async fn list(forge: &dyn ForgeApi) -> Result<()> {
    let summaries = forge.list_open_change_requests().await?;
    let term = console::Term::stdout();
    for pr in &summaries {
        let decision = match &pr.review_status {
            Some(ReviewStatus::Approved) => {
                console::style("Accepted").green()
            }
            Some(ReviewStatus::Rejected) => {
                console::style("Changes Requested").red()
            }
            Some(ReviewStatus::Requested) | None => {
                console::style("Pending")
            }
        };
        term.write_line(&format!(
            "{} {} {}",
            decision,
            console::style(&pr.title).bold(),
            console::style(&pr.url).dim(),
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{ForgeApiMock, OpenChangeRequestSummary};
    use unimock::*;

    #[tokio::test(flavor = "current_thread")]
    async fn list_empty_results() {
        let forge = Unimock::new(
            ForgeApiMock::list_open_change_requests
                .some_call(matching!())
                .returns(Ok(vec![])),
        );
        let result = list(&forge).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_renders_approved_pr() {
        let forge = Unimock::new(
            ForgeApiMock::list_open_change_requests
                .some_call(matching!())
                .returns(Ok(vec![OpenChangeRequestSummary {
                    number: 42,
                    title: "feat: add foo".into(),
                    url: "https://github.com/owner/repo/pull/42".into(),
                    review_status: Some(ReviewStatus::Approved),
                }])),
        );
        let result = list(&forge).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_renders_rejected_pr() {
        let forge = Unimock::new(
            ForgeApiMock::list_open_change_requests
                .some_call(matching!())
                .returns(Ok(vec![OpenChangeRequestSummary {
                    number: 7,
                    title: "fix: broken thing".into(),
                    url: "https://github.com/owner/repo/pull/7".into(),
                    review_status: Some(ReviewStatus::Rejected),
                }])),
        );
        let result = list(&forge).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_renders_pending_pr() {
        let forge = Unimock::new(
            ForgeApiMock::list_open_change_requests
                .some_call(matching!())
                .returns(Ok(vec![OpenChangeRequestSummary {
                    number: 1,
                    title: "chore: update deps".into(),
                    url: "https://github.com/owner/repo/pull/1".into(),
                    review_status: None,
                }])),
        );
        let result = list(&forge).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_renders_multiple_prs() {
        let forge = Unimock::new(
            ForgeApiMock::list_open_change_requests
                .some_call(matching!())
                .returns(Ok(vec![
                    OpenChangeRequestSummary {
                        number: 1,
                        title: "first".into(),
                        url: "https://github.com/o/r/pull/1".into(),
                        review_status: Some(ReviewStatus::Approved),
                    },
                    OpenChangeRequestSummary {
                        number: 2,
                        title: "second".into(),
                        url: "https://github.com/o/r/pull/2".into(),
                        review_status: Some(ReviewStatus::Rejected),
                    },
                    OpenChangeRequestSummary {
                        number: 3,
                        title: "third".into(),
                        url: "https://github.com/o/r/pull/3".into(),
                        review_status: Some(ReviewStatus::Requested),
                    },
                ])),
        );
        let result = list(&forge).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_propagates_forge_error() {
        let forge = Unimock::new(
            ForgeApiMock::list_open_change_requests
                .some_call(matching!())
                .returns(Err(color_eyre::eyre::eyre!("API error"))),
        );
        let result = list(&forge).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API error"));
    }
}
