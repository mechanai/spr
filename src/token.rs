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
//! 2. `gh` CLI config (`~/.config/gh/hosts.yml`)
//! 3. `spr.githubAuthToken` in git config

use std::path::PathBuf;

use log::debug;

/// Attempt to find a GitHub auth token from environment and gh CLI config.
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
    let hosts_path = gh_hosts_path()?;
    let contents = std::fs::read_to_string(&hosts_path).ok()?;

    // The gh hosts.yml format is:
    // github.com:
    //     user: <username>
    //     oauth_token: <token>
    //     git_protocol: https
    //
    // We parse minimally to avoid adding a YAML dependency.
    parse_gh_hosts(&contents, github_host)
}

fn gh_hosts_path() -> Option<PathBuf> {
    // Respect GH_CONFIG_DIR if set
    if let Ok(config_dir) = std::env::var("GH_CONFIG_DIR") {
        let path = PathBuf::from(config_dir).join("hosts.yml");
        if path.exists() {
            return Some(path);
        }
    }

    // XDG_CONFIG_HOME or ~/.config
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .ok()
        .filter(|p| p.is_absolute())
        .or_else(|| dirs_path().map(|home| home.join(".config")))?;

    let path = config_dir.join("gh").join("hosts.yml");
    if path.exists() { Some(path) } else { None }
}

fn dirs_path() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Parse the gh CLI hosts.yml to extract `oauth_token` for a given host.
/// Handles the simple YAML structure without requiring a full YAML parser.
fn parse_gh_hosts(contents: &str, github_host: &str) -> Option<String> {
    let mut in_host_block = false;

    for line in contents.lines() {
        let trimmed = line.trim();

        // Top-level host key (no leading whitespace, ends with colon)
        if !line.starts_with(' ')
            && !line.starts_with('\t')
            && trimmed.ends_with(':')
        {
            let host = trimmed.trim_end_matches(':');
            in_host_block = host == github_host;
            continue;
        }

        if in_host_block {
            // Look for oauth_token field
            if let Some(token) = trimmed.strip_prefix("oauth_token:") {
                let token = token.trim().trim_matches('"').trim_matches('\'');
                if !token.is_empty() {
                    debug!(
                        "Using token from gh CLI config for host {github_host}"
                    );
                    return Some(token.to_string());
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gh_hosts_basic() {
        let yaml = "\
github.com:
    user: testuser
    oauth_token: gho_abc123
    git_protocol: https
";
        assert_eq!(
            parse_gh_hosts(yaml, "github.com"),
            Some("gho_abc123".to_string())
        );
    }

    #[test]
    fn test_parse_gh_hosts_multiple_hosts() {
        let yaml = "\
github.com:
    user: testuser
    oauth_token: gho_abc123
    git_protocol: https
github.enterprise.com:
    user: corpuser
    oauth_token: ghp_enterprise456
    git_protocol: ssh
";
        assert_eq!(
            parse_gh_hosts(yaml, "github.enterprise.com"),
            Some("ghp_enterprise456".to_string())
        );
    }

    #[test]
    fn test_parse_gh_hosts_not_found() {
        let yaml = "\
github.com:
    user: testuser
    oauth_token: gho_abc123
";
        assert_eq!(parse_gh_hosts(yaml, "other.host.com"), None);
    }

    #[test]
    fn test_parse_gh_hosts_quoted_token() {
        let yaml = "\
github.com:
    oauth_token: \"gho_quoted\"
";
        assert_eq!(
            parse_gh_hosts(yaml, "github.com"),
            Some("gho_quoted".to_string())
        );
    }

    #[test]
    fn test_parse_gh_hosts_empty_token() {
        let yaml = "\
github.com:
    oauth_token:
";
        assert_eq!(parse_gh_hosts(yaml, "github.com"), None);
    }
}
