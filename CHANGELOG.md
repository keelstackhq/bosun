# Changelog

All notable changes to bosun. The format follows [Keep a Changelog](https://keepachangelog.com/).

## Unreleased

- `bosun fail`: stop rules (3 equal failures or 0 changed lines block the task) and tier escalation, with `failure` and `escalate` records.
- `bosun claim` refuses new work with exit 6 at 80 % of the daily budget, like `spawn` check 2; mainfix tasks and a re-claim of your own lease are exempt (UC-M4-1).
- `bosun budget check`: the level (normal, guard, stop) of today's spend; at 100 % one `interrupt_session` per live remote session in `next`, for the janitor (UC-M4-2).
- `bosun budget record`: the record that blocks a task over `per_task_usd` opens a `budget` lesson, or adds a hit to it (UC-M4-3).
- `bosun progress [--days N]`: cost per merged task over the last N days, read-only (UC-M4-4).
- `wave plan` and `status` read `.bosun/limits.toml` from the default branch, like `spawn`, `claim` and `budget record`; `status` reports `daily_usd`.
- `bosun pr` moves a claimed task through building to review; `bosun pr` and `bosun reap` move a queued task to merged once GitHub merged its PR (agents never merge).
- `bosun fail`: a stop opens or hits a `stop-rule` lesson.
- `bosun resume`: after a container restart, renew the session's join record and leases and list the open PR and next action of each held task.
- `bosun result <epic> --outcome .. [--pr N]`: a child posts its result, ends its epic lease and leaves the registry; the spawn prompt uses it.
- `bosun pr --auto-merge` works in Claude Code cloud sessions: it falls back to the CCR auto-merge route when GraphQL is refused; the merge-lock also blocks `gh api` auto-merge calls.
- The janitor Routine prompt (`bosun doctor`) runs `bosun budget check` and makes its `interrupt_session` calls.
- The splice channel `gh:keelstackhq/bosun//channel` (D16): pieces `core` (governance seeds, the `bosun`, `risk`, `verdict`, `main-open`, `main-health` and `splice` workflows, the AGENTS.md and REVIEW.md sections), `rust`, `ts-pnpm` and the optional `claude-md`. The gates run in `.github/workflows/bosun.yml` (aggregate `ci-ok`, or `bosun-ok` beside a repo's own ci.yml); consumer workflows run the release pinned in `.bosun/version`, never PR code.
- `bosun init --stack <rust|ts-pnpm>... [--plan FILE] [--owner LOGIN] [--ref REF] [--bosun-version X.Y.Z] [--no-pr]`: adds the channel on branch `bosun/init` and opens one R-gov setup PR with the owner steps. Beside a hull template it sets `own_ci`, skips `claude-md` when the template owns CLAUDE.md, excludes existing seeds and `.claude/settings.json`, and merges the required checks into an existing hoist.toml.
- Gate profiles: `manifest` (a stack whose manifest is missing at the head and the base is skipped, so an empty repo's setup PR is green), `requires`, the `{packages}` placeholder and a closed set of packages adapters (`cargo-metadata`, `pnpm-workspace`, `none`, `generic`).
- The `ts-pnpm` profile (root `package.json` scripts, `pnpm --filter`), the `ts-skip` and `ts` test-integrity adapters, and generic `regex` adapters for any stack, reported as reduced integrity; mixed stacks get one kit per stack.
- `bosun pr --auto-merge` and `bosun doctor` read the required checks as the defaults plus hoist.toml `[github] required_checks` from the default branch; doctor finds `ci-ok` in any workflow.
- `bosun doctor --remote` runs `hoist check` and folds its drift into steps 3, 5 and 10.
- `.bosun/risk.toml`: `channel/**` is R-ci.

## v0.1.0

The first release: the seed (S1-S10, `docs/superpowers/plans/2026-10-22-bosun-seed.md`).

- One JSON envelope on stdout for every command, fixed exit codes 0-6, `error.fix` always set.
- Identity, config and time: `BOSUN_REPO`, `BOSUN_SESSION`, `.bosun/*.toml`, `BOSUN_NOW`.
- Coordination on GitHub issues: `join`, `claim`, `heartbeat`, `release`, `reap`, `status`, with
  leases in the registry issue and the kill switch (an owner's `bosun:halt` issue).
- Spawning and limits: `spawn`, `wave plan`, `wait`, `msg`, `budget record`; the
  claude-code-remote calls come back in `next`.
- Gates: `gate` (rust profile), `test-integrity`, `ci-log`.
- Review and risk: `review-pack`, `verdict post|comment|apply`, `risk-check`, `pr --auto-merge`,
  `ask`.
- Learning loop: `lesson add`, `lesson hit`.
- Hooks: `hook pre-tool-use` (edit-, label- and merge-lock) and `hook session-start`.
- `doctor`, which also prints the janitor, curator and notify Routine prompts.

Install: `npm i -g @keelstackhq/bosun`, `cargo install keel-bosun`, or a binary from the
GitHub Release.

### Added after the seed

- `risk-check` accepts an owner's PR comment `/bosun approve <head sha>` (full 40-character SHA,
  unedited) as the approval of a high-risk PR, beside a GitHub review. The `risk` workflow also
  runs on such comments, from the default branch.
- Kill switch: an open issue labelled `bosun:halt` and opened by an owner halts bosun (exit 4).
  Create the label with `gh label create bosun:halt --repo <owner>/<repo> --color B60205`.
- `bosun verdict comment` posts a verdict as a PR comment, and the `verdict` workflow runs
  `bosun verdict apply <comment-id>` to check it and post the `review-verdict` status, so cloud
  sessions (which cannot write statuses) can deliver verdicts.

### Fixed

- The `verdict` workflow has `pull-requests: write`: with `issues: write` alone, its verdict record
  comment on the PR got 403 after the status was posted (first full run, #73).
- `bosun verdict apply` reads only the ```json block of a verdict comment: the agent proxy
  appends a footer to every comment, which made the second verdict comment on #73 invalid.
- `bosun verdict apply` checks the author's repository permission (write or admin) instead of
  `author_association`: the workflow token sees an org member as `CONTRIBUTOR`, so the first
  verdict comment (#73) was refused.

### Changed

- risk-check counts a `/bosun approve` comment only when GitHub says no app wrote it
  (`performed_via_github_app` present and `null`). Agent sessions comment as the owner's login
  through the Claude app, so before this an agent could approve its own PR.
- The `risk` workflow runs on `pull_request_target` instead of `pull_request`: the judging job
  comes from the base branch, never checks out PR-head code, passes event data through `env`
  only, and has least permissions. `tests/workflows.rs` asserts it.
- The repo variable `BOSUN_HALT` is now a second, optional kill-switch source: 404 (unset) and 403
  (not readable from this token) are no signal; any other read error still halts. Before, a 403
  halted every session behind the agent proxy.
- `bosun doctor` step 5 ("kill switch") checks the `bosun:halt` label instead of the variable.
