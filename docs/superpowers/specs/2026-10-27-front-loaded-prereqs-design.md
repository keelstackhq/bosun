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
- Approvals (an estimate from paths, as risk-check would judge them): a task is listed under
  "approvals at merge" with `needs: owner` when its `touch` overlaps an owner-class rule, a
  delegate rule together with a two-verdicts rule, or a delegate rule on the critical floor (the
  floor only turns delegate matches into owner ones, as `risk::apply_floor` does), or when it says
  `risk: high`; with `needs: delegate` when it hits only delegate rules. A task that matches no
  rule is normal risk and not listed, even on the floor. A rule in `risk::CRITICAL_RULES` that matches
by path needs the owner. A touch glob bosun cannot parse, or one that leaves the repo (`..`),
counts as owner (fail closed). They are grouped into batches by
  dependency level, so an orchestrator spawns a batch together and the owner answers once per
  batch. It is an estimate: risk-check also matches added lines (R-secrets), new dependencies
  (R-deps), capabilities and an unseen diff, which a plan cannot predict, and the bosun-code floor
  applies only where the base has bosun's own code (the `bosun_code` argument). risk-check stays
  the judge at merge.
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
`bosun:prereqs`.

**Trust: the plan decides, the issue only carries ticks.** What an item is (its id, what it
blocks, how it is verified) comes only from the plan's `## Prerequisites` on the default branch,
which reached main through a reviewed PR. The issue is a view the owner taps on; it supplies one
fact per item, the tick. Each line ends with a marker bosun writes:
`<!-- bosun:prereq H1 sum=1a2b3c4d state=api -->`, where `sum` is the first 8 hex characters of
the SHA-256 of the item's title, steps and verify, and `state` is `owner`, `api`, `api ok` or
`api 403` (informational). A tick counts only for an item whose verify is `owner`, or an API item
whose read got a 403, and only on a line whose `sum` matches the plan's item now (an amendment
that reuses an id with new steps makes old ticks stale). An API item (secret, variable,
environment, file) is done only when an API read finds it; bosun and the gates read the API
themselves and never trust an `ok` in the issue. A missing line, a line whose id appears twice,
or a stale `sum` is open. bosun reads the markers whatever the issue's state.

A tick is not authenticated: anyone with write access can tick a box. That is accepted because a
prerequisite only schedules work and risk-check still decides every merge. The PreToolUse hook
refuses an agent's GitHub write through the tools it can see (the GitHub MCP tools, `gh issue
edit/comment/close`, `gh api`) whose arguments contain `bosun:prereq`; that is best effort (a
body read from a file, `--input`, or `curl` with a token escape a string match), so the guarantee
rests on the rules above: an agent can forge at most a tick on an owner item. All text that
reaches the owner (titles, steps, `who`, `bosun hold` text, PR titles in an approval message, the
plan title) is made inert first: control, format and separator characters (Unicode Cc, Cf, Zl,
Zp, among them U+2028, U+200B, U+202E, U+FEFF) become spaces, and the text is shown as an inline
code span, so markdown, HTML, links and @mentions render as plain text. The parser also refuses
`<!--`, `-->` and such characters in plan text.

**F5. Gating, in three places, failing closed.** (1) `bosun plan import`/`amend` of a plan with
a `## Prerequisites` section first refreshes its checklist issue exactly as `bosun prereqs --post
--no-setup` does (API reads, create or reopen the issue), so every such plan has one; with
`--dry-run` it does the reads and writes nothing, and `data.held` comes from those reads. It then
keeps a task `planned` while an open item blocks it or its item (the epic of a blocked item stays
`planned` too) and lists it under `data.held`. The next import after the item is done promotes
it. (2) `bosun claim` refuses (exit 5) a task that an open item blocks, and `bosun wave plan`
leaves it out, because tasks also become ready outside promote (`bosun release --to ready`, reap)
and an item can appear after a task was promoted. The lookup is task -> epic -> the epic's plan
path -> that plan file on the default branch (its items) plus the `bosun:prereqs` issue whose
marker names that path (its ticks); item ids repeat across plans, so no other plan's issue counts.
They fail closed (exit 5, with the fix `bosun prereqs <plan> --post`): when the plan file cannot be
read on the default branch (a 404 is not "no section") or does not parse; when a plan with the
section has no checklist issue and some blocking item needs a tick; and when more than one issue
carries the plan's marker. A plan without the section needs no issue. (3) `bosun spawn` refuses an
epic that an open item blocks as a whole item (`blocks: M5`), with the same lookup and rules.
Everything else proceeds. The automatic items (`D<n>` doctor steps, `K1`) block no task: they are
setup, listed so the owner does them first.

**F6. No mid-run asks.** An agent that hits an undeclared human step does not stop and wait. It
runs `bosun hold <task issue> "<what the owner must do>" [--phone]`, a top-level command that
checks the kill switch, refuses text with comment markers or control characters (as plan text),
and records a hold on the task issue: a record comment `{"bosun":"hold","id":"X<n>",...}` like
the lease records, so the hold is durable outside the checklist. It adds a box `X<n>` (owner
verified, blocking that task) to the checklist issue of the task's plan (created or reopened when
missing or closed), releases the agent's lease, moves the task to `ready` (the claim gate keeps it
there) and records a lesson best-effort (a failed lesson, such as the cap's exit 6, is reported
in `data.lesson_error` and does not undo the hold). The gates read the hold records of the task:
an X item is open until its box is ticked; a missing box is open. The agent takes other work. The
orchestrator relays the updated checklist once and folds the item into the plan's
`## Prerequisites` in the next amendment, as an H item with `replaces: X1`, which closes the hold.
Every rewrite of the checklist carries over the X boxes of open holds. `bosun ask` remains for
decisions, which also go to the orchestrator's batch.

**F7. Approval batches.** `bosun approve --list` adds `data.message`: one short text listing the
ready PRs it shows (at most 10: `#12@abc1234` and the inert title) and the reply to send
(`approve 12 15 17`), plus `data.message_id` (the first 8 hex characters of the SHA-256 of the
shown pairs' sorted `<pr>@<head>` lines) and `data.message_pairs` (exactly those pairs). The id
names a message; the binding is the stored pairs. Only the latest message is answerable: the
orchestrator keeps the `message_pairs` of the last message it relayed and forgets the older ones
when it relays a new one. A reply is mapped to those pairs; a reply that names the id of an older
message, or that the owner sent before the latest message was relayed, is not signed: the
orchestrator says so and relays the latest message again. The id is optional in the reply
(`approve 1a2b3c4d: 12 15` is checked against the latest id; plain `approve 12 15` answers the
latest message), except when a PR's head changed between the last two messages: then the reply
must name the id. PR numbers come only from the owner's own chat message, never from text in a
PR, an issue or a listing (titles are agent-written). Each `(message id, pr)` pair is signed at
most once, also after a compaction or a re-read of the chat. Signing uses `--sign-key` with the
pair's `<pr>@<head>` (a bare number stays exit 2); a head that moved after the message is exit 3,
and that PR goes into the next message. The orchestrator relays a message when a batch from F3 is
complete (or at most every few hours), not per PR.

**F8. Docs.** A spec section (after the spec split lands), `docs/prerequisites.md`, AGENTS.md
rules for agents and orchestrators ("front-load human steps; never block mid-run"), the keelstack
playbook/Start here text, and decision D-009 in keelstackhq/decisions.

## Out of scope

Running hoist from bosun (doctor already does); free-form commands as verifiers (a plan file is
agent-written, bosun must not execute it); notifications beyond the issue and the relayed text.
