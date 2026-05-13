---
name: stacked-prs
description: Use when managing stacked pull requests, creating PR chains, landing dependent PRs in order, rebasing stacks, or when the user mentions spr/stacked-diffs/stacked-PRs workflow
compatibility: Requires spr binary (https://github.com/spacedentist/spr) and git
---

# Stacked PRs with spr

## Overview

Manage stacked pull requests using `spr` — a tool that maps individual Git commits to GitHub PRs, allowing independent review and landing of each change in a dependency chain.

## Mental Model

```
commit C (HEAD)  →  PR #3 (depends on PR #2)
commit B         →  PR #2 (depends on PR #1)
commit A         →  PR #1 (targets main)
```

Each commit = one PR. Amend a commit → `spr diff` updates its PR. Rebase → all PRs rebase.

## Quick Reference

| Task | Command |
|------|---------|
| Create/update PR for HEAD | `spr diff` |
| Create/update all PRs in stack | `spr diff --all` |
| Create PR as draft | `spr diff --draft` |
| Update PR title/body from commit | `spr diff --update-message` |
| Select specific commits | `spr diff --range HEAD~3..HEAD~1` |
| Limit commits processed | `spr diff --all --count 3` |
| Land bottom PR | `spr land` |
| Land all approved PRs | `spr land --all` |
| Close PR for HEAD commit | `spr close` |
| Rebase + update all PRs | `spr sync --update` |
| Check status of stack | `spr status --all` |
| Check if all approved | `spr status --ready` (exit code 0 = ready) |
| Validate commit before push | `spr check` |
| Validate + test cherry-pick | `spr check --cherry-pick` |
| Preview without side effects | `spr diff --all --dry-run` |
| Detailed progress logging | `spr diff --all --verbose` |

## Global CLI Flags

These flags apply to all subcommands:

| Flag | Purpose |
|------|---------|
| `--cd DIR` | Change to DIR before running |
| `--github-auth-token TOKEN` | Override auth token |
| `--github-repository OWNER/REPO` | Override repository |
| `--github-default-branch BRANCH` | Override target branch |
| `--branch-prefix PREFIX` | Override PR branch prefix |
| `--non-interactive` | Never prompt (also `SPR_NON_INTERACTIVE=1`) |
| `--quiet` / `-q` | Essential output only (also `SPR_QUIET=1`) |
| `--dry-run` | Preview without side effects (also `SPR_DRY_RUN=1`) |
| `--verbose` | Detailed progress logging (also `SPR_VERBOSE=1`) |

## Workflow for AI Agents

### Creating a stack

```bash
# Work on feature in atomic commits (one logical change per commit)
git commit -m "feat: add user model

Summary: Database schema and basic CRUD

Test Plan: unit tests for User model"

git commit -m "feat: add user API endpoints

Summary: REST endpoints for user CRUD

Test Plan: integration tests with test database"

# Push all as PRs
spr diff --all
```

### Non-interactive mode (required for automation)

Always use one of:
- `--non-interactive` flag
- `SPR_NON_INTERACTIVE=1` environment variable

This prevents blocking prompts. Without it, `spr diff` will prompt for a commit message when updating existing PRs.

### Dry-run mode (preview without side effects)

```bash
spr diff --all --dry-run
# Previews what would happen without creating/updating PRs or rewriting commits
# Read operations (fetching existing PRs) still work; writes are stubbed
```

Dry-run prevents both GitHub API writes and local commit rewriting (no PR URLs baked into commits). Combine with `--verbose` for maximum visibility:

```bash
spr diff --all --dry-run --verbose
```

### Verbose mode (detailed progress)

```bash
spr diff --all --verbose
# Logs each API call and action as it happens
```

Cannot be combined with `--quiet`.

### Quiet mode for parsing output

```bash
spr diff --all --quiet
# Only outputs essential data: PR URLs, numbers
# Format: owner/repo#123 https://github.com/owner/repo/pull/123
```

Also available as `SPR_QUIET=1` environment variable.

### Landing in order

```bash
# Check readiness first
spr status --ready || echo "Not all approved"

# Land bottom-up (stops at first failure)
spr land --all
```

### After landing

spr automatically:
1. Merges the PR on GitHub
2. Rebases remaining commits onto the new base
3. Deletes the remote PR branch

### Sync after upstream changes

```bash
spr sync          # fetch + rebase only
spr sync --update # fetch + rebase + update all PRs
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Generic error |
| 2 | Auth/config issue (no token, bad repo config) |
| 3 | Conflict (cherry-pick failed, merge conflict) |
| 4 | PR state issue (closed, not approved) |
| 130 | User abort |

## Configuration

Git config keys (set via `git config`):

| Key | Purpose | Default |
|-----|---------|---------|
| `spr.createDraftPRs` | Create PRs as drafts | false |
| `spr.defaultReviewers` | Comma-separated reviewer list | (none) |
| `spr.mergeMethod` | squash/rebase/merge | squash |
| `spr.requireApproval` | Require approval before land | false |
| `spr.requireTestPlan` | Require Test Plan section | true |

## Auth

Token resolution order (first match wins):
1. `--github-auth-token` CLI flag
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` (gh CLI, must be logged in)
4. `spr.githubAuthToken` git config

For CI/automation, set `GITHUB_TOKEN`.

## Commit Message Format

```
feat: short title

Summary: Longer description of the change

Test Plan: How to verify this works

Reviewers: username1, #team-name
```

- **Title**: First line, used as PR title
- **Summary**: PR body
- **Test Plan**: Required by default (disable with `spr.requireTestPlan=false`)
- **Reviewers**: Optional, requested on PR creation

## Common Patterns

### Skip WIP commits in a stack
Prefix title with `WIP` or `[WIP]` — `spr diff --all` skips these.

### Apply labels to new PRs
```bash
spr diff --label "type:feature" --label "priority:high"
```

### Cherry-pick mode (no intermediate deps)
```bash
spr diff --cherry-pick   # PR shows only this commit's changes
spr land --cherry-pick   # Land even if parent isn't on main
```

### Close a PR
```bash
spr close                # Close the PR for HEAD commit
```

### Select specific commits in a range
```bash
spr diff --range HEAD~4..HEAD~1   # Only process commits in this revspec
```

### Force-update PR description from commit
```bash
spr diff --update-message  # Overwrite PR title/body with current commit message
```

### Sync with custom update message
```bash
spr sync --update -m "rebase onto latest main"
```

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Exit code 2 | No auth token | Set GITHUB_TOKEN or run `spr init` |
| Exit code 3 | Merge conflict | Rebase locally, resolve, then `spr diff` |
| "cannot land" | Parent not on main | Use `spr land --cherry-pick` or land parent first |
| Prompt blocks CI | Missing non-interactive | Add `--non-interactive` or `SPR_NON_INTERACTIVE=1` |
| Unsure what spr will do | Want to preview | Use `--dry-run` (add `--verbose` for detail) |
