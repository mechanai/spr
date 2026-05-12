/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::collections::HashMap;

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
        let new_title = message
            .get(&crate::message::MessageSection::Title)
            .cloned();
        if new_title.as_deref() != Some(&cr.title) {
            self.title = new_title;
        }
        let new_body = crate::message::build_github_body(message);
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
            sections: Default::default(),
            reviewers: Default::default(),
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
