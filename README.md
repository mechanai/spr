![spr](./docs/spr.svg)

# spr &middot; [![GitHub](https://img.shields.io/github/license/spacedentist/spr)](https://img.shields.io/github/license/spacedentist/spr) [![GitHub release](https://img.shields.io/github/v/release/spacedentist/spr?include_prereleases)](https://github.com/spacedentist/spr/releases) [![crates.io](https://img.shields.io/crates/v/spr.svg)](https://crates.io/crates/spr) [![homebrew](https://img.shields.io/homebrew/v/spr.svg)](https://formulae.brew.sh/formula/spr) [![GitHub Repo stars](https://img.shields.io/github/stars/spacedentist/spr?style=social)](https://github.com/spacedentist/spr)

A command-line tool for submitting and updating GitHub Pull Requests from local
Git commits that may be amended and rebased. Pull Requests can be stacked to
allow for a series of code reviews of interdependent code.

spr is pronounced /ˈsuːpəɹ/, like the English word 'super'.

## Documentation

Comprehensive documentation is available here: https://spacedentist.github.io/spr/

## Installation

[![Packaging status](https://repology.org/badge/vertical-allrepos/spr-super-pull-requests.svg)](https://repology.org/project/spr-super-pull-requests/versions)

### Binary Installation

#### Using Homebrew

```shell
brew install spr
```

#### Using Nix

spr is available in nixpkgs

```shell
nix run nixpkgs#spr
```

#### Using Cargo

If you have Cargo installed (the Rust build tool), you can install spr by running

```shell
cargo install spr
```

### Install from Source

spr is written in Rust. You need a Rust toolchain to build from source. See [rustup.rs](https://rustup.rs) for information on how to install Rust if you have not got a Rust toolchain on your system already.

With Rust all set up, clone this repository and run `cargo build --release`. The spr binary will be in the `target/release` directory.

## Quickstart

To use spr, run `spr init` inside a local checkout of a GitHub-backed git repository. You will be guided through authorising spr to use the GitHub API in order to create and merge pull requests.

To submit a commit for pull request, run `spr diff`.

If you want to make changes to the pull request, amend your local commit (and/or rebase it) and call `spr diff` again. When updating an existing pull request, spr will ask you for a short message to describe the update.

To squash-merge an open pull request, run `spr land`.

For more information on spr commands and options, run `spr help`. For more information on a specific spr command, run `spr help <COMMAND>` (e.g. `spr help diff`).

## Commands

| Command | Description |
|---------|-------------|
| `spr init` | Interactive setup for configuring spr in a repository |
| `spr diff` | Create or update a Pull Request for the HEAD commit |
| `spr land` | Merge an approved Pull Request |
| `spr status` | Show PR review status for HEAD commit |
| `spr check` | Validate commit message and optionally test cherry-pick viability |
| `spr sync` | Rebase local branch onto upstream and optionally update PRs |
| `spr amend` | Update local commit message from GitHub |
| `spr format` | Reformat commit message |
| `spr list` | List open Pull Requests and their review status |
| `spr patch` | Create a branch from an existing Pull Request |
| `spr close` | Close a Pull Request |

### Working with stacks

```shell
# Create/update PRs for all commits in the branch
spr diff --all

# Limit to the top N commits
spr diff --all --count 3

# Land all approved PRs bottom-to-top
spr land --all

# Check if all PRs are approved (useful in CI)
spr status --ready

# Rebase and update all PRs in one step
spr sync --update
```

### Automation and CI

spr can be driven non-interactively by AI agents or CI systems:

```shell
# Prevent interactive prompts
spr diff --non-interactive
# or
SPR_NON_INTERACTIVE=1 spr diff

# Suppress decorative output, only print essential data (URLs, PR numbers)
spr diff --quiet
# or
SPR_QUIET=1 spr diff
```

Exit codes for scripting:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Generic error |
| 2 | Auth/config issue |
| 3 | Merge conflict |
| 4 | PR state issue (closed, not approved) |
| 130 | User abort |

## Configuration

Git config options (set via `git config`):

| Key | Description | Default |
|-----|-------------|---------|
| `spr.githubAuthToken` | GitHub personal access token | (from `GITHUB_TOKEN` env or gh CLI) |
| `spr.githubRepository` | Repository as `owner/repo` | (required) |
| `spr.githubMasterBranch` | Target branch name | `master` |
| `spr.branchPrefix` | Prefix for PR branches | `spr/<username>/` |
| `spr.createDraftPRs` | Create new PRs as drafts | `false` |
| `spr.defaultReviewers` | Comma-separated reviewer list | (none) |
| `spr.mergeMethod` | `squash`, `rebase`, or `merge` | `squash` |
| `spr.requireApproval` | Require approval before landing | `false` |
| `spr.requireTestPlan` | Require Test Plan in commit message | `true` |

### Authentication

spr resolves GitHub tokens in this order:

1. `--github-auth-token` CLI flag
2. `GITHUB_TOKEN` environment variable
3. `~/.config/gh/hosts.yml` (gh CLI config)
4. `spr.githubAuthToken` git config (set by `spr init`)

## Contributing

Feel free to submit an issue on [GitHub](https://github.com/spacedentist/spr) if you have found a problem. If you can even provide a fix, please raise a pull request!

If there are larger changes or features that you would like to work on, please raise an issue on GitHub first to discuss.

### License

spr is [MIT licensed](./LICENSE).
