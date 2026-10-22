# bosun: design spec

Date: 2026-10-22 · Author: Adam Urban (owner) · Status: accepted for the seed

Names: repo `keelstackhq/bosun`, binary `bosun`, Cargo package `keel-bosun` (lib `bosun`), npm
launcher `@keelstackhq/bosun`. Part of the keelstackhq family (splice, hull, hoard, hoist, berth).

bosun is a stack-agnostic orchestration helper for repos where agents write all the code. It is
one static Rust binary. It owns the process logic, so a session only calls bosun and acts on its
JSON output.

This spec is the "forge" spec of 2026-10-22 with the forge name changed to bosun and the owner's
adjustments applied (D13-D26). Where an adjustment and the original text disagreed, the text below
already follows the adjustment.

## 1. Summary and decisions

bosun lets one or more autonomous Claude Code sessions build a repo with no human edits to code.
Humans set up the repo, own the risk decisions, and approve high-risk PRs. Everything else goes
through bosun commands, GitHub issues and PRs, and CI.

| #   | Decision | Choice |
| --- | -------- | ------ |
| D1  | Product shape | bosun is its own product in its own repo: one static Rust binary, stack-agnostic. It builds itself first. |
| D2  | Coordination substrate | GitHub issues, labels, comments and PRs. No other database. |
| D3  | Process logic | Only in the bosun CLI. JSON output, fixed exit codes. Hooks block the bypasses. |
| D4  | Human authority | Tiered. A human acts only on high-risk changes, questions and budget raises. |
| D5  | Execution | Hybrid: one orchestrator session per area (epic), and wave agents inside it. |
| D6  | Limits | Hard caps on depth, live sessions, children, parallel agents, turns and money. |
| D7  | Verification | CI gates through a generic gate contract, machine-checkable UCs, mutants, and an agent review verdict. |
| D8  | Review | Mode (a) a fresh subagent first. Mode (b) a CI check later (M8). |
| D9  | Learning | Lessons are captured, triaged and pruned. The preferred result is a mechanism, not advice. |
| D10 | Seed | Seed A: a human checklist plus agent seed PRs (see D22). |
| D11 | Slack | Through Claude Tag only. Threads may start sessions with the same checks as any spawn. |
| D12 | Generated files | No reference to the source playbook in generated files. Each comment states its own reason. The evidence for the defaults is in `docs/defaults.md`. |
| D13 | Merging and tags (A1) | Agents never merge, tag or force-push. Merges go through the GitHub merge queue or auto-merge, which `bosun pr --auto-merge` turns on after all gates are green. Releases run through `hoist release` in a workflow; tags come from workflows only. |
| D14 | Waiting (A2) | `bosun wait` polls GitHub state with backoff inside one call. It never depends on send_later or send_message. Child-to-parent results go through issue comments. |
| D15 | Required checks (A3) | `ci-ok` aggregates every `gate-*` job (the org convention). `risk-check`, `review-verdict` and `main-open` stay separate required checks. |
| D16 | Init and upgrade (A4) | `bosun init` runs `splice channel add gh:keelstackhq/bosun//channel` (pieces `core`, `rust`, `ts-pnpm`). There is no `bosun upgrade`: the splice update bot (`keelstackhq/splice/action@v1`) keeps the channel fresh. |
| D17 | Repo settings (A5) | Checklist steps 3 and 5 (branch protection, environments, secrets, variables) are done with `hoist apply`. `bosun doctor --remote` calls `hoist check`. |
| D18 | Workspaces (A6) | Wave agents get isolated workspaces from `berth up` (worktree, env files, port block, database per agent). `berth gc` cleans up after merge. |
| D19 | CI cache (A7) | Generated gate workflows cache through `keelstackhq/hoard@v0`. |
| D20 | Usage (A8) | The usage provider `claude-sessions` reads `usage.cost_usd` from the claude-code-remote `get_session` result. It ships in the seed with the `generic` estimate. |
| D21 | Exit codes (A9) | Exit codes 0-6 as in §1.1; 2 is usage, as in splice and berth. Every error JSON has a `fix` field that says how to fix it. |
| D22 | Seed shape (A10) | The seed is a sequence of PRs of at most 400 changed lines each, not one seed PR. |
| D23 | Environment (A11) | The environment setup script installs Node 24, pnpm 12 (native binary), rust 1.97, cargo-nextest, cargo-deny, and the pinned bosun, splice, hoist and berth. |
| D24 | First publish (A12) | The first publish follows the hoist bootstrap flow (`hoist release bootstrap`). npm trusted publishers expire if unused for 2 days, so the first workflow release follows within 2 days. |
| D25 | Repo and scope (A13) | The repo is `keelstackhq/bosun` (Q1 answered). SP2, the runtime, is out of scope. |
| D26 | Org rule (A14) | "Agents never merge or tag; merges happen via auto-merge after the gates" is proposed to keelstackhq/decisions as D-006. bosun follows it now. |
| D27 | Crate shape | One package `keel-bosun` with modules `cli`, `core`, `github`, `sessions`, `gates`, `plan`, `hooks`, not a workspace of 7 crates. This matches berth and hoist. `core` does no I/O; a test enforces it. |
| D28 | HTTP | `ureq` 3 with rustls (as hoist), not octocrab. bosun is synchronous; no tokio. |
| D29 | Session tools | bosun never calls the claude-code-remote tools itself; a binary cannot. A command that needs a tool call returns it in `next` (tool name and arguments). The session makes the call and passes the result back with `--from -` on stdin. |
| D30 | Clock and tests | Time comes from `BOSUN_NOW` (RFC 3339) when set; the GitHub base URL from `BOSUN_GITHUB_API`. Tests use a fake GitHub server and never reach a real API. |
| D31 | Identity | The repo comes from `BOSUN_REPO` (`owner/name`), else the `origin` remote. The session id comes from `BOSUN_SESSION`, else `CLAUDE_CODE_REMOTE_SESSION_ID` with its `cse_` prefix changed to `session_`, else `local-<host>`. The GitHub token comes from `GITHUB_TOKEN` or `GH_TOKEN`, never argv. |

### 1.1 Exit codes

Fixed for all commands:

| Code | Name | Meaning |
| ---- | ---- | ------- |
| 0 | ok | The command did its work. |
| 1 | error | An unexpected failure (API, I/O, bad config file). The JSON has the error and its fix. |
| 2 | usage | Bad arguments. |
| 3 | conflict | A claim or a path overlaps a live lease, or another orchestrator owns the area. |
| 4 | halted | `BOSUN_HALT=1`, main-red or process-red stops the command. |
| 5 | needs-human | The command needs an owner decision. |
| 6 | limit | A cap or a budget refuses the command. |

### 1.2 JSON output

stdout is always exactly one JSON object, also on errors and usage errors; bosun has no table
output, because its reader is a session (D3). `--json` is accepted and changes nothing, so calls
read the same as for splice, hoist and berth. Logs and child-process output go to stderr. Hook
commands are the one exception: their stdout follows the Claude Code hook protocol (§4, S8).

```json
{"version":1,"command":"claim","ok":false,"exit_code":3,
 "data":{},
 "error":{"kind":"conflict","message":"crates/x.rs is leased by session_02 until 10:30Z",
          "fix":"pick another task with `bosun wave plan`, or wait for the lease to end"},
 "next":[]}
```

- `data` is the command's result. Its shape is per command and documented in `--help`.
- `error` is `null` when `ok` is true. `kind` is the exit-code name of §1.1. `fix` is never empty.
- `next` lists the tool calls the session must make (D29): `{"tool":"create_session","args":{..}}`.

## 2. Scope and sub-projects

SP1: bosun, the orchestration helper (this document). SP2: the agent runtime, in its own repo, is
bosun's first consumer and is designed in a later session (out of scope, D25). This document gives
only the interface that SP2 uses: `bosun init --stack rust --plan runtime-plan.md`.

In scope for bosun: the work lifecycle, spawning and limits, verification gates, risk gating,
main-health, cost, learning, Slack intake, bootstrap and reuse. Not in scope: an LLM client, a
model host, a database, a web UI. bosun uses the GitHub API and asks the session for
claude-code-remote calls (D29). It does not run models.

## 3. Repo layout

bosun repo:

```
Cargo.toml                package keel-bosun, lib bosun, bin bosun
src/cli/                  clap commands, JSON output, exit codes
src/core/                 lifecycle, leases, limits, risk, lessons; no I/O
src/github/               issues, labels, comments, PRs, checks (ureq)
src/sessions/             claude-code-remote requests and results, usage providers
src/gates/                gate contract, profiles, adapters
src/plan/                 plan.md parser, import, amend
src/hooks/                PreToolUse and SessionStart hook entry points
profiles/                 rust.toml, ts-pnpm.toml
channel/                  the splice channel that `bosun init` adds (pieces core, rust, ts-pnpm)
docs/spec.md, docs/defaults.md, docs/plan-format.md
.bosun/                   bosun runs on itself
```

A consumer repo after `bosun init`:

```
.bosun/bosun.toml         project, stacks, review mode, slack
.bosun/limits.toml        caps and budget (§5)
.bosun/risk.toml          risk rules (§8)
.bosun/version            pinned bosun release (R-gov)
governance/owners.toml    GitHub and Slack IDs of the owners
docs/lessons.toml         the lessons store
plans/*.md                plans in the plan format
.github/workflows/        call only `bosun gate <name>`; cache through hoard
.github/CODEOWNERS
.claude/settings.json     hooks: bosun hook pre-tool-use, bosun hook session-start
AGENTS.md                 no lessons; points to bosun commands
REVIEW.md                 review rules for the verdict
.splice/channels/bosun.toml   the channel record, kept fresh by the splice update bot
```

## 4. Work lifecycle and conflict avoidance

Units: a plan has items. Each item becomes one epic issue. An item has tasks. A task is at most
400 changed lines and has one context pack. The critical path of an item has at most 4 tasks.
Each item has a "Done when" list; each line becomes one UC (`UC-<item>-<n>`).

Labels:
- `bosun:state/` planned, ready, claimed, building, review, fixing, queued, merged, blocked, needs-human
- `bosun:kind/` feature, plan, mainfix, bookkeeping, flake, process-fix
- `bosun:area/` one per area (one area = one orchestrator)
- `bosun:source/` plan, slack, lesson, flake, human

State machine:

| From | To | When |
| ---- | -- | ---- |
| planned | ready | only when its plan task merged |
| ready | claimed | `bosun claim`; lease written |
| claimed | building | first commit pushed |
| building | review | `bosun pr`; gates green |
| review | fixing | verdict request-changes |
| fixing | review | fix pushed |
| review | queued | verdict approve; risk ok; `bosun pr --auto-merge` turned auto-merge on |
| queued | merged | the merge queue or auto-merge merged it (GitHub, never an agent: D13) |
| any | blocked | stop rule, conflict, budget |
| any | needs-human | risk high, owner question, tier escalation used up |

Only bosun commands change the state label. A hook refuses a direct label edit.

Stop rules:
- The same failure signature 3 times in a row: the task goes to blocked.
- A fix round with 0 changed lines: the task goes to blocked.
- On blocked, bosun escalates one model tier (haiku -> sonnet -> opus). When opus is used up, the
  task goes to needs-human.
- Each stop rule opens or hits a lesson.
- When main is red, feature work pauses; only mainfix tasks continue.

Leases: a claim is an issue comment with one JSON line:

```json
{"bosun":"lease","session":"session_...","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/core/lease.rs"]}
```

The TTL is 30 min; `bosun heartbeat` extends it. `bosun reap` releases expired leases and sets the
task back to ready. `bosun release` ends a lease in the normal way. A released or reaped lease gets
a second comment `{"bosun":"lease-end","session":..,"reason":"released|reaped"}`; the live lease
of an issue is the newest lease comment with no later lease-end for the same session.

Conflict avoidance (three layers):
1. Registry: `bosun join` registers each session with its origin (spawned, slack, local, routine)
   as a comment on the registry issue (label `bosun:registry`). All caps count all origins.
2. Claim check: `bosun claim` refuses a task if its touch paths overlap a live lease, or if another
   orchestrator owns its area. Exit 3 with a JSON reason. An area owner that is the
   parent of the claiming session does not block it, so spawned workers can claim in their
   orchestrator's area.
3. Edit hook: a PreToolUse hook blocks Edit and Write on a path that another live session claimed.

Touch paths are repo-relative globs (`*` within a segment, `**` across segments). Two touch lists
overlap when a path of one matches a glob of the other, or two globs can match a common path.

Intake: intake is the only way that new work enters. `bosun intake` (from Slack, a lesson, a flake
or a human) writes a plan amendment as a kind/plan task. Only a merged plan task makes its tasks
ready.

## 5. Spawning, waves and limits

Only an epic gets a session (an area orchestrator). Leaf tasks run as wave agents inside that
session, each in its own workspace from `berth up <task>` (D18); `berth gc` removes the workspaces
of merged tasks. `bosun wave plan <epic>` returns the next set of tasks that can run in parallel:
no overlapping touch paths, all dependencies merged, at most `waves.max_parallel`.

`bosun wait` blocks until a PR, a check, an issue or a child session's result comment changes
state (D14). It polls GitHub with backoff (5 s, doubling, at most 60 s) inside one call, up to
`--timeout` (default 30 min), and returns the change as JSON. It replaces polling by the session
and never uses send_later or send_message.

`bosun spawn` checks, in this order:
1. Main is not red.
2. The daily budget is below 80 %.
3. The depth of the new session is at most max_depth.
4. The live sessions (all origins) are fewer than max_live_sessions.
5. The children of the parent are fewer than max_children.
6. There is no conflict with a live lease or area owner.
7. Claim the epic, then return the `create_session` call in `next` (D29).

A refusal is exit 6 (limit) with the failed check in the JSON; a conflict in check 6 is exit 3.
After the session made the call, `bosun spawn --record --from -` reads the `create_session` result
and registers the child. Tags on the new session: `bosun:<project>`, `bosun:issue/<n>`,
`bosun:parent/<id>`. bosun never passes a `[1m]` model.

`.bosun/limits.toml` defaults (evidence in `docs/defaults.md`):

```toml
[sessions]
max_depth = 2          # root -> area orchestrator -> no further sessions
max_live_sessions = 4
max_children = 3
[waves]
max_parallel = 6
[turns]
build = 100
build_cap = 150
review = 40
fix_round = 40
[models]
read = "haiku"
build = "sonnet"
review = "sonnet"
hard = "opus"
[budget]
daily_usd = 150
per_task_usd = 25
```

With `max_live_sessions = 1` the same code path runs: the root session does all the work in waves.

Janitor, resume, results:
- Janitor (Routine, every 15 min, Haiku): `bosun reap`, a stale-session check, a budget check,
  `bosun notify`.
- Resume: after a container restart, `bosun resume` reads the leases and the open PRs of the
  session and continues.
- Results (D14): a child posts its result as an issue comment on its epic,
  `{"bosun":"result","session":..,"outcome":"merged|blocked|needs-human","pr":..}`. The parent
  sees it with `bosun wait`.
- Steering: `bosun msg <session> <text>` returns a `send_message` call in `next` with the prefix
  "From #<issue>:" and refuses text over 500 characters. Nothing in bosun waits for a reply.

Bootstrap order: the root session runs `bosun plan import`, then the wave of plan tasks, then
spawns one session per area, then coordinates with `bosun wait` and `bosun status`.

## 6. Learning loop

A lesson is a failure that is not fixed in code yet. The preferred result of a lesson is a
mechanism (a gate, a hook, a check). Advice text is the last choice.

Capture:
- Automatic: bosun opens a lesson issue on a stop rule, a blocked task, a flake, a review ask that
  repeats, or a task over its budget.
- Manual: `bosun lesson add "<text>"`, or Slack intake with `--kind lesson`.
- Duplicates: a lesson with the same signature adds a hit to the existing issue; it does not open
  a new one.

Triage (curator Routine, daily, Sonnet):
- blocking: sets process-red, so waves pause as on main-red.
- mechanizable: opens a kind/process-fix task, which runs before the features of the same area.
- advice only: adds an entry to `docs/lessons.toml`.
The lesson issue closes when its mechanism merges.

Store `docs/lessons.toml`:

```toml
[[lesson]]
id = "L-012"
rule = "Run cargo metadata once per gate run, not once per package."
why = "The test gate took 9 min on 40 packages."
scope = ["src/gates/**"]
evidence = ["#214", "#230"]
added = "2026-11-03"
last_hit = "2026-11-10"
hits = 3
```

- Caps: at most 40 entries, at most 8 global (no scope).
- A context pack includes only the lessons whose scope matches the paths of the task, plus the
  global ones. AGENTS.md has no lessons.

Prune:
- `bosun lesson hit <id>` records a hit.
- At the cap, `bosun lesson add` refuses with exit 6. The curator must then mechanize a lesson,
  merge two lessons, or retire a lesson with no hits in 30 days.
- A lesson that gets a hit after its mechanism merged reopens as blocking.
- `bosun lesson --upstream` sends a lesson about bosun itself to keelstackhq/bosun.
- Metrics: `bosun progress --lessons` (open, mechanized, hits per week).

## 7. Verification and the gate contract

Use cases: each "Done when" line of an item becomes `UC-<item>-<n>`. The plan task writes each UC
as a pending test (for example `#[ignore = "UC-M1-2"]`). The build agent only removes the marker
and makes the test pass. `bosun accept <epic>` checks that each UC test exists, is not pending,
and passes; only then does the epic close.

Gate contract (stack-agnostic):
- fmt, lint, types, deps, arch: on PR.
- test: on PR (affected packages) and in the merge queue (all).
- mutants, smoke, snapshots: in the merge queue.
- test-integrity: always, as a gate job.
- risk-check, review-verdict, main-open: always, each its own required check (D15).

CI templates call only `bosun gate <name>`; each gate is one job `gate-<name>`, and the job
`ci-ok` needs every `gate-*` job and fails unless all succeeded. Jobs cache through
`keelstackhq/hoard@v0` (D19). A profile maps each gate to a command.

Profiles map gates to commands with the placeholders `{packages}`, `{files}` and `{diff}`. Each
profile has three adapters:
- packages: finds the affected packages from the diff;
- pending: finds and counts pending-test markers;
- assertions: counts assertions, so test-integrity can see a weakened test.

| Gate | rust | ts-pnpm |
| ---- | ---- | ------- |
| fmt | cargo fmt --check | prettier --check {files} |
| lint | cargo clippy --all-targets -- -D warnings | eslint --max-warnings 0 {files} |
| types | (in lint) | tsc -b |
| test | cargo nextest run -p {packages} | vitest run --project {packages} |
| deps | cargo deny check | bosun deps-check |
| mutants | cargo mutants --in-diff {diff} | stryker run --incremental |
| snapshots | cargo insta test --check | vitest snapshots |
| adapters | cargo-metadata, rust-ignore, rust | pnpm-workspace, vitest-fixme, ts-expect |

- Custom profiles and generic regex adapters are allowed; `bosun status` shows "reduced
  integrity" when a generic adapter is used.
- A mixed repo uses one `[[stack]]` per path (`path = "web/"`, `profile = "ts-pnpm"`).
- `unsupported = ["mutants"]` needs a reason and an owner approval (R-gov).
- Packages adapter (rust): a changed root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` or `.cargo/**`, or a changed `.rs`, `.toml` or `.lock` file in no package, means all packages; other files outside every package (docs) affect none.
- A gate with `each_package` and `when_no_package = "skip"` is skipped (exit 0, `skipped: true`) when no package is affected; the default `when_no_package` is `run`, which runs the gate with its `all` arguments.

Time budgets: a PR run is at most 10 min, a merge queue run at most 15 min, one job at most 8 min.
A gate over budget opens a lesson.

test-integrity:
- Refuses a PR that deletes a test, adds a skip marker, or lowers the assertion count, unless the
  plan says so.
- A snapshot change needs the line `snapshot-change: <reason>` in the PR body.
- Until `--pr` reads the PR and the task metadata, the PR body comes from env `BOSUN_PR_BODY` and the allowed kinds (`delete-test`, `skip`, `fewer-asserts`) from env `BOSUN_INTEGRITY_ALLOW` (comma-separated). Findings the task allows are reported with `allowed: true` and do not fail. A `#[ignore = "UC-…"]` marker is pending, not a skip.
- Mutants that survive fail the merge queue.

Review verdict: `bosun review-pack <pr>` builds the input (diff, task, UCs, matching lessons,
REVIEW.md). The reviewer returns:

```json
{"pr":42,"head_sha":"abc123","verdict":"approve","confidence":0.9,
 "findings":[{"severity":"major","file":"src/core/lease.rs","line":88,"claim":"...","evidence":"..."}],
 "lessons_hit":["L-012"],"escalate_risk":false}
```

- `bosun verdict post` writes it as the review-verdict check on `head_sha`.
- Approve needs confidence >= 0.85 and no blocker finding.
- A later review of the same PR is a delta review (only the new commits).
- `verdict post` also comments `{"bosun":"verdict","session":…,"head_sha":…,"approved":…}` on the PR; the next `review-pack` takes `since_sha` from the newest such record (statuses are per SHA, so they cannot carry it).
- `verdict post` moves the linked task (`Closes #N`) to `fixing` when not approved and to `needs-human` when `escalate_risk` is true, if the state machine allows it; approval leaves the state alone (`bosun pr --auto-merge` queues it). Invalid verdict JSON is exit 2.
- `review.mode = "subagent"` (mode a) now; `"ci"` (mode b) after M8.

Flakes: a test that fails and then passes on the same commit is a flake. bosun opens a kind/flake
task and a lesson. A test is never skipped to get green.

## 8. Risk gating

`.bosun/risk.toml` rules:
- R-gov: "Governance files decide who approves what." Paths `.bosun/**`, `governance/**`,
  `.github/CODEOWNERS`, `.claude/**`, `.splice/**`.
- R-ci: "CI decides what is verified." Paths `.github/workflows/**`.
- R-secrets: "A secret in the repo is a leak." Content `(?i)(api[_-]?key|secret|token)\s*=`.
- R-deps: "A new dependency adds code that nobody reviewed." `new_dependency = true`.
- R-prod-write: "A write to production cannot be undone by a revert." `capability = "prod-write"`.

Rule evaluation:
- risk-check evaluates the rules from the base branch, never from the PR head. A PR cannot change
  the rules that judge it.
- A rule only escalates; nothing de-escalates.
- The bosun version pin (`.bosun/version`) is R-gov.

High-risk PRs:
- A high PR needs a GitHub review on the head SHA by a user in `governance/owners.toml`, and
  CODEOWNERS.
- bosun writes a Risk note into the PR body: the rules that matched, why, and what the owner must
  check.
- `bosun ask` sends the question to the owner (GitHub mention, and Slack if set). There is no
  timeout and no automatic approval.
- A risky task is small: the target is under 100 changed lines.

Kill switch: the repo variable `BOSUN_HALT=1` stops all bosun commands that start or change work
(exit 4). Only the owner sets it.

Audit: `bosun progress --risk` lists the high PRs, who approved them, and the time to approval.

## 9. Main-health, cost and spend guard

Main-health:
- The main-health workflow runs on each push to main and nightly. It opens one main-red issue on
  failure and closes it on recovery.
- The required check main-open fails every non-mainfix PR while main is red.
- `bosun main-triage` bisects the merged PRs in code and names the first bad one.
- The default fix is `bosun revert <pr>` as a kind/mainfix PR. A forward fix is allowed only if it
  is smaller.
- Each main-red opens a lesson.

Cost:
- Usage providers (D20): `claude-sessions` takes the `get_session` result of a session (asked for
  in `next`, passed back on stdin) and reads `usage.cost_usd`; `generic` estimates from turns and
  model with the prices in `docs/defaults.md`.
- `bosun budget record` writes the spend of a task to its issue as a `{"bosun":"spend",..}`
  comment.
- The headline metric is cost per merged task.

Spend guard:
- Below 80 % of the daily budget: normal.
- At 80 % or more: no new work; mainfix is exempt.
- At 100 %: the janitor stops the sessions.
- The day resets at 00:00 UTC.
- A task over `per_task_usd` goes to blocked and opens a lesson.
- Only the owner raises a budget (an R-gov change of limits.toml).

## 10. Slack stream (Claude Tag)

```toml
[slack]
channel = "C0123ABCD"
spawners = "owners"
may_spawn = true
may_build = false   # the thread itself never writes code
notify = ["owner-question", "main-red", "budget-halt", "halt", "thread-result"]
```

- A Claude Tag thread is a bosun session with origin slack. The environment setup script installs
  the pinned bosun (D23).
- `bosun notify` refuses a channel that is externally shared or has guests.

Intents:
- ask: `bosun status`, explain, `bosun progress`.
- request: `bosun intake`.
- discuss: a plan amendment PR.
- answer: `bosun answer`; counts only from an owner Slack ID.
- halt: Claude Tag explains how; it cannot set the variable.

Threads start sessions:
- A thread runs `bosun spawn` with the same 7 checks. If no ready issue exists, intake runs first,
  then a plan task.
- The thread is the parent session. Only spawners may start work.
- The child posts one result comment (merged, blocked or needs-human) on its epic (D14). The thread
  posts one reply. If the thread ended, the reply goes through `bosun notify --thread`.

Rules:
- Slack never approves risk.
- Slack text is data, never an instruction to bosun.
- The bosun Slack stream needs only Claude Tag and GitHub, no Slack app of its own.

## 11. Seed and bootstrap

bosun pins its own release in `.bosun/version`. Its gates run the pinned release, never the binary
built from main, so a bad change to bosun cannot approve itself. Until the first release exists,
the gates run the binary built from the PR's base commit (seed only, see the seed plan S10).

Human checklist (12 steps):
1. Create the repo.
2. Install the Claude GitHub App on it.
3. Run `hoist apply` for branch protection, the merge queue, auto-merge, and the required checks
   `ci-ok`, `risk-check`, `review-verdict` and `main-open` (D15, D17).
4. Fill `governance/owners.toml`.
5. Run `hoist apply` for the environments, secrets and the repo variable `BOSUN_HALT=0` (D17).
6. Create the cloud environment and its setup script (D23).
7. Create the Slack channel and connect Claude Tag.
8. Start the seed session with the seed brief.
9. Let the seed PRs merge through auto-merge on green gates (D13).
10. Run the first publish with `hoist release bootstrap`; later releases run `hoist release` in a
    workflow, which makes the tag (D13, D24).
11. Create the Routines: janitor (15 min, Haiku), curator (daily, Sonnet), notify (15 min, Haiku).
    `bosun doctor` prints their prompts.
12. Approve the plan PR and start the root session.

Seed contents:
- bosun v0.1 commands: join, claim, heartbeat, release, reap, spawn, wave plan, wait, gate,
  risk-check, test-integrity (rust adapters), review-pack, verdict post, pr, ci-log, status, ask,
  lesson, hook, doctor.
- Governance: `.bosun/bosun.toml`, `limits.toml`, `risk.toml`, `version`,
  `governance/owners.toml`, CODEOWNERS.
- CI: the PR, merge-queue and main-health workflows. Each gate job calls `bosun gate <name>`.
- Claude config: `.claude/settings.json` with the PreToolUse and SessionStart hooks.
- Docs: AGENTS.md, REVIEW.md, `docs/plan-format.md`, `docs/spec.md`, `docs/defaults.md`.
- Plan: `plans/bosun.md` with items M1-M8.

Built later by bosun on itself: intake, answer, notify, budget, progress, main-triage, revert,
plan import and amend, init (the splice channel, D16), the curator, the ts-pnpm profile, `doctor
--remote`.

Seed steps (the seed plan in `docs/superpowers/plans/` splits each into PRs):
- S1: crate, modules, clap skeleton, JSON output, exit codes. Done when `bosun --help` lists all
  v0.1 commands and each command returns JSON.
- S2: GitHub client (labels, issues, comments, PRs, checks). Done when integration tests against a
  recorded fake pass.
- S3: registry and leases (join, claim, heartbeat, release, reap). Done when a claim with
  overlapping touch returns exit 3.
- S4: spawn and the 7 checks, wave plan, wait. Done when each check has a refusal test with exit 6.
- S5: gate runner, rust profile and adapters, test-integrity. Done when `bosun gate test` runs
  nextest on the affected packages only.
- S6: risk-check from the base branch, ask. Done when a PR that edits risk.toml is high, evaluated
  with the base rules.
- S7: review-pack, verdict post, pr (with `--auto-merge`). Done when a verdict below 0.85 does not
  approve.
- S8: hooks (edit-lock, label-lock, session-start). Done when an Edit on a claimed path is blocked.
- S9: lesson (manual add, hit, cap), status, doctor, usage providers. Done when `bosun doctor`
  reports each checklist step as ok or missing.
- S10: governance files, CI, AGENTS.md, REVIEW.md, plan. Done when CI is green on the S10 PR with
  the gates run by bosun.

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

M2 Plan import. Tasks: plan-format parser; bosun plan import (epics, plan tasks, UC pending tests);
bosun plan amend; bosun intake. Done when:
- UC-M2-1: importing the same plan twice creates no duplicate issues.
- UC-M2-2: a task becomes ready only after its plan task merged.
- UC-M2-3: each "Done when" line has a pending UC test after import.
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

## 13. Open questions

- Q1: the repo name. ANSWERED: keelstackhq/bosun (D25).
- Q2: the Slack app identity of SP2. Owner; needed by the SP2 design.
- Q3: the default values of limits.toml and the budget are first estimates. They need a review
  after M4 with real cost data.
- Q4: the 0.85 confidence limit and the 18-of-20 agreement limit of M8 need data from mode (a)
  first.
- Q5: the gates of the seed run the binary built from the base commit until v0.1.0 is published
  (§11). The owner decides when the pin moves to the first release.
