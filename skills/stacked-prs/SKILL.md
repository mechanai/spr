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
| Land bottom PR | `spr land` |
| Land all approved PRs | `spr land --all` |
| Rebase + update all PRs | `spr sync --update` |
| Check status of stack | `spr status --all` |
| Check if all approved | `spr status --ready` (exit code 0 = ready) |
| Validate commit before push | `spr check` |
| Validate + test cherry-pick | `spr check --cherry-pick` |
| Limit commits processed | `spr diff --all --count 3` |

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

### Quiet mode for parsing output

```bash
SPR_QUIET=1 spr diff --all
# Only outputs essential data: PR URLs, numbers
# Format: owner/repo#123 https://github.com/owner/repo/pull/123
```

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

Token resolution order:
1. `GITHUB_TOKEN` environment variable
2. `~/.config/gh/hosts.yml` (gh CLI)
3. `spr.githubAuthToken` git config

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

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Exit code 2 | No auth token | Set GITHUB_TOKEN or run `spr init` |
| Exit code 3 | Merge conflict | Rebase locally, resolve, then `spr diff` |
| "cannot land" | Parent not on main | Use `spr land --cherry-pick` or land parent first |
| Prompt blocks CI | Missing non-interactive | Add `--non-interactive` or `SPR_NON_INTERACTIVE=1` |
