/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! GitHub authentication token resolution.
//!
//! Resolves a GitHub auth token from multiple sources in priority order:
//! 1. `GITHUB_TOKEN` environment variable
//! 2. `gh auth token` CLI command
//! 3. `spr.githubAuthToken` in git config

use log::debug;

/// Attempt to find a GitHub auth token from environment and gh CLI.
/// Returns `None` if no token is found from these sources (caller should
/// fall back to git config).
#[must_use]
pub fn find_token(github_host: &str) -> Option<String> {
    if let Some(token) = from_env() {
        return Some(token);
    }

    if let Some(token) = from_gh_cli(github_host) {
        return Some(token);
    }

    None
}

fn from_env() -> Option<String> {
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) if !token.is_empty() => {
            debug!("Using token from GITHUB_TOKEN environment variable");
            Some(token)
        }
        _ => None,
    }
}

fn from_gh_cli(github_host: &str) -> Option<String> {
    let output = match std::process::Command::new("gh")
        .args(["auth", "token", "--hostname", github_host])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("gh CLI not found, skipping gh auth token lookup");
            return None;
        }
        Err(_) => return None,
    };

    if !output.status.success() {
        debug!("gh auth token failed for host {github_host} (not logged in?)");
        return None;
    }

    let token = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if token.is_empty() {
        return None;
    }

    debug!("Using token from gh CLI for host {github_host}");
    Some(token)
}
