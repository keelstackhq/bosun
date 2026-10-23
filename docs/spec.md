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
| D19 | CI cache (A7) | Generated gate workflows cache through `keelstackhq/hoard@v0`. hoard is a Turborepo remote cache, so it applies to ts-pnpm stacks; Rust gate jobs (bosun's own CI included) cache with `Swatinem/rust-cache`, as berth and hoist do. |
| D20 | Usage (A8) | The usage provider `claude-sessions` reads `usage.cost_usd` from the claude-code-remote `get_session` result. It ships in the seed with the `generic` estimate. |
| D21 | Exit codes (A9) | Exit codes 0-6 as in §1.1; 2 is usage, as in splice and berth. Every error JSON has a `fix` field that says how to fix it. |
| D22 | Seed shape (A10) | The seed is a sequence of PRs of at most 400 changed lines each, not one seed PR. |
| D23 | Environment (A11) | The environment setup script installs Node 24, pnpm 12 (native binary), rust 1.97, cargo-nextest, cargo-deny, and the pinned bosun, splice, hoist and berth. |
| D24 | First publish (A12) | The first publish follows the hoist bootstrap flow (`hoist release bootstrap`). npm trusted publishers expire if unused for 2 days, so the first workflow release follows within 2 days. `release.yml` publishes nothing until the owner sets the repo variable `BOSUN_RELEASE=enabled`; the steps are in `docs/releasing.md`. |
| D25 | Repo and scope (A13) | The repo is `keelstackhq/bosun` (Q1 answered). SP2, the runtime, is out of scope. |
| D26 | Org rule (A14) | "Agents never merge or tag; merges happen via auto-merge after the gates" is org decision D-006 in keelstackhq/decisions (settled 2026-10-22). It reaches this repo through the decisions channel (D-004): `docs/agents/org-decisions.md`, imported by CLAUDE.md. |
| D27 | Crate shape | One package `keel-bosun` with modules `cli`, `core`, `github`, `sessions`, `gates`, `plan`, `hooks`, not a workspace of 7 crates. This matches berth and hoist. `core` does no I/O; a test enforces it. |
| D28 | HTTP | `ureq` 3 with rustls (as hoist), not octocrab. bosun is synchronous; no tokio. |
| D29 | Session tools | bosun never calls the claude-code-remote tools itself; a binary cannot. A command that needs a tool call returns it in `next` (tool name and arguments). The session makes the call and passes the result back with `--from -` on stdin. |
| D30 | Clock and tests | Time comes from `BOSUN_NOW` (RFC 3339) when set; the GitHub base URL from `BOSUN_GITHUB_API`. Tests use a fake GitHub server and never reach a real API. |
| D31 | Identity | The repo comes from `BOSUN_REPO` (`owner/name`), else the `origin` remote. The session id comes from `BOSUN_SESSION`, else `CLAUDE_CODE_REMOTE_SESSION_ID` with its `cse_` prefix changed to `session_`, else `local-<host>`. The GitHub token comes from `GITHUB_TOKEN` or `GH_TOKEN`, never argv. |
| D32 | Status posts | `risk-check` and `review-verdict` are commit statuses, because only GitHub Apps can create check runs. |

### 1.1 Exit codes

Fixed for all commands:

| Code | Name | Meaning |
| ---- | ---- | ------- |
| 0 | ok | The command did its work. |
| 1 | error | An unexpected failure (API, I/O, bad config file). The JSON has the error and its fix. |
| 2 | usage | Bad arguments. |
| 3 | conflict | A claim or a path overlaps a live lease, or another orchestrator owns the area. |
| 4 | halted | The kill switch (an owner's open `bosun:halt` issue, or `BOSUN_HALT=1`), main-red or process-red stops the command. |
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
- The npm launcher (`@keelstackhq/bosun`, `npm/bosun`) keeps this contract when it cannot start the binary: it prints one envelope with `exit_code` 1 and a `fix`, and the message on stderr. That holds for `hook` commands too; Claude Code treats a hook's exit 1 as a non-blocking error.

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

A channel version is a bosun release tag (`v<x.y.z>`), and the channel writes that version into
`.bosun/version`, so the update bot moves the channel files and the pinned binary in one R-gov PR.
A branch or sha ref has no release version; such a consumer sets the template var `bosun_version`.

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

`bosun pr` sets claimed -> building -> review (a PR exists only after the first push). `bosun pr` or `bosun reap` sets queued -> merged once GitHub merged the PR, and ends the leases on the issue with reason `merged`; `reap` also looks at queued tasks whose issue is closed.

Stop rules:
- The same failure signature 3 times in a row: the task goes to blocked.
- A fix round with 0 changed lines: the task goes to blocked.
- On blocked, bosun escalates one model tier (haiku -> sonnet -> opus). When opus is used up, the
  task goes to needs-human.
- `bosun fail <issue> --signature S [--changed-lines N] [--model M]` posts a `failure` record and
  applies these rules. A stop sets `blocked` and posts an `escalate` record (`from`, `to`,
  `reason`); an escalation restarts the count. With opus used up it sets `needs-human` (exit 5).
  The signature is in `data` for the lessons.
- Each stop (both paths) also captures a lesson of kind `stop-rule` from the rule and the
  signature; the same stop on another task hits the same lesson. `data.lesson` is `{issue,
  created, hits}`, or null with `lesson_error` when capture fails (the stop still stands).
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
   The hook reads live leases from `<git common dir>/bosun/leases.json` (refreshed after
   `BOSUN_LEASE_CACHE_SECS`, default 60). With GitHub down it uses a cache younger than the lease
   TTL, else it allows with a stderr warning. Paths outside the repo are never blocked. The edited path is matched as a literal path (brackets and braces included) against each lease glob.
   The merge-lock also checks commands nested in `$(..)`, backticks, `( .. )`, `sh|bash|zsh -c` and `eval` (depth 3), `gh api`, `curl`, `wget` and `http` writes that merge, create refs or releases or set auto-merge, and pushes of version-like refs (`v1.2.3`). Its default-branch rules (commit, merge, rebase, reset, cherry-pick, plain push) read the branch where git acts: the hook's directory moved by a `cd dir` joined with `&&`, `;` or a newline, and by `git -C dir`, `--git-dir` and `--work-tree`; a `cd` counts only when the shell surely runs it here (after the start, `;`, a newline, or in an `&&` chain that reaches the git command; never in a pipeline or a `( .. )` group); a directory it cannot read literally (`$VAR`, `~`, `cd -`), a moved target git cannot read (a failed `cd`), or `GIT_DIR`, `GIT_WORK_TREE` or `core.worktree` count as the default branch. Branch protection stays the real guard, since a hook cannot see every shell trick.

Leases conflict per issue, not per session: wave agents run inside their orchestrator's session, so a
lease on another issue conflicts even when it has the same session id; only a lease of the same session
on the issue being claimed is the same claim (a re-claim is idempotent). Give each wave agent
`BOSUN_SESSION=<session>/<task>` in its berth workspace; `S/x` conflicts with `S/y`, not with `S/x`.
`wave plan` counts the epic's active tasks that hold a live lease against `waves.max_parallel` and
blocks picks that overlap their touch paths. The edit hook keeps the session rule (an agent may edit
paths its orchestrator `S` leased), a known limit: without per-task ids it cannot tell wave agents apart.
Claim and spawn re-check after writing: they re-read the comments of the claimed issue and the active
issues, and when a live overlapping lease has an earlier comment (creation time, then comment id) they
post `lease-end` with reason `lost-race`, leave the state alone and exit 3 naming the winner.

Trust in records: a `{"bosun":…}` record in a comment is trusted from any commenter with write access to the repo; bosun does not check who wrote it. Checks that do not rely on records: commit statuses (`ci-ok`, `risk-check`, `review-verdict`, `main-open`), and the files read from the base or default branch (`docs/lessons.toml`, `REVIEW.md`, `.bosun/limits.toml`). A `spend` record with a negative or non-finite amount is ignored. A delta review (`since_sha` of `review-pack`) needs the `verdict` record and a successful `review-verdict` status on that sha (records on heads without one are skipped), and the sha must be an ancestor of the head and a commit of this PR; otherwise the review is full.

Touch paths are repo-relative globs (`*` within a segment, `**` across segments). Two touch lists
overlap when a path of one matches a glob of the other, or two globs can match a common path.

Intake: intake is the only way that new work enters. `bosun intake` (from Slack, a lesson, a flake
or a human) writes a plan amendment as a kind/plan task. Only a merged plan task makes its tasks
ready. After an amendment merges, `bosun plan amend` imports the new work and closes what the plan dropped.
`bosun intake "<text>" --source slack|lesson|flake|human [--item ID] [--title T] [--plan FILE]` opens one
`kind/plan` issue (state ready, `bosun:source/<source>`, `bosun:area/<area>` of `--item`) that asks for the
amendment of the plan file (default `plans/<project>.md`; a missing file or unknown item is exit 2). An open
issue of the same source with the same title is returned instead (`created: false`).

## 5. Spawning, waves and limits

Only an epic gets a session (an area orchestrator). Leaf tasks run as wave agents inside that
session, each in its own workspace from `berth up <task>` (D18); `berth gc` removes the workspaces
of merged tasks. `bosun wave plan <epic>` returns the next set of tasks that can run in parallel:
no overlapping touch paths, all dependencies merged, at most `waves.max_parallel`.
A task counts as merged when its state is `merged` or its issue is closed; tasks of other epics count too.

`bosun wait` blocks until a PR, a check, an issue or a child session's result comment changes
state (D14). It polls GitHub with backoff (5 s, doubling, at most 60 s) inside one call, up to
`--timeout` (default 30 min), and returns the change as JSON. It replaces polling by the session
and never uses send_later or send_message.
`polls` counts snapshots, the first included. A child's result is the newest `result` record of that session in the comments of open issues.

`bosun spawn` checks, in this order:
1. Main is not red, and process is not red (a mainfix or process-fix epic is exempt from process-red).
2. The daily budget is below 80 %.
3. The depth of the new session is at most max_depth.
4. The live sessions (all origins) are fewer than max_live_sessions.
5. The children of the parent are fewer than max_children.
6. There is no conflict with a live lease or area owner.
7. Claim the epic, then return the `create_session` call in `next` (D29).

`bosun wave plan` returns exit 4 while main is red (a mainfix epic is exempt) or while process is red (mainfix and process-fix epics are exempt). Under a feature epic with process-fix tasks, process-red plans only those tasks; the others wait.

`spawn`, `claim`, `budget record`, `wave plan` and `status` read `.bosun/limits.toml` from the default branch (the working tree copy never raises a cap; a differing local file is mentioned on stderr), and use the spec defaults when the file is absent there. The depth of the spawner is its live `join` record: a session without one is refused (exit 6, check 3, fix "run `bosun join` first"), and a later `join` never lowers the depth nor changes the parent of an existing record (it may raise the depth).

A refusal is exit 6 (limit) with the failed check in the JSON; a conflict in check 6 is exit 3.
After the session made the call, `bosun spawn --record --from -` reads the `create_session` result
and registers the child. Tags on the new session: `bosun:<project>`, `bosun:issue/<n>`,
`bosun:parent/<id>`. bosun never passes a `[1m]` model.
The epic lease written by spawn has no touch paths (`touch: []`); `--record` ends it with reason `spawned` and writes the same kind of lease for the child, plus the child's `join` (origin spawned, depth + 1, the epic's area).

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
- Janitor (Routine, every 15 min, Haiku): `bosun reap`, a stale-session check, a budget check (`bosun budget check`),
  `bosun notify`.
- Resume: after a container restart, `bosun resume` reads the leases and the open PRs of the
  session and continues.
  It renews the join record of the session (joining again if the registry has none) and every
  lease of the session in place. An expired lease is renewed only when no live lease overlaps
  its touch paths, else it is listed under `lost`. For each held task it returns the state,
  the linked open PR, its branch and a short `action`.
- Results (D14): a child runs `bosun result <epic> --outcome merged|blocked|needs-human [--pr N]`.
  It posts an issue comment on the epic with the record
  `{"bosun":"result","session":..,"outcome":..,"pr":..}`, ends the lease of the child on the epic
  (reason `result`) and leaves the registry (reason `done`). It works when bosun is halted. The
  parent sees the result with `bosun wait --child`.
- Steering: `bosun msg <session> <text>` returns a `send_message` call in `next` with the prefix
  "From #<issue>:" and refuses text over 500 characters. Without `--issue` the prefix is "From <session>:". Nothing in bosun waits for a reply.

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
- Lesson issue: label `bosun:lesson`, title `lesson: <title>`, and the marker
  `<!-- bosun:lesson {"signature":"<hex>","kind":"<kind>"} -->` in the body. The signature is
  FNV-1a 64 of the kind and the normalised failure text (hex ids, times, numbers and temp paths
  replaced; first 20 lines). `bosun lesson capture --kind <k> [--issue N] --from -` opens the
  issue, or posts a `Seen again` comment with a `lesson-seen` record on the open one with the same
  signature; that record is the hit.

Triage (curator Routine, daily, Sonnet):
- blocking: sets process-red, so waves pause as on main-red. Process-red is an open issue labelled
  `bosun:process-red` (a blocking lesson carries it). While one is open, `claim` and `spawn` refuse
  work other than `mainfix` and `process-fix` with exit 4, and `wave plan` returns exit 4, except
  for a mainfix or process-fix epic.
- `bosun lesson triage <issue> --as blocking|mechanizable|advice` applies one of the labels
  `bosun:lesson/blocking`, `bosun:lesson/mechanizable`, `bosun:lesson/advice`; a re-triage removes
  the other two.
- mechanizable: opens a kind/process-fix task (`bosun:kind/process-fix` + `bosun:state/ready`),
  which runs before the features of the same area.
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
- `bosun lesson hit <id>` records a hit. `lesson add` and `lesson hit` change the local file `docs/lessons.toml` and need no GitHub; only `lesson hit --issue N`, which posts a `lesson-hit` record, does (and obeys the kill switch).
- At the cap, `bosun lesson add` refuses with exit 6. The curator must then mechanize a lesson,
  merge two lessons, or retire a lesson with no hits in 30 days.
- A lesson that gets a hit after its mechanism merged reopens as blocking. A lesson is mechanized
  when its issue is labelled `bosun:lesson/mechanizable` and closed as `completed`. When
  `lesson capture` finds no open issue with the signature but a mechanized one, it reopens that
  issue, makes it `bosun:lesson/blocking` (the other triage labels go), adds `bosun:process-red`, and comments "The mechanism did not
  hold: seen again. Reopened as blocking" with the `lesson-seen` record; the output has
  `reopened: true`. Issues closed as `not_planned` or `duplicate`, advice lessons and untriaged
  lessons are ignored: a new lesson opens.
- `bosun lesson add --upstream "<text>" [--why ...]` sends a lesson about bosun itself to
  keelstackhq/bosun as an issue (`bosun:lesson`, `bosun:source/lesson`; title cut to 120
  characters), not to the local store.
  It obeys the kill switch; data `{upstream: true, issue}`.
- Metrics: `bosun progress --lessons` (open, mechanized, hits per week). data `lessons: {open,
  blocking, mechanized, advice, hits_last_7_days, hits_by_week: [{week_start, hits}]}` (4 weeks,
  oldest first), counted from `lesson-seen` and `lesson-hit` records.

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
- Generic adapters, for stacks bosun has no built-in adapters for (a repo profile `.bosun/profiles/<name>.toml`): `packages = "none"` (or `"generic"`) means the stack has no packages, so a per-package gate runs once over the whole stack with its `all` arguments; `pending = "regex"` and `assertions = "regex"` read `[adapters.regex]`: `test_files` (globs relative to the stack directory, required), `pending` (regexes; a matching line is a skip marker, and a `UC-…` id on it makes it a plan marker) and `assertions` (regexes; each match is one assertion). Both `pending` and `assertions` are `regex`, or neither, and each has at least one pattern. Profiles without `[adapters.regex]` are unchanged. There is no test counter, so `fewer-tests` does not fire for a regex stack. When any stack's profile uses a generic adapter, test-integrity still runs, and its data and `bosun status` say `"integrity": "reduced"` with `integrity_reason` (for example "stack py/ uses regex adapters", naming the stack path); otherwise `"integrity": "full"`. Adapter names are a closed set (packages: `cargo-metadata`, `pnpm-workspace`, `none`, `generic`; pending: `rust-ignore`, `ts-skip`, `regex`; assertions: `rust`, `ts`, `regex`): any other name is an error with a fix listing them, and `bosun status` shows such a stack as reduced. Every stack gets a kit, and the innermost stack holding a file decides which adapters read it.
- A mixed repo uses one `[[stack]]` per path (`path = "web/"`, `profile = "ts-pnpm"`).
- `unsupported = ["mutants"]` needs a reason and an owner approval (R-gov).
- Packages adapter (rust): a changed root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` or `.cargo/**`, or a changed `.rs`, `.toml` or `.lock` file in no package, means all packages; other files outside every package (docs) affect none.
- A profile may name its stack `manifest` (rust: `Cargo.toml`); a stack whose manifest is missing at the head and at the base skips each gate (exit 0, `skipped: true`, with a `reason`), so an empty repo after `bosun init` stays green.
- A gate may name `requires = "<file>"` (a path in the stack directory); the gate is skipped (`skipped: true`, `reason: "no <file>"`) only when that file is missing at the head and at the base, also with `--all`, like `manifest`: a branch that deletes the file still runs the gate.
- In a gate's `cmd`, an item that is exactly `{packages}` marks where the `each_package` (or `all`) arguments go, for tools whose filters must come first; without it they are appended. The profile's `packages` adapter (see the closed set above) loads the packages and says which a diff affects; an unknown adapter name is an error with a fix.
- ts-pnpm profile: the gates call the workspace's root `package.json` scripts, so a repo chooses its tools: fmt runs `pnpm run format:check`, lint `pnpm run lint`, types `pnpm run typecheck`; test runs `pnpm --filter ...<pkg> run test` for each affected package (`...pkg` selects the package and its dependents, so `data.packages` lists the changed packages and the run covers their dependents too) and `pnpm -r --include-workspace-root run test` for all; deps runs `pnpm audit --prod --audit-level high` and `requires` `pnpm-lock.yaml`. With `--include-workspace-root` the gate runs the root project's own `test` script too, so the root `test` script must not be a recursive `pnpm -r run test` (it would run every package twice). Packages adapter `pnpm-workspace` reads the `packages:` globs of `pnpm-workspace.yaml` (a block list or a one-line list `[a, 'b']`; any other value, or a glob with braces, is an error with a fix) and the directories with a `package.json` that has a `name`. It does not enter dot directories, symlinks or `node_modules`; it skips directories named `target`, `dist`, `build` or `coverage` unless a `packages:` glob matches that directory (`packages/build` is a package when `packages/*` is listed). A changed file inside a package affects that package, whatever its extension. A root `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `.npmrc` or `.nvmrc`, or a file with extension ts, tsx, mts, cts, js, jsx, mjs, cjs, json, yaml or yml in no package, means all packages; other files outside every package (docs) affect none.
- A gate with `each_package` and `when_no_package = "skip"` is skipped (exit 0, `skipped: true`) when no package is affected; the default `when_no_package` is `run`, which runs the gate with its `all` arguments.

Time budgets: a PR run is at most 10 min, a merge queue run at most 15 min, one job at most 8 min.
A gate over budget opens a lesson.

test-integrity:
- Refuses a PR that deletes a test, adds a skip marker, or lowers the assertion count, unless the
  plan says so.
- A snapshot change needs the line `snapshot-change: <reason>` in the PR body.
- Until `--pr` reads the PR and the task metadata, the PR body comes from env `BOSUN_PR_BODY` and the allowed kinds (`delete-test`, `skip`, `only`, `fewer-asserts`) from env `BOSUN_INTEGRITY_ALLOW` (comma-separated). Findings the task allows are reported with `allowed: true` and do not fail. A `#[ignore = "UC-…"]` marker that the base file lacks (by id) is a new skip; removing one is fine. A lower count of `#[test]`, `#[tokio::test]` and `#[rstest]` attributes in a file is a `fewer-tests` finding, allowed by `delete-test`. Files are compared with the merge base, not the base tip.
- ts adapters: test-integrity reads `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs` and `.cjs` files with them, `.rs` files with the rust adapters, and no other file is a test file. Test files are `*.test.<ext>`, `*.spec.<ext>` and files under `__tests__/`. Comments (`//`, `/* */`), regex literals and the inside of string and template literals are ignored; `fit` and `fdescribe` count only when called with a string or template literal title, and a name after `function` or `.` is not a declaration. `ts-skip` finds `it`, `test` and `describe` (also `suite`) declarations with a `.skip`, `.todo`, `.skipIf`, `.runIf` or `.fixme` modifier anywhere in the chain (`describe.skip.each`, `test.concurrent.skip`), and `xit`, `xtest` and `xdescribe`; a new one is a `new-skip` finding. A new `.only` (`it.only`, `test.only`, `describe.only`, `fit`, `fdescribe`) is a `new-only` finding, allowed by `only`. `ts` counts `expect(`, `assert(` and `assert.`, and `it`/`test` declarations (skipped ones too) for `fewer-tests`. Each side of a change is read with the adapter for its own path; a file that was a test file and no longer is one (renamed to `a.test.ts.disabled`) is a `deleted-test` finding.
- Mutants that survive fail the merge queue.

Review verdict: `bosun review-pack <pr>` builds the input (diff, task, UCs, matching lessons,
REVIEW.md). The reviewer returns:

```json
{"pr":42,"head_sha":"abc123","verdict":"approve","confidence":0.9,
 "findings":[{"severity":"major","file":"src/core/lease.rs","line":88,"claim":"...","evidence":"..."}],
 "lessons_hit":["L-012"],"escalate_risk":false}
```

- `bosun verdict post` writes it as the review-verdict check on `head_sha`.
- Cloud sessions cannot write commit statuses (the agent proxy answers 403), so a reviewer there
  runs `bosun verdict comment --from -`. It validates the JSON, refuses a stale `head_sha`
  (exit 3) and posts a PR comment: a summary, `<!-- bosun:verdict -->` and the JSON. The
  `verdict` workflow (`issue_comment` created, comments with the marker only) runs
  `bosun verdict apply <comment-id>` with the release pinned in `.bosun/version` on the default branch (built from it only while the pin says `unreleased`), never PR code. `apply`
  re-reads the comment and refuses it (exit 1) unless it is unedited, its author has write (or
  admin) permission on the repo (`GET collaborators/{user}/permission`; `author_association` is no
  proof, since a workflow token sees an org member as `CONTRIBUTOR`), it sits on the PR its JSON names, and the JSON is valid; a
  `head_sha` that is no longer the head is exit 3, and a comment is refused when a newer verdict
  comment for the same head exists (the workflow also runs one comment per PR at a time). The
  verdict record names the reviewer session from the comment's `<!-- bosun:reviewer … -->` tag,
  which the session writes, so it is an audit hint, not proof. Any member session can post a
  verdict, the PR's author session included, as with `verdict post`. Then it does what
  `verdict post` does: the
  status (as github-actions), the verdict record and the task state. A verdict may come through
  the agent app: it is a reviewer's opinion, not an owner's approval. Permissions of the
  workflow: `contents: read`, `pull-requests: write` (the record comment on the PR; `issues: write`
  alone gets 403 there), `issues: write` (the task's state labels), `statuses: write`; `tests/workflows.rs` asserts them.
- Approve needs confidence >= 0.85 and no blocker finding.
- A later review of the same PR is a delta review (only the new commits).
- `verdict post` also comments `{"bosun":"verdict","session":…,"head_sha":…,"approved":…}` on the PR; the next `review-pack` takes `since_sha` from the newest such record (statuses are per SHA, so they cannot carry it) whose sha has a successful `review-verdict` status (the newest older head that was approved) and is an ancestor of the head (the compare status is `ahead`) and one of the PR's commits (`pulls/{n}/commits`, at most 250), else it is a full review. Any compare error, a sha that is not an ancestor, or more than 20 older heads to check also gives a full review, with `since_sha` null and `delta_fallback` saying why.
- `bosun pr --auto-merge` reads the required contexts from env `BOSUN_REQUIRED_CHECKS` (comma-separated, replaces all), else the defaults `ci-ok,risk-check,review-verdict,main-open` plus any extra entries of `hoist.toml` `[github] required_checks` read from the default branch via the API; a context counts as green when its commit status or its check run is `success`. `bosun pr` refuses the repo's default branch (from GitHub), `main`, `master` and a detached HEAD (exit 2), opens the PR against the default branch, and `--auto-merge` is refused (exit 3) unless the task is in `review`, before any GraphQL call. Auto-merge goes through GraphQL; when a Claude Code cloud session's proxy refuses it (403, "GraphQL is not available from Claude Code sessions"), bosun uses `PUT /repos/{o}/{r}/pulls/{n}/ccr/auto_merge` with `merge_method: squash` and reports `auto_merge_route` (`graphql` or `ccr`); any other error stops as before. The merge-lock hook still blocks agents from calling these routes themselves. `bosun gate` and `test-integrity` take their base from `origin/HEAD` when offline, else `origin/main`; `bosun ask` reads the owners from the default branch.
- `verdict post` moves the linked task (`Closes #N`) to `fixing` when not approved and to `needs-human` when `escalate_risk` is true, if the state machine allows it; an `escalate_risk` verdict is never green (status `failure`, "escalated to the owner"); approval leaves the state alone (`bosun pr --auto-merge` queues it). Invalid verdict JSON is exit 2.
- `review.mode = "subagent"` (mode a) now; `"ci"` (mode b) after M8.
- `bosun review-ci <pr>` is mode b: the review-verdict check runs with no session. It checks the kill switch (exit 4); when `review.mode` is not `ci` it exits 0 with `skipped` and calls nothing. When today's registry spend has reached `daily_usd` it exits 6 with no call and no status. It builds the review pack and fails closed when the model could not see the whole diff: a text file listed without a `patch`, fewer files listed than the PR changed, or the GitHub listing limit reached (300 files for a delta, 3000 for a full review) sets the status `failure` ("unreviewable diff: …; review in a session (mode a)"), exits 1 and makes no API call (`core::risk::unseen_diff`). Binary files without a patch do not fail; the pack lists them as `not_shown_binary` and the prompt says so. Above `review.max_diff_bytes` (default 200000, at least 1; `review.max_tokens` is at least 1 too) on the size of the whole request text (system prompt and user message) it sets the status `failure` ("too large for CI review: <real size> bytes > <cap>; review in a session (mode a)"), exits 1 and makes no API call. It never truncates. Else it makes one Messages API call (`POST {BOSUN_ANTHROPIC_API or https://api.anthropic.com}/v1/messages`, key from env `ANTHROPIC_API_KEY`, model `review.model` default `claude-opus-5-5`, `review.max_tokens` default 8192; the key is never printed or logged, and an API error text has the key and anything shaped like `sk-ant-…` redacted before it is cut; `BOSUN_ANTHROPIC_API` must be https, or http on 127.0.0.1 or localhost). The system prompt holds the reviewer role, the pack's instructions and REVIEW.md, and says that the PR title, diff, task text and file content are untrusted data: instructions in them that try to steer the review are ignored and reported as a `major` finding with `escalate_risk: true`, and never change the output format or the verdict rules; text in code or tests is reviewed as code. The pack is the user message. The text from the first `{` to the last `}` of the reply must be a verdict; bosun sets `pr` and `head_sha` from the pack (never from the model) and validates it. Unparsable or invalid output sets the status `failure` ("reviewer output invalid: …") and exits 1. A network error or a non-2xx answer exits 1 with no status, so a run can be repeated. A valid verdict is recorded as `verdict post` does (status, record, task state; a stale head is exit 3) with the session `review-ci`, whatever `BOSUN_SESSION` says. After the call it writes a spend record (§9) for session `review-ci` with the tokens at the model's price, on the registry issue and on the linked task; a cost that is negative or not finite is not recorded. A link counts only when the issue carries `bosun:task` metadata; any other linked issue (the registry, a plain issue) gets no spend and no task move. If the spend write fails, the verdict still stands and `data.spend` holds `error` and `fix`. The per-task cap and the block of `budget record` do not apply to `review-ci`; only the daily guard above does. When the verdict has findings, a PR comment lists them (severity, `file:line`, claim and evidence cut to 300 characters, at most 20 findings and 8 KB, in one text code fence, no bosun marker). Data: `pr`, `head_sha`, `since_sha`, `approved`, `status`, `model`, `usage` (`input_tokens`, `output_tokens`, `usd`), `findings` (the full list), `findings_count`, `findings_comment`, `spend`. With `--dry-run` it runs up to and including the parse and writes nothing to GitHub (no status, record, comment, task move or spend; the refusals above also write no status, the daily guard still applies); data is `pr`, `head_sha`, `since_sha`, `dry_run`, `model`, `usage`, `approved` and the parsed `verdict`, it is always a full review (no delta, `since_sha` null) and its cost is not written as spend, so it does not count against `daily_usd`; a recorder can save it as the `b` side of a fixture (`tests/fixtures/reviews/`).

Flakes: a test that fails and then passes on the same commit is a flake. bosun opens a kind/flake
task and a lesson. A test is never skipped to get green.

## 8. Risk gating

`.bosun/risk.toml` rules:
- R-gov: "Governance files decide who approves what." Paths `.bosun/**`, `governance/**`,
  `.github/CODEOWNERS`, `.claude/**`, `.splice/**`.
- R-ci: "CI decides what is verified." Paths `.github/workflows/**`, `deny.toml`, `rust-toolchain.toml` and `channel/**` (the CI and governance `bosun init` ships).
- R-secrets: "A secret in the repo is a leak." Content `(?i)(api[_-]?key|secret|token)\s*=`.
- R-deps: "A new dependency adds code that nobody reviewed." `new_dependency = true`.
- R-prod-write: "A write to production cannot be undone by a revert." `capability = "prod-write"`.

Rule evaluation:
- risk-check unions the capabilities of every issue the PR body links (`Closes|Fixes|Resolves #N`, any case, and `<!-- bosun:task-pr N -->`), because the agent writes the body.
- risk-check adds the built-in match `R-unseen` ("bosun could not see the whole diff", so high) when GitHub lists fewer files than the PR changed, or lists a file with changed lines but without a patch (GitHub reports 0 changed lines for a binary file; the file name never exempts a file).
- risk-check evaluates the rules from the base branch, never from the PR head. A PR cannot change
  the rules that judge it.
- A rule only escalates; nothing de-escalates.
- The bosun version pin (`.bosun/version`) is R-gov.
- A rule with several conditions matches when any one does. Content rules read only the lines a PR adds, and their evidence names the file and line number, not the line, so a secret is not copied into the PR body. `bosun risk-check` reads capabilities from the `capabilities` list of the linked task's metadata and from `bosun:capability/<c>` labels on the PR.

High-risk PRs:
- A high PR needs an approval on the head SHA by a user in `governance/owners.toml`, and
  CODEOWNERS. Either counts: a GitHub review that approves the head SHA (the owner's newest
  approve, request-changes or dismissed review on that SHA), or a PR comment by the owner whose
  body, trimmed, is exactly `/bosun approve <head_sha>` with the full 40-character SHA. The
  comment path exists because GitHub forbids approving your own PR, and an owner may also be the
  account that opens the PRs. A comment binds to that SHA: a new push needs a new comment. An
  edited comment does not count (users with write access can edit other users' comments). Only a
  comment written with a plain user token counts: its `performed_via_github_app` is present and
  `null`. Agent sessions act as the same login through the Claude app, and GitHub stamps that
  app on their comments, so an agent cannot approve; a response without the field does not
  count either (fail closed). When
  the owner's newest decisive review on the head SHA requests changes, their comment does not
  count either.
- The `risk` workflow runs on `pull_request_target` (opened, synchronize, reopened, edited), on
  reviews, and on PR comments (`issue_comment` created) that contain `/bosun approve`, so a
  comment re-evaluates `risk-check` without a push. `pull_request_target` and `issue_comment` run
  the workflow file from the base branch, so a PR cannot rewrite the job that judges it. The job
  never checks out or runs PR-head code: it runs the release pinned in `.bosun/version` of the
  PR's base SHA (on a comment, the default branch; built from it only while the pin says
  `unreleased`), so a base that does not compile cannot block its own mainfix, and reads the PR through the API. Event data reaches scripts through `env`
  only. Permissions: `contents: read`, `issues: read`, `statuses: write`, and
  `pull-requests: write` for the Risk note in the PR body. `tests/workflows.rs` asserts all of
  this. Limits: `pull_request_review` has no base-branch variant, so a review event runs the PR's
  copy of the file; and a same-repo PR can add a workflow of its own that requests
  `statuses: write`. Both are R-ci, so an owner sees them, but neither is blocked by bosun.
- bosun writes a Risk note into the PR body: the rules that matched, why, and what the owner must
  check.
- `bosun ask` sends the question to the owner (GitHub mention, and Slack if set). Seed: the GitHub mention only, owners read from `governance/owners.toml` on `main`; the comment carries an `ask` record. There is no
  timeout and no automatic approval.
- A risky task is small: the target is under 100 changed lines.

Kill switch: bosun is halted while either source says so, and then every command that starts or
changes work (including `bosun pr` and `bosun verdict post`) is exit 4:
- An open issue with the label `bosun:halt`, opened by a login in `governance/owners.toml` (read
  from the default branch). An issue opened by anyone else is ignored. Any error reading the
  issues or the owners is exit 4 (fail closed), and so is an open halt issue while no owners
  are listed. Agents must never close a halt issue or remove its label; bosun does not yet
  enforce that (a token with Issues: write can), so against a misbehaving agent the owner also
  sets `BOSUN_HALT=1`, which agents cannot change. The owner creates the label once:
  `gh label create bosun:halt --repo <owner>/<repo> --color B60205`. To halt:
  `gh issue create --repo <owner>/<repo> --label bosun:halt --title "halt" --body "<why>"`; to
  resume, close the issue.
- The repo variable `BOSUN_HALT`, where the token can read it: `1` halts. 404 (unset) and 403 (not
  readable from this token, as in cloud sessions behind the agent proxy) are no signal from this
  source. Any other error reading it is exit 4. Only the owner sets it.

`bosun status` and the SessionStart context show whether bosun is halted; `bosun doctor` step 5
checks that the `bosun:halt` label exists and that the kill switch is readable.

Audit: `bosun progress --risk` lists the high PRs, who approved them, and the time to approval.

## 9. Main-health, cost and spend guard

Main-health:
- The main-health workflow runs on each push to main and nightly. It opens one main-red issue on
  failure and closes it on recovery.
- The required check main-open fails every non-mainfix PR while main is red.
- `bosun gate main-open --pr N` posts the `main-open` status on the PR head: success while main is open or for a mainfix PR; else failure and exit 4. Only the label `bosun:kind/mainfix` makes a mainfix: on the PR, or on a linked issue that is open and not a pull request. A task marker in a body is never trusted (labels need triage rights, bodies do not).
- `bosun main-triage` bisects the merged PRs in code and names the first bad one.
  `bosun main-triage --good REV [--bad REV] [--run CMD]` bisects the first-parent commits between
  good and bad (bad defaults to HEAD) in temporary worktrees. `--run` is a shell command, non-zero
  is bad; the default is `bosun gate test --all`. It prints the first bad sha, subject and PR.
- The default fix is `bosun revert <pr>` as a kind/mainfix PR. A forward fix is allowed only if it
  is smaller.
- `bosun revert <pr>` needs a merged PR (else exit 3). It reverts the merge commit (`-m 1` for a
  merge) on the new branch `bosun/revert-<pr>` from the default branch, pushes that branch (never
  the default branch, never with force), opens a PR "Revert #<pr>: <title>" with the body marker
  `<!-- bosun:revert <pr> -->` and the main-red issue, and labels it `bosun:kind/mainfix`. Main-red
  does not block it. It works in a temporary worktree, so the caller's checkout is never touched. A revert
  conflict is exit 1: revert by hand or make a forward fix. `data.warning` flags a PR with more than
  one commit whose merge commit has one parent (a possible rebase merge: only the last commit is reverted).
- Each main-red opens a lesson.
- `bosun main-health --result R [--run-url U]` does this: `failure` opens the main-red issue (if none is open) and captures a `main-red` lesson; `success` comments and closes it; `timed_out` and `startup_failure` count as `failure`; any other result changes nothing. `success` closes every open main-red issue; two parallel failures keep the oldest issue. On every success or failure it re-judges every open PR (`main-open` status; a failed re-judge makes the exit 1), so queued PRs cannot auto-merge into a red main. Repeated main-red episodes hit the same open lesson (same signature, §6 dedupe). It checks no kill switch: it starts no work.

Cost:
- Usage providers (D20): `claude-sessions` takes the `get_session` result of a session (asked for
  in `next`, passed back on stdin) and reads `usage.cost_usd`; `generic` estimates from turns and
  model with the prices in `docs/defaults.md`.
- `bosun budget record` posts the spend on the task issue and on the registry issue (created if missing), reads the `get_session` result from `--from -` (or a file), accepts `usage.cost_usd` at the top level or under `session`, and sets the task to blocked when the sum of its spend records exceeds `per_task_usd`. The record that blocks the task also opens a lesson of kind `budget` through the lesson capture of §6 (one lesson for all tasks; each further task over its cap adds a hit). A failed capture does not fail the record: it exits 0 with `lesson: {error, fix}`, and the spend must not be recorded again. The record is a `{"bosun":"spend",..}`
  comment.
- The headline metric is cost per merged task.
- `bosun progress [--days N]` (default 7, read-only) reports it for the window of N days ending now: `merged` counts task issues closed with `closed_at` in the window (closed as `not_planned` or `duplicate` does not count), `spend_usd` sums the registry spend records with `at` in the window (unmerged work counts), and `cost_per_merged_task_usd` is `spend_usd / merged` (`null` when none merged); `task_cost_avg_usd` is the mean of the merged tasks' own spend (all time), and `tasks` lists `{issue, title, closed_at, usd}` by issue.

Spend guard:
- Below 80 % of the daily budget: normal.
- At 80 % or more: no new work; mainfix is exempt. `claim` checks it like spawn check 2 (exit 6).
- At 100 %: the janitor stops the sessions.
  The janitor runs `bosun budget check`, which returns one `interrupt_session` per live remote session in `next` (a wave agent `S/task` counts as `S`; never the caller's own session). It writes nothing, also works while halted, and a `daily_usd` of 0 turns it off. An interrupted session that resumes is still refused new work by spawn and claim.
- The day resets at 00:00 UTC.
- A task over `per_task_usd` goes to blocked and opens a lesson.
- Only the owner raises a budget (an R-gov change of limits.toml).
- Spend today is the sum of the `spend` records on the registry issue with `at` since 00:00 UTC (task issues hold the same records, so they are not summed again). `bosun status` reports it with `daily_usd` (from the default branch, as in §5) and its percent of it, and reports `integrity: reduced` when a stack uses a profile other than the built-in `rust`.

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
- `bosun notify [--channel-info -|FILE] [--ack KEY]` lists the posts that are due (halt, main-red, budget-halt, owner-question; `[slack] notify` filters, empty means all). bosun never calls Slack and holds no Slack token (S1): each post comes back in `next` as `{"tool":"slack_post","args":{"channel","text"}}` (D29) and the caller posts it. It works while halted, to announce the halt. Keys are stable (`halt:<issue>`, `main-red:<issue>`, `budget-halt:<date>`, `ask:<issue>:<comment>`); `--ack KEY` writes one `notified` record on the registry issue (idempotent; a key must match `^[a-z-]+:[0-9a-z:-]+$`, else exit 2), and a key with a record is never listed again, so each key is posted at most once per ack; a caller that dies before `--ack`, or two notifiers racing, can repeat one post (S4). `--ack` refuses (exit 3) a key that is not pending now, and a `notified` or `ask` record counts only from a user with write access; the halt variable `BOSUN_HALT=1` is announced as `halt-var:<date>`; a failing section is reported in `data.errors` and does not hide halt and main-red. With posts due, `--channel-info` must carry `{"id","is_ext_shared","is_org_shared","is_shared","guests"}`; bosun exits 5 (nothing to post) when it is missing or malformed, the id is not `[slack] channel`, a shared flag is true, or `guests > 0` (S2). Residual risk: these facts are the caller's, bosun cannot prove them; the owner keeps the channel private, not shared and without guests. Text from GitHub (ask questions) is escaped (`& < >`, `@`) before it enters a post (S8).

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
built from main, so a bad change to bosun cannot approve itself: job `bosun-bin` reads the pin from
the PR's base commit and installs `@keelstackhq/bosun@<pin>` from npm, checking that it reports that
version. A base that pins `unreleased` (the seed, see the seed plan S10) builds bosun from the base
commit instead. Job `pin` checks the pin a PR sets: that release must install and report its version. One
bootstrap exception: when the base has no self-gating CI yet (the PR that adds it, and PRs stacked
on it before it merges), the change builds bosun itself, and CI says so in a warning.

`bosun doctor` reports the 12 steps below as ok or missing (exit 5 with the count if any is missing). It cannot see Routines, so step 11 is missing unless the owner sets `BOSUN_ROUTINES_OK=1`; the three Routine prompts are in `data.routines`. Step 2 counts a login that starts with `claude` as the app; step 4 accepts any non-empty `owner`/`owners` key in `governance/owners.toml`. `bosun doctor --remote` also runs `hoist check --json` (binary from `BOSUN_HOIST`, killed with its process group after 120 s, or `BOSUN_HOIST_TIMEOUT_SECS`) and maps its items to steps: 3 for github settings, rulesets and checks, 5 for release and environments (and github environments, secrets, variables), 10 for npm and crates first publish and trusted publishers. Drift makes a step missing and keeps its local problem and fix. `data.hoist` lists `drift`, `unknown` (items hoist could not read, which fail no step) and `notes` (failed verifies, which fail no step) (D17).

Human checklist (12 steps):
1. Create the repo.
2. Install the Claude GitHub App on it.
3. Run `hoist apply` for branch protection, auto-merge (the merge queue follows in M3: `risk-check` and `review-verdict` are statuses on the PR head, which a queue commit does not carry), and the required checks
   `ci-ok`, `risk-check`, `review-verdict` and `main-open` (D15, D17).
4. Fill `governance/owners.toml`.
5. Run `hoist apply` for the environments and secrets (D17), and create the kill-switch label: `gh label create bosun:halt --repo <owner>/<repo> --color B60205`. The repo variable `BOSUN_HALT=0` is optional.
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

`bosun init --stack <rust|ts-pnpm>... [--plan FILE] [--owner LOGIN] [--source SRC] [--ref REF] [--bosun-version X.Y.Z] [--no-pr]` runs `splice channel add` for the bosun channel on branch `bosun/init` (ref `v<bosun version>` by default; a non-release ref also passes `bosun_version`), keeps an existing `.claude/settings.json` or `splice.yml` (`--exclude`), commits, pushes and opens one R-gov setup PR whose body lists the owner steps. It skips the kill switch (a fresh repo has no labels); `--no-pr` stops after the commit and needs no token. Beside a hull template it passes `own_ci=1` when a workflow already defines a `ci-ok` job, adds the `claude-md` piece unless a splice lock lists `CLAUDE.md`, excludes every existing seed path and `.claude/settings.json` (if that file lacks the bosun hooks, an owner step says to take them from the template that owns it, not to edit it), and appends the missing required checks to an existing `hoist.toml` (`bosun-ok` too with `own_ci`), checked before splice runs and refused when the edit cannot be made safely.

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
