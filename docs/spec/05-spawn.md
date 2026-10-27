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
  parent sees the result with `bosun wait --child`. `blocked` and `needs-human` also set that
  state on the epic (unless it is merged; `data.task_state` is the state after), because the
  result ends the lease, so `reap` never sends the epic back to ready and it would stay claimed.
  `merged` leaves the state alone: only the merge itself moves a task (D13), through `bosun pr`
  or `reap`. Closing an orchestrator epic whose tasks all merged is not part of `result`.
- Steering: `bosun msg <session> <text>` returns a `send_message` call in `next` with the prefix
  "From #<issue>:" and refuses text over 500 characters. Without `--issue` the prefix is "From <session>:". Nothing in bosun waits for a reply.

Bootstrap order: the root session runs `bosun plan import`, then the wave of plan tasks, then
spawns one session per area, then coordinates with `bosun wait` and `bosun status`.
