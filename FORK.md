# Fork Differences from upstream spacedentist/spr

This fork adds features for AI-agent automation and general usability improvements.

## New Commands

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `spr status` | Show status of change requests in the stack | `--all`, `--ready` |
| `spr check` | Check CI/merge status of change requests | `--cherry-pick` |
| `spr sync` | Sync local branch with remote | `--update`, `--message` |

## New Flags

| Flag | Command(s) | Description |
|------|-----------|-------------|
| `--all` / `-a` | diff, land | Operate on all commits in the stack |
| `--count` / `-n` | diff | Limit number of commits to process |
| `--dry-run` | (global) | Preview remote actions without performing them (local git still runs) |
| `--verbose` | (global) | Show detailed progress for each action |
| `--quiet` / `-q` | (global) | Suppress decorative output |
| `--non-interactive` | (global) | Skip all prompts |
| `--cherry-pick` | check | Check with cherry-pick strategy |
| `--ready` | status | Show only ready-to-merge change requests |
| `--update` | sync | Update change request messages |
| `--message` / `-m` | sync, diff | Specify commit message |
| `--draft` | diff | Create as draft |
| `--label` / `-l` | diff | Add labels to change request |
| `--refs` / `-r` | diff | Specify refs |

## Configuration

| Key | Type | Description | Default |
|-----|------|-------------|---------|
| `spr.mergeMethod` | string | Merge method: squash, rebase, merge | squash |
| `spr.requireApproval` | bool | Require approval before landing | false |
| `spr.requireTestPlan` | bool | Require test plan section in commit message | true |
| `spr.createDraftPRs` | bool | Create change requests as draft by default | false |
| `spr.branchPrefix` | string | Prefix for spr-managed branches | spr/main/ |

## Authentication

Token resolution order:
1. `--github-auth-token` CLI flag
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` command (gh CLI)
4. `spr.githubAuthToken` git config

## Build & Dependencies

- **TLS:** Migrated from OpenSSL to rustls (octocrab `rustls` feature). git2 still uses vendored OpenSSL (libgit2 limitation).
- **CI:** Modern multi-platform release workflow (replaces legacy macos-10.15).
- **hyper-rustls:** Explicit dependency to fix octocrab 0.45.0 missing `rustls-native-certs`.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Auth/config error (missing token, invalid githubRepository, branchPrefix) |
| 3 | Conflict error (merge conflict, cherry-pick failure) |
| 4 | Change request state error (closed, not approved) |
| 130 | Interrupted (user abort) |

## Behavioral Changes

- **Short change request refs:** `owner/repo#123` format in output
- **Stack view:** Change request bodies include a stack view section with HTML markers, updated on every `spr diff`
- **Quiet mode:** `--quiet` suppresses decorative output; essential output (URLs, numbers) always shown
- **Non-interactive mode:** `--non-interactive` skips all prompts (for CI/automation)
- **WIP skip:** Commits prefixed with `WIP` are skipped during `spr diff --all`
- **Forge-agnostic architecture:** Internal `ForgeApi` trait enables future Forgejo/GitLab support
- **Dry-run limitation:** `--dry-run` prevents remote side effects (push, API calls) but local git operations (cherry-pick, rewrite) still execute
- **Existing PRs:** On first `spr diff` after upgrade, existing PRs will have a stack section appended to their body
- **Config key rename:** `spr.githubMasterBranch` → `spr.githubDefaultBranch` (old key still works as fallback)
