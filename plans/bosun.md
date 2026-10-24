# Plan: bosun

project: bosun

The milestones after the seed (spec §12). Each item is one epic and one area; `bosun plan import`
(M2) turns this file into issues. Until M2 lands, the root session works the items by hand with
the same commands.

## M1: Lifecycle

area: lifecycle
depends: seed

The state machine with the label-lock hook, stop rules and tier escalation, resume, and the
janitor.

### Tasks

- T1: State changes only through bosun commands, end to end
  touch: src/app.rs, src/cli/claim.rs, src/cli/pr.rs, src/cli/release.rs, tests/cli_lifecycle.rs
  size: 300
- T2: Stop rules (same failure 3 times, empty fix round) and tier escalation
  touch: src/core/stop.rs, src/cli/stop.rs, tests/cli_stop.rs
  size: 350
- T3: bosun resume after a container restart
  touch: src/cli/resume.rs, tests/cli_resume.rs
  depends: T1
  size: 250
- T4: bosun result for child results, and the janitor prompt
  touch: src/cli/result.rs, src/cli/doctor.rs, tests/cli_result.rs
  size: 200
- T5: Reap smoke test with a real clock jump
  touch: tests/smoke_reap.rs
  depends: T1
  size: 120

### Done when

- UC-M1-1: a task moves planned -> merged through bosun commands and auto-merge only, and a direct label edit is refused.
- UC-M1-2: the same failure 3 times sets blocked and escalates one tier.
- UC-M1-3: a lease that is not renewed for 30 min is reaped and the task is ready again (smoke test).
- UC-M1-4: after a container restart, bosun resume continues the open PR of the session.

## M2: Plan import

area: plan
depends: M1

The plan-format parser, import, amend and intake.

### Tasks

- T1: Plan-format parser with line-numbered errors
  touch: src/plan/**, tests/plan_parse.rs
  size: 380
- T2: bosun plan import (epics, tasks), idempotent by marker
  touch: src/cli/plan.rs, tests/cli_plan_import.rs
  depends: T1
  size: 380
- T3: bosun plan amend; a removed item closes its epic as not planned
  touch: src/cli/plan_amend.rs, tests/cli_plan_amend.rs
  depends: T2
  size: 250
- T4: bosun intake writes a kind/plan task
  touch: src/cli/intake.rs, tests/cli_intake.rs
  depends: T2
  size: 250

### Done when

- UC-M2-1: importing the same plan twice creates no duplicate issues.
- UC-M2-2: a task becomes ready only after its plan task merged.
- UC-M2-3: each "Done when" line is listed in its epic, and import writes no test files.
- UC-M2-4: an amendment that removes an item closes its epic as not planned.

## M3: Main-health

area: health
depends: M2

main-open and main-red in bosun, triage by bisect, revert, and the merge queue.

### Tasks

- T1: bosun gate main-open replaces the shell step
  touch: src/gates/main_open.rs, .github/workflows/main-open.yml, tests/cli_main_open.rs
  risk: high
  size: 150
- T2: bosun main-triage bisects merged PRs
  touch: src/cli/main_triage.rs, src/core/bisect.rs, tests/cli_main_triage.rs
  size: 350
- T3: bosun revert opens a mainfix PR
  touch: src/cli/revert.rs, tests/cli_revert.rs
  size: 250
- T4: main-red opens a lesson
  touch: src/cli/main_health.rs, tests/cli_main_health.rs
  size: 200

### Done when

- UC-M3-1: a red push to main opens one main-red issue, and green closes it.
- UC-M3-2: while main is red, a feature PR fails main-open and a mainfix PR passes.
- UC-M3-3: main-triage names the first bad PR in a fake history of 8 merges.
- UC-M3-4: bosun revert <pr> opens a mainfix PR that turns main green.

## M4: Cost and spend guard

area: cost
depends: M2

Spend in claim and the janitor, the per-task cap, and the cost view.

### Tasks

- T1: spend guard in claim (spawn has it) with the mainfix exemption
  touch: src/cli/claim.rs, tests/cli_spend_guard.rs
  size: 200
- T2: janitor budget check stops sessions at 100 %
  touch: src/cli/janitor.rs, tests/cli_janitor.rs
  size: 300
- T3: per-task cap opens a lesson
  touch: src/cli/budget.rs, tests/cli_budget.rs
  size: 150
- T4: bosun progress cost view (cost per merged task, 7 days)
  touch: src/cli/progress.rs, tests/cli_progress.rs
  size: 300

### Done when

- UC-M4-1: at 80 % daily spend, spawn and claim refuse with exit 6, except mainfix.
- UC-M4-2: at 100 %, the janitor stops the live sessions.
- UC-M4-3: a task over per_task_usd goes to blocked and opens a lesson.
- UC-M4-4: bosun progress shows the cost per merged task for the last 7 days.

## M5: Learning loop

area: learning
depends: M3, M4

Automatic capture, dedupe, triage, process-red and pruning.

### Tasks

- T1: automatic capture with a failure signature and dedupe
  touch: src/core/signature.rs, src/cli/lesson.rs, tests/cli_lesson_capture.rs
  size: 350
- T2: process-red pauses waves like main-red
  touch: src/app.rs, src/cli/wave.rs, tests/cli_process_red.rs
  size: 200
- T3: curator prompt and triage commands
  touch: src/cli/lesson_triage.rs, tests/cli_lesson_triage.rs
  size: 300
- T4: a hit on a mechanized lesson reopens it as blocking; --upstream; progress --lessons
  touch: src/cli/lesson.rs, src/cli/progress.rs, tests/cli_lesson_prune.rs
  depends: T1
  size: 300

### Done when

- UC-M5-1: a stop rule opens a lesson; the same signature again adds a hit, not a new issue.
- UC-M5-2: a blocking lesson sets process-red, and waves pause.
- UC-M5-3: a pack includes only lessons whose scope matches the task paths, plus global ones.
- UC-M5-4: at 40 entries, bosun lesson add refuses with exit 6.
- UC-M5-5: a hit on a mechanized lesson reopens it as blocking.

## M6: Slack

area: slack
depends: M5

notify, answer, thread spawn and the result reply, against a fake Slack.

### Tasks

- T1: bosun notify with the shared-channel and guest check
  touch: src/cli/notify.rs, src/slack/**, tests/cli_notify.rs
  size: 350
- T2: bosun answer counts only owner Slack IDs
  touch: src/cli/answer.rs, tests/cli_answer.rs
  size: 200
- T3: thread spawn with intake first when no task is ready
  touch: src/cli/spawn.rs, tests/cli_thread_spawn.rs
  size: 300
- T4: exactly one thread reply per child result
  touch: src/cli/notify.rs, tests/cli_thread_reply.rs
  depends: T1
  size: 200

### Done when

- UC-M6-1: notify refuses a shared channel or a channel with guests.
- UC-M6-2: an answer from a non-owner Slack ID does not count.
- UC-M6-3: a thread request with no ready issue runs intake, then a plan task, then spawn.
- UC-M6-4: a child result produces exactly one thread reply, also when the thread ended.

## M7: Reuse

area: reuse
depends: M2, M5

The splice channel and bosun init, the ts-pnpm profile, generic adapters, mixed stacks and
doctor --remote.

### Tasks

- T1: splice channel gh:keelstackhq/bosun//channel with pieces core, rust, ts-pnpm
  touch: channel/**
  risk: high
  size: 380
- T2: bosun init runs splice channel add
  touch: src/cli/init.rs, tests/cli_init.rs
  depends: T1
  size: 250
- T3: ts-pnpm profile and adapters (hoard cache in its workflows)
  touch: profiles/ts-pnpm.toml, src/gates/adapters.rs, tests/cli_gate_ts.rs
  size: 380
- T4: generic regex adapters and reduced integrity; mixed stacks
  touch: src/gates/**, tests/cli_gate_mixed.rs
  depends: T3
  size: 300
- T5: bosun doctor --remote calls hoist check
  touch: src/cli/doctor.rs, tests/cli_doctor_remote.rs
  size: 200

### Done when

- UC-M7-1: bosun init --stack rust on an empty repo gives green CI on the first PR.
- UC-M7-2: bosun init --stack ts-pnpm does the same for a pnpm workspace.
- UC-M7-3: a repo with two [[stack]] entries runs each gate with the right profile.
- UC-M7-4: a new channel version reaches a consumer as one R-gov PR from the splice update bot.
- UC-M7-5: a generic adapter shows "reduced integrity" in bosun status.

## M8: Review mode (b)

area: review
depends: M7

The review verdict as a CI check, delta reviews, and the agreement study.

### Tasks

- T1: review.mode = "ci" runs the verdict in a workflow
  touch: src/cli/review_ci.rs, .github/workflows/review.yml, tests/cli_review_ci.rs
  risk: high
  size: 350
- T2: delta review covers only new commits in CI mode
  touch: src/cli/review_pack.rs, tests/cli_review_delta.rs
  size: 150
- T3: mode (a) vs mode (b) agreement on 20 recorded PRs
  touch: tests/review_agreement.rs, tests/fixtures/reviews/**
  depends: T1
  size: 300

### Done when

- UC-M8-1: with review.mode = "ci", the review-verdict check runs with no session.
- UC-M8-2: a second review covers only the new commits.
- UC-M8-3: on 20 recorded PRs, mode (b) agrees with mode (a) on at least 18.
