# Plan: clean template updates (B5)

project: bosun

Design: spec §17 (`docs/spec/17-template-updates.md`, proposed, not built). A weekly splice
update PR whose lock diff only moves an allowlisted source's pin, whose content equals a fresh
render at base, and whose author is the splice bot App, gets the `template` approval class: the
bot plus green gates, no owner, for the paths in `[template_update] paths` only. The template
floor (the §8 critical floor, every R-ci path, `.claude/**`, `REVIEW.md`, `docs/lessons.toml`,
the trust files, `hoist.toml`, `docs/setup-script.sh`) still needs the owner, and so does a
whole PR that touches any path outside `paths`.

Status: **blocked on the owner and on §15.** No task starts before the decisions in §17 "Owner
decision needed" are made (H1) and §15 (App check runs, B2) is built (H3); the answers may cut
or change tasks. This plan merges after the B2/B4 design PR (§15, §16), and its tasks touch
`risk.yml` and risk-check only after §15's tasks have landed there. Every task changes
risk-check, the gate code, or a workflow, so each one is `risk: high` and needs the owner.

Seed constraints hold: one task per PR of at most 400 changed lines, `src/core` does no I/O,
tests never reach a real API (the fake GitHub in `tests/common` gets the new routes), one
envelope on stdout, `error.fix` never empty, fragments instead of CHANGELOG/PROGRESS edits.
Each task updates `docs/spec/17-template-updates.md` for what it builds and moves its part from
"proposed" to "built".

## Prerequisites

- H1: decide the open choices of spec §17 (1-8)
  on: phone
  steps: read docs/spec/17-template-updates.md "Owner decision needed" and answer each item in the B5 PR or to the orchestrator
  blocks: B5
- H2: a splice bot GitHub App with Contents and Pull requests write on this repo (plus Workflows write in a repo that uses the bosun channel, to push rendered workflow files, which stay in the floor); its key in the environment splice-bot with a main-only branch policy
  on: computer
  steps: GitHub > Settings > Developer settings > GitHub Apps > New; install on keelstackhq/bosun; repo > Settings > Environments > New splice-bot, deployment branches main only, add the App id and key
  blocks: B5
- H3: §15 App check runs (B2) built and merged, the gate App installed
  on: computer
  steps: see docs/spec/15-app-checks.md and its plan
  blocks: B5

## B5: Clean template updates

area: risk
depends: seed

Lower approval class for clean splice template updates (spec §17, C1-C7).

### Tasks

- T1: `[template_update]` config (`sources`, `bot`, `paths`, `splice`, `splice_sha256`), parsed from base; floor paths in `paths` never lower, and one changed path outside `paths` makes the whole PR owner-only
  touch: src/core/risk.rs, src/core/template_update.rs, src/core/mod.rs, tests/risk_template_config.rs, docs/spec/17-template-updates.md
  risk: high
  size: 200
- T2: lock diff (C1) and declared files (C6, deletions never clean) as pure functions over two TOML texts and a file list
  touch: src/core/template_update.rs, tests/core_template_lock.rs
  depends: T1
  risk: high
  size: 300
- T3: GitHub reads for C3, C5 and C7 (mergeable, author type, commit verification, compare `{track}...{commit}`, releases or tags) and the gate App's `template-render` check runs, with fake routes
  touch: src/github/mod.rs, src/github/types.rs, tests/common/**, tests/github_template.rs
  depends: T1
  risk: high
  size: 300
- T4: risk-check applies the `template` class (status text, `data.template_update`, floor, fail closed), compares `BOSUN_TEMPLATE_TREE` with the head tree (C2), posts the per-head `template-render` check run and reads it on later runs by App id
  touch: src/cli/risk_check.rs, tests/cli_risk_template.rs, docs/spec/17-template-updates.md
  depends: T2, T3
  risk: high
  size: 350
- T5: the `template` render job in `risk.yml` (bot login and `user.type`, 40-hex commit check, C7 before any fetch, base checkout, pinned splice with sha256, `contents: read`, no credentials or key; `risk` job `needs` it with `if: always()`) and its channel copy
  touch: .github/workflows/risk.yml, channel/pieces/core/files/.github/workflows/risk.yml, tests/workflows.rs, docs/spec/17-template-updates.md
  depends: T4
  risk: high
  size: 150
- T6: splice.yml takes the bot App token from the `splice-bot` environment, and its channel copy
  touch: .github/workflows/splice.yml, channel/pieces/core/files/.github/workflows/splice.yml, tests/workflow_splice_bot.rs
  risk: high
  size: 80

### Done when

- UC-B5-1: a bot PR that only moves an allowlisted source's version and commit, whose head tree equals the render at base and whose changed files are all in `paths`, passes risk-check with the `template` class and no owner.
- UC-B5-2: the same PR with one hand-edited rendered line, a changed `template.source`, a fork commit that is not an ancestor of `track`, a deleted file, a non-bot merge commit or a PAT author fails closed to the owner class with a `reason`.
- UC-B5-3: a clean update that also changes a file under `.github/workflows/**`, `.bosun/**` or `.claude/**`, `REVIEW.md`, `hoist.toml`, `docs/setup-script.sh` or any other path outside `paths` (also one that matches no rule) lowers nothing: the whole PR needs the owner.
- UC-B5-4: any failure of the render job (no tree SHA, checksum mismatch, timeout, a commit that is not 40 hex) leaves the PR in its §8 class and never posts an `error` status.
- UC-B5-5: a later risk-check on the same head (the verdict.yml re-run) keeps the class through the gate App's `template-render` check run, and ignores a check run of another app or a commit status of that name.
- UC-B5-6: the render job checks C7 through the API before it fetches or renders anything (a fork commit is never fetched), checks out the base SHA, holds `contents: read` only, persists no credentials, and the `risk` job runs when it is skipped (`tests/workflows.rs`).

## Notes

- §15 (App check runs, B2) is a hard prerequisite: without it the verdict.yml re-run has no
  unforgeable render record and the class flips to "owner needed". The bot App is never the
  gate App (§15 and §17 keep them separate).
- No carry-over: a merge of main by anyone but the bot fails C5, so the bot refreshes the PR.
- Splice dependency: C2 needs a splice mode that renders a given lock at a given commit and runs
  no piece code, and C5 needs the bot's commits made through the API. If splice 0.2.0 lacks
  either, that is a splice PR first (keelstackhq/splice).
- Cost: about one billed minute per bot-PR push (`docs/ci-minutes.md`); skipped for other PRs.
