# Front-loaded human prerequisites (mobile-first owner)

Owner requirement (2026-10-27): "it must work primarily on mobile. So if a human starts a new
request, and there are any tasks to do/prepare that the agent cannot do, this must be handled in
the very beginning, so the agent can then finish the work without any dependency to a human."

Goal: when work enters (`bosun intake`, `bosun plan import`/`amend`, an orchestrator starting a
plan), every human-only step is found and asked for up front as one short, phone-friendly
checklist. Once the owner has done it, agents run to the end; the only later human touch is the
approvals the risk model requires, and those come in batches answerable in chat ("approve 12 15").

Decisions were taken without asking the owner (as instructed); each lists its reason.

## What exists (and is reused)

- `bosun doctor`: the setup checklist (spec §11) as items `{step, name, status, detail, fix}`.
- Risk rules (`.bosun/risk.toml`), approval classes `owner | delegate | two-verdicts`, the
  hard-coded critical floor (`risk::CRITICAL_PATHS`, `APPROVAL_CODE_PATHS`, `CRITICAL_RULES`).
- Signed chat approvals and `bosun approve --list` (ready PRs, `approve_command`).
- `bosun ask` (exit 5, needs-human) for questions that come up during a task.
- `bosun plan import`/`amend`: `promote` moves tasks `planned -> ready` once the plan task merged.
- `core::glob::Glob::overlaps` (glob against glob).

## Design

**F1. Prerequisites live in the plan file.** A `## Prerequisites` section (its heading is prose
under plan-format rule 2, so the plan parser and older bosun versions ignore it):

```markdown
## Prerequisites

- H1: npm token for the release environment
  on: phone
  steps: npmjs.com > Access Tokens > Generate (granular, publish, @keelstackhq); GitHub app > repo > Settings > Environments > release > Add secret NPM_TOKEN
  verify: secret release/NPM_TOKEN
  blocks: M4/T2, M5
```

Fields: `on:` (required) `phone` or `computer`; `steps:` (required) one line, copy-paste ready;
`verify:` (default `owner`); `blocks:` (required) tasks `<item>/<task>` or whole items; `who:`
(default `owner`). IDs match `^H[1-9][0-9]{0,2}$` (H1 to H999). Why in the plan and why `blocks:` on the
prerequisite instead of `needs:` on the task: one place lists all the owner must do (the checklist
is a view of it), it is reviewed in the kind/plan PR like the rest of the plan, and
`src/core/plan.rs` (edited by the parallel-conflicts work) stays untouched.

**F2. Verification.** `verify:` is one of `owner` (the owner ticks the box in the checklist
issue), `secret [ENV/]NAME`, `variable [ENV/]NAME`, `environment NAME`, `file PATH` (on the
default branch). bosun reads the API; a 403 (the session's token cannot read it) falls back to the
tick. A verified item is ticked by bosun; an API-verifiable item is never done on a tick alone
when the API says it is missing. Why: "done" should be a fact bosun can read, not a promise.

**F3. Automatic items.**
- Setup: every `missing` step of `bosun doctor` becomes an item (`D<step>`), so a fresh repo's
  owner steps come first (skip with `--no-setup`).
- Approvals (an estimate from paths): each task whose `touch` overlaps the critical floor or an owner-class rule, or that
  says `risk: high`, is listed under "approvals at merge" with `needs: owner`; one that hits only
  delegate rules is `needs: delegate`. They are grouped into batches by dependency level, so an
  orchestrator spawns a batch together and the owner answers once per batch. It is an estimate:
  risk-check also matches added lines (R-secrets), new dependencies (R-deps), capabilities and
  an unseen diff, which a plan cannot predict, and the bosun-code floor applies only where the
  base has bosun's own code (the `bosun_code` argument). risk-check stays the judge at merge.
- Delegate key: when some task is delegate-class and `.bosun/approvers.toml` has no delegate key,
  an item "enrol a delegate key" (computer) is added. Why: the owner prefers to delegate the
  non-critical classes; without a key every such PR would wait for them.

**F4. `bosun prereqs <plan>... [--post] [--no-setup]`.** Prints `data.items` (sorted: open
computer items first, then open phone items, then done), `data.approvals`, `data.open`,
`data.message` (the checklist as at most ~25 short lines, ready to relay to the owner's chat) and,
with `--post`, upserts one issue per plan (marker `<!-- bosun:prereqs <plan path> -->`, title
`Owner checklist: <plan title>`) whose body is the checklist with task-list boxes the owner ticks
in the GitHub app; ticks are read back before each rewrite, verified items are ticked by bosun,
and the issue closes when nothing is open. Why one issue: GitHub mobile notifies on it, boxes are
tappable on a phone, and the orchestrator relays one link. The issue carries the label
`bosun:prereqs`, and each line's hidden marker names what it blocks
(`<!-- bosun:prereq H1 blocks=M4/T2,M5 -->`), so the gates below read one issue, not the API.
A tick is not authenticated: anyone with write access can tick a box. That is accepted because a
prerequisite only schedules work; risk-check still decides every merge, API-verifiable items never
pass on a tick alone, and agents never tick boxes (AGENTS.md).

**F5. Gating, in two places.** (1) `bosun plan import`/`amend` keeps a task `planned` while an
open prerequisite blocks it (or its item) and lists it under `data.held`; the next import after
the prerequisite is done promotes it. (2) `bosun claim` refuses (exit 5) a task that an open item
of a `bosun:prereqs` issue blocks, and `bosun wave plan` leaves it out, because tasks also become
ready outside promote (`bosun release --to ready`, reap) and a prerequisite can appear after a
task was promoted. Everything else proceeds.

**F6. No mid-run asks.** An agent that hits an undeclared human step does not stop and wait. It
runs `bosun prereqs hold <issue> "<what the owner must do>" [--phone]`: bosun adds an item
`X<n>` (blocks: that task) to the plan's checklist issue, releases the task to `ready` (the claim
gate now keeps it there) and records a lesson. The agent takes other work. The orchestrator
relays the updated checklist once and folds the item into the plan's `## Prerequisites` in the
next amendment. `bosun ask` remains for decisions, which also go to the orchestrator's batch.

**F7. Approval batches.** `bosun approve --list` adds `data.message`: one short text listing the
ready PRs (`#12@abc1234`, task, title) and the one reply (`approve 12 15 17`). The orchestrator
maps the numbers in the reply to the `<pr>@<head>` pairs of that same listing and signs each with
`--sign-key` (a bare number stays exit 2). If a head moved after the message, signing it is exit
3 and that PR goes into the next message. The orchestrator relays a message when a batch from F3
is complete (or at most every few hours), not per PR.

**F8. Docs.** A spec section (after the spec split lands), `docs/prerequisites.md`, AGENTS.md
rules for agents and orchestrators ("front-load human steps; never block mid-run"), the keelstack
playbook/Start here text, and decision D-009 in keelstackhq/decisions.

## Out of scope

Running hoist from bosun (doctor already does); free-form commands as verifiers (a plan file is
agent-written, bosun must not execute it); notifications beyond the issue and the relayed text.
