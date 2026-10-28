# Plan: clean template updates (B5)

project: bosun

Design: spec §15 (`docs/spec/15-template-updates.md`, proposed, not built). A weekly splice
update PR whose lock diff only moves an allowlisted source's pin, whose content equals a fresh
render at base, and whose author is the splice bot App, gets the `template` approval class: the
bot plus green gates, no owner. The template floor (the §8 critical floor, every R-ci path and
`.claude/**`) still needs the owner.

Status: **blocked on the owner.** No task starts before the decisions in §15 "Owner decision
needed" are made (H1 below); the answers may cut or change tasks. Every task changes risk-check,
the gate code, or a workflow, so each one is `risk: high` and needs the owner's approval.

Seed constraints hold: one task per PR of at most 400 changed lines, `src/core` does no I/O,
tests never reach a real API (the fake GitHub in `tests/common` gets the new routes), one
envelope on stdout, `error.fix` never empty, fragments instead of CHANGELOG/PROGRESS edits.
Each task updates `docs/spec/15-template-updates.md` for what it builds and moves its part from
"proposed" to "built".

## Prerequisites

- H1: decide the open choices of spec §15 (1-8)
  on: phone
  steps: read docs/spec/15-template-updates.md "Owner decision needed" and answer each item in the B5 PR or to the orchestrator
  blocks: B5
- H2: a splice bot GitHub App with Contents and Pull requests write on this repo; its installation token in SPLICE_BOT_TOKEN
  on: computer
  steps: GitHub > Settings > Developer settings > GitHub Apps > New; install on keelstackhq/bosun; put the App id and key in the splice workflow's secrets
  blocks: B5/T3, B5/T5
- H3: B2, risk-check posted by a dedicated GitHub App as a check run (PROGRESS Blocked item 5), if decision 5 says B2 first
  on: computer
  steps: see docs/PROGRESS.md Blocked item 5
  blocks: B5/T5

## B5: Clean template updates

area: risk
depends: seed

Lower approval class for clean splice template updates (spec §15, C1-C7).

### Tasks

- T1: `[template_update]` config and the `template_update` rule flag, parsed from base
  touch: src/core/risk.rs, src/core/template_update.rs, src/core/mod.rs, tests/risk_template_config.rs, docs/spec/15-template-updates.md
  risk: high
  size: 200
- T2: lock diff (C1) and declared files (C6) as pure functions over two TOML texts and a file list
  touch: src/core/template_update.rs, tests/core_template_lock.rs
  depends: T1
  risk: high
  size: 300
- T3: GitHub reads for C3, C5 and C7 (mergeable, PR author type, commit verification, compare against the source's default branch, releases or tags) and their fake routes
  touch: src/github/mod.rs, src/github/types.rs, tests/common/**, tests/github_template.rs
  depends: T1
  risk: high
  size: 300
- T4: risk-check applies the `template` class (status text, `data.template_update`, floor, fail closed) and compares `BOSUN_TEMPLATE_TREE` with the head tree (C2)
  touch: src/cli/risk_check.rs, tests/cli_risk_template.rs, docs/spec/15-template-updates.md
  depends: T2, T3
  risk: high
  size: 350
- T5: the `template` render job in `risk.yml` (base checkout, pinned splice with sha256, `contents: read`, no credentials) and its workflow test; the channel copy for consumers after B2 if decision 5 says so
  touch: .github/workflows/risk.yml, channel/pieces/core/files/.github/workflows/risk.yml, tests/workflows.rs, docs/spec/15-template-updates.md
  depends: T4
  risk: high
  size: 150
- T6: approval carry-over carries the `template` class across clean merges of main (§8 rules 1-5)
  touch: src/cli/carry_over.rs, tests/cli_carry_template.rs, docs/spec/08-risk.md
  depends: T4
  risk: high
  size: 200

### Done when

- UC-B5-1: a bot PR that only moves an allowlisted source's version and commit, whose head tree equals the render at base, passes risk-check with the `template` class and no owner.
- UC-B5-2: the same PR with one hand-edited rendered line, a changed `template.source`, a fork commit that is not on the source's default branch, or a PAT author fails closed to the owner class with a `reason`.
- UC-B5-3: a clean update that changes a file under `.github/workflows/**`, `.bosun/**` or `.claude/**` still needs the owner.
- UC-B5-4: any failure of the render job (no tree SHA, checksum mismatch, timeout) leaves the PR in its §8 class and never posts an `error` status.
- UC-B5-5: the render job checks out the base SHA, holds `contents: read` only and persists no credentials (`tests/workflows.rs`).

## Notes

- B2 dependency: T5 can hand the tree SHA over as a job output now; with B2 the render result
  becomes a check run of the judging App, which the bot App must never be (spec §15).
- Splice dependency: C2 needs a splice mode that renders a given lock at a given commit and runs
  no piece code. If splice 0.2.0 lacks it, that is a splice PR first (keelstackhq/splice).
- Cost: about one billed minute per bot-PR push (`docs/ci-minutes.md`); skipped for other PRs.
