/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! Forge authentication token resolution.
//!
//! Provides the [`ForgeTokenResolver`] trait and a GitHub implementation.

use color_eyre::eyre::Result;
use log::debug;
use secrecy::SecretString;

/// Resolve an auth token for a forge.
///
/// Intentionally synchronous — all current resolvers are blocking
/// (env var lookup, subprocess invocation). Migrate to async only
/// if a future resolver requires it.
pub trait ForgeTokenResolver {
    /// Resolve an auth token.
    ///
    /// - `Ok(Some(token))` — token found.
    /// - `Ok(None)` — no token configured.
    /// - `Err(...)` — resolver failure.
    fn resolve(&self) -> Result<Option<SecretString>>;
}

/// Resolves GitHub auth tokens from multiple sources.
///
/// Priority: env var (`GITHUB_TOKEN`) → `gh auth token` CLI → git config value.
pub struct GitHubTokenResolver {
    host: String,
    git_config_value: Option<String>,
}

impl GitHubTokenResolver {
    #[must_use]
    pub fn new(host: String, git_config_value: Option<String>) -> Self {
        Self {
            host,
            git_config_value,
        }
    }
}

impl ForgeTokenResolver for GitHubTokenResolver {
    fn resolve(&self) -> Result<Option<SecretString>> {
        // 1. GITHUB_TOKEN env var
        if let Some(token) = from_env("GITHUB_TOKEN") {
            return Ok(Some(token));
        }

        // 2. gh auth token CLI
        if let Some(token) = from_gh_cli(&self.host) {
            return Ok(Some(token));
        }

        // 3. Git config fallback
        if let Some(ref value) = self.git_config_value
            && !value.is_empty()
        {
            debug!("Using token from git config");
            return Ok(Some(SecretString::from(value.clone())));
        }

        Ok(None)
    }
}

fn from_env(var_name: &str) -> Option<SecretString> {
    match std::env::var(var_name) {
        Ok(token) if !token.is_empty() => {
            debug!("Using token from {var_name} environment variable");
            Some(SecretString::from(token))
        }
        _ => None,
    }
}

fn from_gh_cli(host: &str) -> Option<SecretString> {
    let output = match std::process::Command::new("gh")
        .args(["auth", "token", "--hostname", host])
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
        debug!("gh auth token failed for host {host} (not logged in?)");
        return None;
    }

    let token = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if token.is_empty() {
        return None;
    }

    debug!("Using token from gh CLI for host {host}");
    Some(SecretString::from(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret as _;

    #[test]
    fn resolver_with_git_config_value() {
        let resolver = GitHubTokenResolver::new(
            "github.com".into(),
            Some("test-token".into()),
        );
        let result = resolver.resolve().unwrap();
        if let Some(token) = result {
            assert!(!token.expose_secret().is_empty());
        }
    }

    #[test]
    fn resolver_with_no_sources() {
        let resolver = GitHubTokenResolver::new(
            "no-such-host.example.com".into(),
            None,
        );
        let _result = resolver.resolve().unwrap();
    }

    #[test]
    fn from_env_returns_secret() {
        // SAFETY: test-only, single-threaded access to unique env var
        unsafe { std::env::set_var("SPR_TEST_TOKEN_42", "secret123") };
        let token = from_env("SPR_TEST_TOKEN_42").unwrap();
        assert_eq!(token.expose_secret(), "secret123");
        unsafe { std::env::remove_var("SPR_TEST_TOKEN_42") };
    }

    #[test]
    fn from_env_empty_returns_none() {
        // SAFETY: test-only, single-threaded access to unique env var
        unsafe { std::env::set_var("SPR_TEST_TOKEN_EMPTY", "") };
        assert!(from_env("SPR_TEST_TOKEN_EMPTY").is_none());
        unsafe { std::env::remove_var("SPR_TEST_TOKEN_EMPTY") };
    }
}
