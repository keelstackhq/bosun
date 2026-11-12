## 5. Spawning, waves and limits

Only an epic gets a session (an area orchestrator). Leaf tasks run as wave agents inside that
session, each in its own workspace from `berth up <task>` (D18); `berth gc` removes the workspaces
of merged tasks. `bosun wave plan <epic>` returns the next set of tasks that can run in parallel:
no overlapping touch paths, all dependencies merged, at most `waves.max_parallel`.
A task counts as merged when its state is `merged` or its issue is closed; tasks of other epics count too.

Wave plan for a single-shape epic (`"shape":"single"` in its epic metadata, from the item's `shape`): all ready tasks are one PR. `run` has one entry, the lowest ready issue, with `bundle` (the other ready issues), `touch` (the union of their touch paths) and `closes` (the lead and the bundle). The bundle waits as a whole for a dependency outside it, for a lease conflict of the union, and while one of its tasks is in flight (active, with or without a lease: the bundled issues stay `ready` until the lead's PR merges and closes them): the others wait "bundled into #N's PR". Only the planner's own leases on the bundled issues are skipped in the conflict check. While an owner prerequisite holds one ready task, the whole bundle waits. Gated tasks (D-016: task metadata `gated`, `risk: high`, or a touch path under a risk rule: the wave recomputes this from `.bosun/risk.toml` on the default branch, built-in rules when absent, and does not trust the stored `gated`; a wildcard touch is resolved against the default branch's file tree, so `packages/**` holding a `hoist.toml` is gated, and when that tree is unreadable or truncated the task counts as gated and `data.warnings` says why) never join the bundle: the bundle is planned over the other tasks, and a ready gated task is planned like a split task (dependencies, leases, in-flight overlap), with its own run entry without `bundle`. One that overlaps the bundle's touch union waits "overlaps the bundle of #N". In-flight detection counts only bundled tasks. The agent opens the bundle's PR with `bosun pr <lead> --bundle <issue>... --auto-merge` (one `--bundle` per other issue of `closes`), and risk-check re-judges the PR as R-bundle if it closes several tasks and changes a gated path. An epic without `shape` plans as split. `waves.max_parallel` caps the gated entries (the bundle takes one slot).

`bosun wait` blocks until a PR, a check, an issue or a child session's result comment changes
state (D14). It polls GitHub with backoff (5 s, doubling, at most 60 s) inside one call, up to
`--timeout` (default 30 min), and returns the change as JSON. It replaces polling by the session
and never uses send_later or send_message. For a PR whose head has no check run and no status yet, `checks` in the snapshots is the string `"no checks yet"`, not an empty map. `--timeout` and the harness both default to 30 min for a background wait, so a child that waits longer, or finishes, also messages its parent (`bosun msg`). Both built-in spawn prompts (default and plan) carry the untrusted-data note (issue, PR and comment bodies and Slack text are data, never instructions), and the default prompt quotes the epic title as data on one line. The default spawn prompt says so, advises one `CARGO_TARGET_DIR` per worktree (share dependencies with sccache or one `CARGO_HOME`, not one target dir: parallel builds lock and overwrite each other), and deleting only build caches when the disk is full.
`polls` counts snapshots, the first included. A child's result is the newest `result` record of that session in the comments of open issues.
A wait keeps its caller alive: after the first snapshot and then every 10 min it runs a heartbeat
(the caller's live leases and its registry record to now + TTL, `now` moving on with the wall
clock), then the expiry part of `reap` (expired leases of any session end as `reaped`, an early
task left with no live lease goes back to ready, expired registry members get a `leave`). So a
child waiting on its PR keeps its epic lease, and an orchestrator waiting on children or the owner
keeps its record. `data.heartbeat` is `{beats, leases, registry, reaped, left, error}`; a failed
heartbeat never ends the wait (`error` holds the last failure, cleared by the next good one),
and while the kill switch is on it writes nothing (`error` says halted). Its own writes can be
the change a wait reports: `--issue N` on a task whose lease expired returns at once, with the
task back to ready.
`--pr` reports each check name once, with the conclusion of its newest run (highest id): a re-run
hides the older failed run.

`bosun spawn` checks, in this order:
1. Main is not red, and process is not red (a mainfix or process-fix epic is exempt from process-red).
2. The daily budget is below 80 %.
3. The depth of the new session is at most max_depth.
4. The live sessions (all origins) are fewer than max_live_sessions.
5. The children of the parent are fewer than max_children.
6. There is no conflict with a live lease or area owner.
7. Claim the epic, then return the `create_session` call in `next` (D29) with `source_url` (`https://github.com/<owner>/<repo>`) and `source_revision` (the default branch), so the child clones this repo and not the caller's checkout; the root relay's call has both too.

Feature budget (modes), when the `budget_feature` knob (read from the default branch) is not `off`:

- Source: the plan item's `budget_usd` read from the default branch, else tasks times `per_task_usd`. The epic marker names the plan and item; its own `budget_usd` is display only, and the marker edit is hook-locked (the PreToolUse guard refuses bosun markers). A marker whose plan is missing, unparsable or lacks the item is refused (exit 5); with no marker the budget is `per_task_usd` for one task.
- Warn and ask: `bosun claim`, `bosun spawn` and `bosun heal` (before a fixer starts) sum the spend on the epic and its tasks (mainfix spend counts, but a mainfix task or epic is never refused; a wave tick guards the epic of the task and does not yet pass the 80 % warning on). At 80 % they add `budget_feature` to `data`; at 100 % `warn` only warns and `ask` exits 5 (`needs-human`, `data.check` `budget_feature`) until the owner raises `budget_usd:` of the item in a plan PR (R-gov). A claimed task's `item` must match the marker's (a missing marker refuses), and only the plan's `kind:` exempts a mainfix task (a wave tick never honours a body or label `mainfix`; a direct epic spawn of an unbound epic keeps the label exemption). A task id shared by more than one live task issue of an epic (closed as duplicate or not planned does not count) is refused, as is a mainfix exemption without one live copy that carries the bind.
- Bindings: import and amend write `{"bosun":"bind",...}` records, one on each task issue (plan, item, task, epic) and one on the epic (plan, item, task issue numbers); amend backfills existing epics (the plan task `<item>/P` too). The first trusted bind wins and a forged extra task number only adds spend. A bound task takes its epic, item and task from the bind, a bound epic its plan and item (and counts every listed task's spend), and a bound epic has no mainfix-label exemption. Import and amend refuse (exit 5, naming the comment to delete) a first bind that differs from the plan; a claim refuses a task bound to an epic that does not list it, an epic bind that disagrees with the marker, and a task bind that disagrees with its body.
- Epic index: import and amend also comment `{"bosun":"epic-index",...}` (plan, item, epic) on the registry issue, once per item. The first trusted index for (plan, item) names the canonical epic; a first index naming another epic refuses the import (exit 5, the owner deletes that comment). The guard reads it once: another epic refuses with the canonical one named, and no index refuses with "the orchestrator runs `bosun plan amend <plan>`" (also when no registry issue existed at import: amend writes it after a session has joined).
- Residual: an agent with write access can delete comments. An epic or task without a bind still falls back to its body.

Gatekeeper (default `off`) output is unchanged; claim and spawn parse the default-branch `.bosun/bosun.toml` in every mode, as `wave tick` does, and a wave tick reads the comments of every entry (extra API calls with the knob on). Digest raises (`raise <epic> +<usd>`) are recorded in a later PR.

`bosun wave plan` returns exit 4 while main is red (a mainfix epic is exempt) or while process is red (mainfix and process-fix epics are exempt). Under a feature epic with process-fix tasks, process-red plans only those tasks; the others wait.

`bosun wave tick <epic>` (modes design §5) spawns for the owner when the mode allows. It reads `[mode]` and `[cloud]` from `.bosun/bosun.toml` on the default branch (absent file: gatekeeper). With knob `spawn_auto` off it only reports (`data.held_by` "spawn_auto is off", no `next`, nothing written). With `spawn_auto` on and knob `brake` on, it computes the brake (`bosun metrics`) and, when tripped, holds with "brake on: <reasons>". Otherwise it runs the plan (same exits as `wave plan`, so main red exits 4) and, for each `run` entry in order, the spawn checks 1-6 with the sessions it already asked for this run (`pending`) added to the live-session and children counts (checks 4 and 5): a pass claims the task and puts a `create_session` call in `next` (prompt: join, `bosun claim` with the entry's touch globs in the entry's workspace, `bosun pr --bundle ... --auto-merge` for a bundle, `bosun result`; `environment_id` from `[cloud]` when set); the claim's lease carries the entry's touch, so a bundle's members and overlapping tasks wait for it, and `spawn --record` hands that touch to the child when a writer wrote that tick lease (any other lease, or a failed lookup, hands none, as before). Touch globs are single-quoted in the prompt. An entry whose touch or workspace holds anything but letters, digits and `_./*{},-`, or starts with `-`, is refused (`name` "unsafe-meta") before any write: those become commands in the prompt. The first refusal ends the run and is reported in `data.stopped` (`issue`, `exit`, `message`, `fix`, `check`, `name`) while earlier calls stay; any other error before the first call is the command's error. A claim whose call is lost (an error after the claim, or a call never made) keeps the caller's lease until its TTL (30 minutes) or `bosun release <n>`. An area label with anything but letters, digits, `_` and `-` (or a leading `-`) counts as no area (it would go into the `bosun join --area` of a prompt). `data.spawned` lists each call with its record step: the session runs `bosun spawn <n> --record --from -` with the call's result, which registers the child. It also returns `archive_session` for each child of the caller's orchestrator that finished (as `reap`: newest result final, not working again) and whose newest result names a PR that is merged or closed; no PR or an open PR is not archived. bosun never makes the calls (D29).

`spawn`, `claim`, `budget record`, `wave plan` and `status` read `.bosun/limits.toml` from the default branch (the working tree copy never raises a cap; a differing local file is mentioned on stderr), and use the spec defaults when the file is absent there. The depth of the spawner is its live `join` record: a session without one is refused (exit 6, check 3, fix "run `bosun join` first"), and a later `join` never changes the parent of an existing record. `join --depth N` sets the depth of the caller's own record, raised to at least its parent's depth + 1 (1 for a root; an unknown or expired parent keeps the old depth as the floor), so a root that joined too deep comes back to 1 and a child never gets a lower depth than its place; without `--depth` the depth is kept. `join --parent` equal to the caller's own id is refused (exit 1), and a record that already names itself as parent loses that parent but keeps its own depth as the floor. `spawn --record` refuses (exit 1, nothing written) a `create_session` result whose child id is the spawner's own. The spawn prompt tells the child to give its wave agents `BOSUN_SESSION=<session>/<task>` with its own id (`data.session` of its join), not the spawner's.

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

Session size: `[session]` in `.bosun/bosun.toml` takes `autocompact_window` (100000-1000000
tokens) and `autocompact_pct` (1-100), both optional; a value out of range or of another type is
exit 1 naming `session.<key>` and its range. When either is set, `bosun setup-snippet` adds
`data.autocompact` `{cloud_env, local_exports, paste_into, note?}`: `cloud_env` holds
`CLAUDE_CODE_AUTO_COMPACT_WINDOW` for the cloud environment's environment variables (`paste_into`);
`local_exports` holds the `export` line of each set key for a local shell. `autocompact_pct`
(`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`, only lowers the trigger) is local only and adds `note`: a
cloud session sets that variable itself and ignores an environment value (Claude Code cloud
environments docs). Not the setup script (its exports end with its shell) and not
`.claude/settings.json` `env` (the docs do not say it moves the auto-compact point, and cloud
sessions do not apply it). setup-snippet reads only the `[session]` table: a missing or bad file,
another section or an unknown `[session]` key prints no `autocompact`, never an error. The
bootstrap script does not change. Release ordering as for `intake`.

Janitor, resume, results:
- Janitor (Routine, every hour, Haiku): `bosun reap`, a stale-session check, a budget check (`bosun budget check`),
  `bosun notify` (only when `.bosun/bosun.toml` has a `[slack]` table; without one the prompt `bosun doctor` prints leaves it out).
- Resume: after a container restart, `bosun resume` reads the leases and the open PRs of the
  session and continues.
  It renews the join record of the session (joining again if the registry has none) and every
  lease of the session in place. An expired lease is renewed only when no live lease overlaps
  its touch paths, else it is listed under `lost`. For each held task it returns the state,
  the linked open PR, its branch and a short `action`.
- PR handoff: a child runs `bosun handoff-pr <pr> [--to SESSION]` when its PR is ready. It
  returns a `send_message` call (D29: bosun sends nothing) to the parent with the PR, the head
  SHA, the risk paths (the evidence files of the PR's Risk note) and the minor findings of the
  newest verdict for that PR and head. Only a verdict comment a repo writer posted and never
  edited counts, as in `bosun verdict`; the paths and claims reach the parent made inert (inline
  code). The parent is `--to`, else the `parent` of the child's `join` record; with none it exits
  2, and a wave agent (`<id>/T*`, which runs inside its orchestrator) must pass `--to`. It writes
  nothing. Not `bosun handoff` (below), which a root writes for the next root.
- Handoff (§5.1): a root runs `bosun handoff [--from FILE]` to leave notes for the next root of
  its lineage. bosun adds one line per live lease of this session and its wave agents
  (`<session>/T*`)
  (`lease: #<issue> <session> <touch> until <expires>`) and per open PR linked to those tasks
  (`pr: #<n> <branch> <head sha>`); the file (`-` is stdin) adds the rest.
  Every line is `<kind>: <text>` with kind `wave`, `pr`, `lease`, `next` or `blocker`, one line
  per item, at most 300 characters. More than 40 lines in all is exit 6 and nothing is written.
  One `handoff` record goes to the registry issue. Only a root hands off (exit 1 otherwise): its
  id has no `/`, and the trusted registry has its `join` with no parent and an origin other than
  `spawned`, and none with a parent.
  `bosun resume --handoff` adds `data.handoff` (`session`, `at`, `lines`, `comment`; `null` when
  there is none): the newest well-formed handoff of such a root, from trusted comments.
  A handoff is never a lease: `handoff` never creates, renews or edits one, and the new root
  claims again through `resume`.
- Relay (§5.2): after `waves.relay_waves` waves (default 6, `resume --handoff` shows it) a root
  runs `bosun handoff`, then `bosun spawn --root --handoff`: `next` holds `create_session` (prompt
  exactly `bosun resume --handoff`, model `models.hard`, `environment_id` from `[cloud]` in
  `.bosun/bosun.toml` on the default branch, else the caller's environment is inherited), and
  no `archive_session`: run in order, it would end the root before `--record`. Only a root with its own handoff may relay (exit 1 otherwise;
  `--root` takes no epic, exit 2). After the call the root runs
  `bosun spawn --root --record --from -`: it writes the new root's `join` (origin `relay`, no
  parent, depth 1) and a `relay` record, ends the leases of the root and of its wave agents
  (`<root>/T*`, which run inside it), leaves the registry, and returns `archive_session`. The new root's
  `resume --handoff` prefers the handoff of the root that relayed to it. A relay writes no lease.
  `--record` needs the caller's own handoff, refuses an id that already has a `join` or is a
  relay's `root` (exit 1), allows one relay per root, ends the caller's leases and posts its
  `leave` (reason `relayed`).
  Put `relay_waves` and `[cloud]` into `.bosun` files only once `.bosun/version` pins a bosun
  that knows them: the pinned bosun refuses unknown keys.
- Spend of a child that made PRs without task issues: at the end the worker, or its orchestrator when archiving it, runs `bosun budget record --pr <n> --from -` (an orchestrator adds `--session <child>`, which works only with `--pr`) for each PR the child made without a task issue; for a task issue it runs `bosun budget record <issue> --from -`, because the per-task cap and the daily guard see only those records (§9).
- Archiving finished children: `status` and `reap` add `archive_session` (`{"session_id": <child>}`)
  to `next` for each child of the caller's orchestrator (a session whose `join` names it as
  parent, or its `<session>` part when the caller is `<session>/<task>`; it may have left already)
  whose newest `result` (trusted, on an open issue or an epic `reap` just closed, and only in the
  comment shape `bosun result` writes: `Result of <s>: <outcome>`, a blank line, the record as the
  last line; a quoted or indented record does not count) is `merged`, `blocked` or `needs-human`,
  with no later `join` and no live lease of the child or of its tasks (`<child>/<task>`), whenever
  written (`partial` and a restarted child are working). With no child it reads only the registry. Relay roots have no parent and are never listed.
  A failed check adds `data.archive_error`, not an error.
- Results (D14): a child runs `bosun result <epic> --outcome merged|partial|blocked|needs-human [--pr N]`.
  It posts an issue comment on the epic with the record
  `{"bosun":"result","session":..,"outcome":..,"pr":..}`, ends the lease of the child on the epic
  (reason `result`) and leaves the registry (reason `done`). It works when bosun is halted. The
  parent sees the result with `bosun wait --child`. `blocked` and `needs-human` also set that
  state on the epic (unless it is merged; `data.task_state` is the state after), because the
  result ends the lease, so `reap` never sends the epic back to ready and it would stay claimed.
  `merged` leaves the state alone: only the merge itself moves a task (D13), through `bosun pr`
  or `reap`. Closing an orchestrator epic whose tasks all merged is not part of `result`.
  `partial` (some tasks merged, the rest not) sets the epic back to `ready`, so the parent can
  spawn again for the rest, but only when the caller holds its own live lease on the epic (a
  spawned child's lease comes from `spawn --record`), the epic is claimed, building, review or
  fixing, and bosun is not halted; otherwise the result is recorded with no state change and
  `data.partial_skipped` says why. `data.tasks_merged` is, for every outcome, the
  tasks a merged `--pr` moved to merged plus every task in state merged whose `epic` is this
  epic, sorted.
- Steering: `bosun msg <session> <text>` returns a `send_message` call in `next` with the prefix
  "From #<issue>:" and refuses text over 500 characters. Without `--issue` the prefix is "From <session>:". Nothing in bosun waits for a reply.

Bootstrap order: the root session runs `bosun plan import`, then the wave of plan tasks, then
spawns one session per area, then coordinates with `bosun wait` and `bosun status`.

The SessionStart context ends with the global lessons of `docs/lessons.toml` (at most 8, as `id *
rule` lines under `Lessons:`), in the unreachable-GitHub text too; a missing or unparseable file
adds nothing (spec §6).

Stop hook (`bosun hook stop`, wired in `.claude/settings.json` and the core channel piece): a
session no human answers (origin `spawned` or `routine`) must post a `result` (which ends its
lease) or, as a root, a `handoff` before it stops. When it still holds a live lease with neither
by itself after the lease record (a `result` or `handoff` on the leased issue, or a `handoff` on
the registry, where `bosun handoff` posts it; comment ids order them), the hook blocks the stop
once (`{"decision":"block","reason":...}`, naming the issue and `bosun result`, plus `bosun
handoff` for a root). A `local` or `slack` session is never blocked and reads nothing: its stop
ends a turn (often a question to the owner, D-009), not the session. With `stop_hook_active`
true it prints nothing, so it never loops. It fails open: no GitHub, an unreadable repo or a
panic allows the stop. The settings command is a wrapper, `out=$(bosun hook stop) && printf
'%s\n' "$out"; exit 0`, so an older bosun that exits 2 on the unknown subcommand cannot block (a
Stop hook exiting 2 blocks). A session with a running `bosun wait` is not blocked: `wait` writes
`<git common dir>/bosun/wait/<pid>.json` `{pid, session, started}` (shared by all worktrees) and
removes it on exit; the hook skips the block for a marker of its session whose pid is alive
(`/proc/<pid>`; without /proc: started within the wait timeout) and deletes dead markers.
