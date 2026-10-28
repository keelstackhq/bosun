## 12. Implementation plan

Each milestone is one item of `plans/bosun.md`, so one epic. Tasks are at most 400 lines. "Done
when" lines become UCs. A milestone that ships changes the bosun binary and is released by the
release workflow (`hoist release`); the pin moves with an R-gov PR.

Dependencies: seed -> M1 -> M2 -> M3 -> M4 -> M5 -> M6. M1 -> M7 -> bootstrap 2, and M7 -> M8. M3
and M4 can run in parallel after M2. M7 needs M2 (plan import) and M5 (lessons store). M8 needs M7.

M1 Lifecycle. Tasks: state machine with the label-lock hook; stop rules and tier escalation;
bosun resume; bosun msg; janitor prompt; reap smoke test. Done when:
- UC-M1-1: a task moves planned -> merged through bosun commands and auto-merge only, and a direct
  label edit is refused.
- UC-M1-2: the same failure 3 times sets blocked and escalates one tier.
- UC-M1-3: a lease that is not renewed for 30 min is reaped and the task is ready again (smoke test).
- UC-M1-4: after a container restart, bosun resume continues the open PR of the session.

M2 Plan import. Tasks: plan-format parser; bosun plan import (epics, plan tasks);
bosun plan amend; bosun intake. Done when:
- UC-M2-1: importing the same plan twice creates no duplicate issues.
- UC-M2-2: a task becomes ready only after its plan task merged.
- UC-M2-3: each "Done when" line is listed in its epic, and import writes no test files.
- UC-M2-4: an amendment that removes an item closes its epic as not planned.

M3 Main-health. Tasks: main-health workflow; main-open check; bosun main-triage (bisect); bosun
revert; main-red lesson. Done when:
- UC-M3-1: a red push to main opens one main-red issue, and green closes it.
- UC-M3-2: while main is red, a feature PR fails main-open and a mainfix PR passes.
- UC-M3-3: main-triage names the first bad PR in a fake history of 8 merges.
- UC-M3-4: bosun revert <pr> opens a mainfix PR that turns main green.

M4 Cost and spend guard. Tasks: budget record with both usage providers; spend guard in spawn,
claim and janitor; per-task cap; bosun progress cost view. Done when:
- UC-M4-1: at 80 % daily spend, spawn and claim refuse with exit 6, except mainfix.
- UC-M4-2: at 100 %, the janitor stops the live sessions.
- UC-M4-3: a task over per_task_usd goes to blocked and opens a lesson.
- UC-M4-4: bosun progress shows the cost per merged task for the last 7 days.

M5 Learning loop. Tasks: automatic capture; signature and dedupe; curator Routine and triage;
process-red; scope matching in packs; caps and prune; --upstream; bosun progress --lessons. Done
when:
- UC-M5-1: a stop rule opens a lesson; the same signature again adds a hit, not a new issue.
- UC-M5-2: a blocking lesson sets process-red, and waves pause.
- UC-M5-3: a pack includes only lessons whose scope matches the task paths, plus global ones.
- UC-M5-4: at 40 entries, bosun lesson add refuses with exit 6.
- UC-M5-5: a hit on a mechanized lesson reopens it as blocking.

M6 Slack. Tasks: bosun notify with the channel check; bosun answer with owner IDs; thread spawn;
result reply; tests against a fake Slack. Done when:
- UC-M6-1: notify refuses a shared channel or a channel with guests.
- UC-M6-2: an answer from a non-owner Slack ID does not count.
- UC-M6-3: a thread request with no ready issue runs intake, then a plan task, then spawn.
- UC-M6-4: a child result produces exactly one thread reply, also when the thread ended.

M7 Reuse. Tasks: the splice channel (pieces core, rust, ts-pnpm) and bosun init (D16); ts-pnpm
profile and adapters; generic regex adapters; mixed stacks; bosun doctor --remote with hoist check
(D17). Done when:
- UC-M7-1: bosun init --stack rust on an empty repo gives green CI on the first PR.
- UC-M7-2: bosun init --stack ts-pnpm does the same for a pnpm workspace.
- UC-M7-3: a repo with two [[stack]] entries runs each gate with the right profile.
- UC-M7-4: a new channel version reaches a consumer as one R-gov PR from the splice update bot.
- UC-M7-5: a generic adapter shows "reduced integrity" in bosun status.

`bosun init --stack <rust|ts-pnpm>... [--plan FILE] [--owner LOGIN] [--source SRC] [--ref REF] [--bosun-version X.Y.Z] [--branch NAME] [--no-pr]` runs `splice channel add` for the bosun channel on branch `bosun/init` (`claude/bosun-init` when `CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE` starts with `cloud`, since cloud sessions push only to `claude/` branches; `--branch` overrides, checked with `git check-ref-format`) (ref `v<bosun version>` by default; a non-release ref also passes `bosun_version`), keeps an existing `.claude/settings.json` or `splice.yml` (`--exclude`), commits, pushes and opens one R-gov setup PR whose body lists the owner steps. It skips the kill switch (a fresh repo has no labels); `--no-pr` stops after the commit, needs no token and returns `pr_title` and `pr_body` for a hand-opened PR. Beside a hull template it passes `own_ci=1` when a workflow already reports the check `ci-ok` (a job named `ci-ok`, or keyed so without a `name:`; `core::checks::workflow_checks`), adds the `claude-md` piece unless a splice lock lists `CLAUDE.md`, excludes every existing seed path and `.claude/settings.json` (if that file lacks the bosun hooks, an owner step says to take them from the template that owns it, not to edit it), and appends the missing required checks to an existing `hoist.toml` (`bosun-ok` too with `own_ci`), checked before splice runs and refused when the edit cannot be made safely. In the same text edit it sets `[github] merge_queue = false` (after `required_checks`) when the key is absent, since bosun posts risk-check and review-verdict on each PR head and never on merge-queue (`merge_group`) refs, so a queue would block every PR; an explicit value is the owner's and stays, and `true` adds a line to `data.warnings` and the PR body. `data.hoist_merge_queue` is `added`, `kept` or null (no hoist.toml); a second run changes nothing.

M8 Review mode (b). Tasks: review as a CI check; delta review; comparison of mode (a) and mode (b)
verdicts on the same PRs. Done when:
- UC-M8-1: with review.mode = "ci", the review-verdict check runs with no session.
- UC-M8-2: a second review covers only the new commits.
- UC-M8-3: on 20 recorded PRs, mode (b) agrees with mode (a) on at least 18.

Bootstrap 2 (the runtime repo, SP2) starts after the M7 release:
1. Do checklist steps 1-7 for the runtime repo, checked with `bosun doctor --remote`.
2. Run `bosun init --stack rust --plan runtime-plan.md`, which opens one R-gov setup PR.
3. The owner reviews the setup PR (it merges through auto-merge), creates the Routines, and
   approves the plan PR.
4. The root session runs the same loop as in bosun: import, the plan-task wave, one session per
   area, coordinate.
