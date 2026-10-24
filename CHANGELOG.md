# Changelog

All notable changes to bosun. The format follows [Keep a Changelog](https://keepachangelog.com/).

## Unreleased

## v0.2.0

Released 2026-10-24.

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
- `bosun plan check <file>` and `bosun plan import <file>`: epics, tasks and one plan task per item from a plan file; a second import creates nothing (UC-M2-1).
- `bosun plan import` moves an item's tasks to ready once its plan task merged and writes stubs for missing UC tests (UC-M2-2, UC-M2-3).
- `bosun plan amend <file>`: items and tasks gone from the plan close as not planned; changed ones update in place (UC-M2-4).
- `bosun intake "<text>" --source ..`: opens a kind/plan issue that asks for a plan amendment.
- `bosun lesson capture`: a failure opens a lesson issue, or adds a hit to the open lesson with the same signature (UC-M5-1).
- Process-red: an open `bosun:process-red` issue pauses `claim`, `spawn` and `wave plan` for all but mainfix and process-fix work (UC-M5-2).
- `bosun lesson triage <issue> --as blocking|mechanizable|advice`, called by the curator Routine prompt.
- A hit on a mechanized lesson reopens it as blocking (UC-M5-5); `lesson add --upstream`; `bosun progress --lessons`.
- The merge-lock follows `cd`, `git -C`, `--git-dir` and `--work-tree`; the label-lock reads label fields only.
- Paging works through the Claude Code cloud proxy.
- `risk.yml` and `verdict.yml` run the pinned bosun release instead of building main.
- `bosun review-ci <pr>`: review mode (b). The review verdict comes from one Anthropic API call in CI, with no session (UC-M8-1); it fails closed on a diff it cannot see or a request over the cap, and does nothing unless `review.mode = "ci"`. `--dry-run` records a verdict and writes nothing.
- A later review covers only the new commits, and only from an approved ancestor that is one of the PR's commits (UC-M8-2).
- `core::agreement` and the recorded-review fixtures for comparing mode (a) with mode (b) (UC-M8-3 waits for 20 recordings).
- risk-check flags a file with changed lines and no patch whatever its name.
- `bosun notify`: owner questions, halt, main-red, budget-halt and thread results as `slack_post` items in `next`, posted by Claude Tag or the notify Routine and acknowledged with `--ack`, so each post is made once; it refuses a channel that is shared or has guests (UC-M6-1, UC-M6-4). bosun holds no Slack token.
- `bosun answer`: an owner answers an ask from Slack; only owner Slack IDs count and the answer never changes the task state (UC-M6-2).
- `bosun spawn --slack-thread`: a Slack thread spawns a child (a new request goes through intake), with the same checks and budget as any spawn; the result is posted back into the thread (UC-M6-3, UC-M6-4).

### Features

- plan import writes no UC stubs; UC tests land with their code (#179) (#199) (b0e0d48)
- channel workflows run the pinned bosun (2/2) (#109) (cebf97b)
- one thread reply per child result (M6/T4) (#196) (c23a253)
- thread spawn with intake first, no exemptions (M6/T3) (#195) (8b69b49)
- bosun answer counts only owner Slack IDs (M6/T2) (#194) (63b60b3)
- bosun notify with the channel check (M6/T1) (#193) (36ec948)
- agreement harness and review-ci --dry-run (M8/T3) (#184) (d2d7565)
- bosun review-ci, review mode (b) with no session (M8/T1a) (#182) (28f12e0)
- delta since the newest approved head of this PR (M8/T2) (#180) (cdf576b)
- bosun main-health opens and closes main-red, opens a lesson (M3 T4) (#155) (0b49fcf)
- bosun intake (M2/T4) (#165) (377f31b)
- splice channel with pieces core, rust and ts-pnpm (1/2) (#107) (d3e50a2)
- ts-pnpm profile with the pnpm-workspace packages adapter (M7 T3 1b) (#122) (ee753f9)
- generic regex adapters, reduced integrity in status, mixed stacks (UC-M7-3, UC-M7-5) (#168) (d90fa30)
- --remote runs hoist check for steps 3 and 5 (#118) (cb277bc)
- work beside a hull template (own_ci, claude-md, excludes, hoist checks) (#157) (a69bb22)
- reopen mechanized lessons as blocking, --upstream, progress --lessons (M5/T4) (#166) (747a579)
- ts-skip and ts assertion adapters for test-integrity (M7 T3 2/2) (#123) (9e931d2)
- runner keys {packages} and requires; packages adapter dispatch (M7 T3 1a) (#167) (79ab011)
- required checks follow hoist.toml; doctor finds ci-ok in any workflow (#158) (8285d9f)
- bosun plan amend (M2/T3) (#163) (0627b7d)
- bosun revert opens a mainfix PR (M3 T3) (#111) (f7a126d)
- lesson triage and the curator prompt (M5/T3) (#154) (80a4c75)
- bosun init adds the splice channel and opens the setup PR (#112) (701bf2b)
- ready after the plan task merged, UC stubs, list-lag guard (M2/T2b) (#153) (b98a709)
- critical path and dependency cycles (M2/T1b) (#151) (b8bae91)
- janitor prompt runs bosun budget check (M1 janitor, UC-M4-2) (#174) (0bd9015)
- bosun main-triage bisects merged PRs (M3 T2) (#108) (5c50abd)
- bosun result for child results (M1 T4) (#116) (b2748d5)
- a task over per_task_usd opens a budget lesson (UC-M4-3) (#171) (adb980d)
- bosun budget check stops the live sessions at 100 % (UC-M4-2) (#169) (d50dc05)
- fall back to the CCR auto-merge route when GraphQL is refused (#164) (33f2a1f)
- a stop rule opens or hits a lesson (M1 T2b) (#149) (35082e9)
- bosun pr sets building, pr and reap set merged (M1 T1) (#115) (6747ffc)
- cost per merged task over the last N days (UC-M4-4) (#127) (1b3fe20)
- spend guard at 80 % of the daily budget (UC-M4-1) (#126) (569c846)
- bosun plan check and import (M2/T2a) (#124) (7817c6c)
- bosun gate main-open (M3 T1) (#117) (81c8824)
- plan-format parser, rules 1-5, 7, 8 (M2/T1a) (#113) (274b418)
- skip a stack whose manifest does not exist yet (#110) (f3f61ba)
- bosun resume after a container restart (M1 T3) (#105) (c0462f6)
- process-red pauses claim, spawn and waves (M5/T2) (#102) (012629d)
- capture lessons by failure signature (M5/T1) (#103) (6b7bf7e)
- bosun fail — stop rules and tier escalation (M1 T2) (#104) (7b3a42a)

### Fixes

- a file with changed lines and no patch is unseen, whatever its name (#181) (3d3d87e)
- the label-lock reads label fields, not prose (#162) (7e5efbb)
- wave plan and status read the limits from the default branch (#170) (8dbcc5d)
- COMMANDS holds 26 entries (plan and progress both merged) (#152) (ca13968)
- follow next-page links by owner/repo, not repository id (#120) (6a6b556)
- merge-lock reads the branch where git acts (#101) (f182f14)

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
