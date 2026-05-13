/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! Typed error variants for structured exit code handling.

use std::fmt;

/// Application-level errors with associated exit codes.
///
/// Commands raise these variants instead of bare string errors so that
/// `main()` can determine the correct exit code without string matching.
#[derive(Debug)]
pub enum SprError {
    /// Exit 2 — authentication or configuration problem.
    Auth(String),
    /// Exit 3 — merge conflict or cherry-pick failure.
    Conflict(String),
    /// Exit 4 — change request state issue (closed, not approved, not found).
    ChangeRequestState(String),
    /// Exit 130 — user chose to abort.
    UserAbort,
}

impl SprError {
    /// Process exit code for this error category.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Auth(_) => 2,
            Self::Conflict(_) => 3,
            Self::ChangeRequestState(_) => 4,
            Self::UserAbort => 130,
        }
    }
}

impl fmt::Display for SprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth(msg) | Self::Conflict(msg) | Self::ChangeRequestState(msg) => {
                f.write_str(msg)
            }
            Self::UserAbort => f.write_str("Aborted as per user request"),
        }
    }
}

impl std::error::Error for SprError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_codes() {
        assert_eq!(SprError::Auth("x".into()).exit_code(), 2);
        assert_eq!(SprError::Conflict("x".into()).exit_code(), 3);
        assert_eq!(SprError::ChangeRequestState("x".into()).exit_code(), 4);
        assert_eq!(SprError::UserAbort.exit_code(), 130);
    }

    #[test]
    fn test_display() {
        assert_eq!(SprError::Auth("no token".into()).to_string(), "no token");
        assert_eq!(SprError::UserAbort.to_string(), "Aborted as per user request");
    }
}
