# Plan: App check runs and owner teams

project: bosun

Design: spec §15 (B2, the gates as check runs of a dedicated App, pinned in the ruleset) and §16
(B4, owner teams; §16, B4: next PR). Seed constraints hold: one task per PR of at most 400 changed lines, `src/core`
does no I/O, tests never reach a real API, fragments instead of CHANGELOG/PROGRESS edits.
B2 lands first. B4 code starts only once B2's App token reaches the workflows (B2W/T1).
B2W builds on B1 (`fix/risk-no-review-event`): risk.yml has no `pull_request_review` trigger,
so no job with the environment runs the PR's copy; reviews are judged on the next run.
Owner order: H1-H4, H6, H7, then H5.
`.bosun/bosun.toml` `[checks]` and an owners.toml `team` entry land only after the pin is a release
that knows them, since unknown keys are errors.

## B2: Check runs from the App

area: risk
depends: seed

The check-run client, the `[checks]` mode, the gate writer with dual posting, and reading the
gates (and the verdict records) back by App (§15, "bosun changes").

### Tasks

- T1: Check-run client in src/github
  touch: src/github/mod.rs, src/github/types.rs, tests/common/fake.rs, tests/github_check_runs.rs
  size: 250
  risk: high
- T2: `[checks]` config and the hoist.toml table form of required_checks
  touch: src/config.rs, src/core/checks.rs, tests/config_checks.rs
  size: 200
  risk: high
- T3: Gate writer with statuses, dual and checks modes
  touch: src/github/gate.rs, src/cli/risk_check.rs, src/cli/verdict.rs, src/gates/main_open.rs, src/cli/review_ci.rs, tests/cli_checks_post.rs, docs/spec/15-app-checks.md
  depends: T1, T2
  size: 350
  risk: high
- T4: Read gates and verdict records back by App
  touch: src/core/approve.rs, src/core/risk.rs, src/cli/carry_over.rs, src/cli/review_pack.rs, src/cli/pr.rs, src/cli/approve.rs, src/cli/verdict.rs, tests/cli_checks_read.rs
  depends: T3
  size: 350
  risk: high
- T5: Refusals in checks mode and the doctor pin check
  touch: src/cli/verdict.rs, src/cli/approve.rs, src/cli/doctor.rs, tests/cli_checks_refuse.rs
  depends: T4
  size: 200
  risk: high

### Done when

- UC-B2-1: in checks mode risk-check creates a `risk-check` check run (name, head_sha, conclusion, summary) with the App token and posts no status.
- UC-B2-2: in dual mode risk-check, verdict apply and gate main-open post a status and a check run with the same verdict.
- UC-B2-3: in checks mode a same-name check run from github-actions, or a status, never counts for `bosun approve --list` or `bosun pr --auto-merge`.
- UC-B2-4: in checks mode a verdict record not written by `<app_slug>[bot]` counts for no verdict class.
- UC-B2-5: checks mode without BOSUN_APP_TOKEN is exit 1 with a fix that names the bosun-app environment.
- UC-B2-6: no conclusion other than success or failure is ever posted for a gate.
- UC-B2-7: no comment written with the App token contains `<!-- bosun:verdict -->`, `/bosun approve` or `/bosun recheck`.

## B2W: Workflows on the App token

area: risk
depends: B2

The workflows, then the channel copies, take the `bosun-app` environment and the token step;
then the ruleset pins the sources and the statuses stop (§15, Transition).

### Tasks

- T1: risk, verdict, main-open and main-health jobs on the App token, dual mode
  touch: .github/workflows/risk.yml, .github/workflows/verdict.yml, .github/workflows/main-open.yml, .github/workflows/main-health.yml, .bosun/bosun.toml, tests/workflows.rs
  size: 200
  risk: high
- T2: Channel copies behind a template var
  touch: channel/pieces/core/files/.github/workflows/**, channel/pieces/core/splice.toml, tests/channel.rs
  depends: T1
  size: 300
  risk: high
- T3: hoist.toml pins each required check to its source
  touch: hoist.toml, tests/governance.rs
  depends: T1
  size: 20
  risk: high
- T4: Switch to checks mode and retire D32
  touch: .bosun/bosun.toml, docs/spec/01-summary.md, docs/spec/07-gates.md, docs/spec/08-risk.md, docs/spec/15-app-checks.md
  depends: T3
  size: 60
  risk: high

- T5: Optional, the owner's choice: re-judge on review through a no-permission relay
  touch: .github/workflows/review-relay.yml, .github/workflows/risk.yml, tests/workflows.rs
  depends: T1
  size: 120
  risk: high

### Done when

- UC-B2W-1: no workflow with the bosun-app environment has a pull_request or pull_request_review trigger; every such job runs on pull_request_target, issue_comment, workflow_run or push to main, never checks out the PR head, and passes the key only to the token step.
- UC-B2W-2: no job that runs repo code (ci, main-health's test job) names the environment or asks for checks or statuses write.
- UC-B2W-3: hoist.toml pins risk-check, review-verdict and main-open to the App and ci-ok to GitHub Actions.
- UC-B2W-4: every keyed workflow_run job filters as main-health does: `branches: [main]`, `event == 'push'`, the run's `head_repository` is this repo, and it checks out `github.sha`.

## B4: Owner teams

area: risk
depends: B2

`[[owner]] team = "org/slug"`, expanded on the App token, cached per run, failing closed (§16).

### Tasks

- T1: owners.toml team entries
  touch: src/core/risk.rs, tests/owners_parse.rs
  depends: B2W/T1
  size: 120
  risk: high
- T2: Team expansion with the per-run cache, failing closed
  touch: src/github/mod.rs, src/github/types.rs, src/cli/risk_check.rs, tests/common/fake.rs, tests/cli_owner_teams.rs, docs/spec/16-owner-teams.md
  depends: T1
  size: 350
  risk: high
- T3: Kill switch, approve, ask and notify with teams
  touch: src/app.rs, src/cli/approve.rs, src/cli/ask.rs, src/cli/notify.rs, tests/cli_owner_teams_cmds.rs
  depends: T2
  size: 250
  risk: high
- T4: Bus-factor note and a team example in the shipped owners files
  touch: governance/owners.toml, channel/pieces/core/files/governance/**, docs/spec/03-layout.md
  depends: T3
  kind: bookkeeping
  size: 30
  risk: high

### Done when

- UC-B4-1: a team member's APPROVED review or `/bosun approve` comment on the head approves a high PR, and a non-member's does not.
- UC-B4-2: a 403 or 404 on the team list counts no member, lists the team in `data.owner_teams_unreadable`, and the fix names Members read.
- UC-B4-3: a two-page team list is read once per run although several owner checks need it.
- UC-B4-4: a halt issue by a team member halts, and with the team unreadable a halt issue by a non-user halts too.

## Outside this repo

- hoist: request (an issue in keelstackhq/hoist) required-check source pinning, as in §15: table
  entries `{ context, app }` or `{ context, integration_id }` in `[github] required_checks`,
  `app` resolved with `GET /apps/{slug}`, written as the ruleset's `integration_id`, and drift
  in `hoist check`. B2W/T3 waits for a hoist release with it, pinned in the setup script.

## Prerequisites

- H1: Create the bosun gate GitHub App
  on: computer
  steps: Org settings > Developer settings > GitHub Apps > New: name keelstackhq-bosun-gate, Webhook Active off, repo permissions Checks rw, Pull requests rw, Contents r, Issues r, Commit statuses none; org permission Members r; Only on this account; install on keelstackhq/bosun only
  blocks: B2W, B4
- H2: Generate the App's private key
  on: computer
  steps: App settings > Private keys > Generate; keep the .pem only until H4, then delete it
  blocks: B2W
- H3: Create the bosun-app environment, main only
  on: computer
  steps: Repo Settings > Environments > New bosun-app; Deployment branches and tags > Selected > add branch rule main; no tags, no reviewers
  verify: environment bosun-app
  blocks: B2W
- H4: Add the App id and key to the environment
  on: computer
  steps: gh variable set BOSUN_APP_ID --env bosun-app --body <app id>; gh secret set BOSUN_APP_KEY --env bosun-app < key.pem; never as a repo or org secret
  verify: secret bosun-app/BOSUN_APP_KEY
  blocks: B2W
- H5: Switch the ruleset to the pinned sources
  on: computer
  steps: after B2W/T3 merges and dual runs show the App's check runs, from a checkout of main with an admin token: hoist plan --area github, then hoist apply --area github --yes
  blocks: B2W/T4
- H6: Release B2 and pin it
  on: phone
  steps: run the release workflow for a version with B2 (docs/releasing.md), then approve the pin PR that moves .bosun/version to it
  blocks: B2W
- H7: Check the agents' GitHub credential
  on: computer
  steps: the Claude GitHub App or token the agents use has no Administration, Environments or Secrets write on the repo; remove any it has
  blocks: B2W
