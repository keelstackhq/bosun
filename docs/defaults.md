# Defaults and their evidence

Each default value of bosun, why it has that value, and how sure we are. "Estimate" means no
measured data exists yet; spec Q3 and Q4 track the review after M4 and M8. A change to a default
changes this file in the same PR.

## Work units

| Value | Default | Evidence |
| ----- | ------- | -------- |
| Task size | no cap, except gated tasks: at most 400 lines (plan error); drift note at 2x planned | D-016: the shape follows the goal. A PR over twice the planned `size:` of its tasks (tests and lockfiles excluded) gets a drift note in `review-pack`. |
| Review chunk | 40000 patch bytes per reviewer subagent | Estimate: keeps each subagent's diff well inside its context. |
| Risky task size | under 100 changed lines | A high-risk PR waits for a human. A small diff keeps the owner's check short and a revert cheap. Estimate. |
| Critical path per item | at most 4 tasks | With 30-60 min per task, 4 sequential tasks keep an epic inside one working day; a longer path is a sign the item is two items. Estimate. |
| UC per "Done when" line | 1 | One line, one test, so `bosun accept` is a mechanical check. |

## Leases and stop rules

| Value | Default | Evidence |
| ----- | ------- | -------- |
| Lease TTL | 30 min | Long enough for one build step between heartbeats; short enough that a dead session blocks a task for at most 90 min (TTL plus one hourly janitor run). |
| Janitor interval | 1 hour | Routines run every hour (first-user feedback F10). Haiku, so the cost is small. |
| Same failure signature | 3 times | One retry can be bad luck and two can be a flaky test; the third identical failure means the agent is stuck. Estimate. |
| Fix round with 0 changed lines | blocked at once | A round that changes nothing cannot fix the failure; another round costs money for the same result. |
| Tier escalation | haiku -> sonnet -> opus -> needs-human | A stuck task often needs a stronger model, not more turns. The orchestrator brief for this repo uses the same tiers (haiku to read, sonnet to build, opus for design and final review). |

## Sessions and waves (`.bosun/limits.toml`)

| Value | Default | Evidence |
| ----- | ------- | -------- |
| `sessions.max_depth` | 2 | Root -> area orchestrator. Child sessions could not relay merges or schedule messages on 2026-10-22 (D13, D14), so a third level would have no reliable path back. |
| `sessions.max_live_sessions` | 4 | Estimate: 3 areas plus the root. Each live session holds a container and a lease set. |
| `sessions.max_children` | 3 | max_live_sessions minus the root. |
| `waves.max_parallel` | 6 | Estimate. Each wave agent gets a berth port block of 10 ports and its own database; 6 agents fit the default berth port window and a 4-core container. |
| `waves.relay_waves` | 6 | Estimate (spec §5.2). About the number of waves after which a root's context nears the point where reports are summarized; the root then relays to a fresh root. Add the key to `.bosun/limits.toml` only once `.bosun/version` pins a bosun that knows it (the pinned bosun refuses unknown keys; the same holds for `[cloud]` in `bosun.toml`); 0 is refused. |
| `turns.build` / `build_cap` | 100 / 150 | Estimate; no turn counts were recorded yet. The cap gives one extension of 50 %. |
| `turns.review` / `fix_round` | 40 / 40 | Estimate. A review reads the diff, the task and the UCs once. |
| `models.*` | read haiku, build sonnet, review sonnet, hard opus | The cheapest model that does the job; opus only for design review and stuck tasks. Same split as the orchestrator brief. |
| No `[1m]` models | never passed | A 1M-context model costs more per token, and a task with one context pack does not need it. |

## Money (`[budget]`)

| Value | Default | Evidence |
| ----- | ------- | -------- |
| `budget.daily_usd` | 150 | Estimate (Q3). About 4 sessions working a full day on sonnet. Review after M4 with real `usage.cost_usd` data. |
| `budget.per_task_usd` | 25 | Estimate (Q3). Several times the expected cost of one 400-line task, so only a stuck task reaches it. |
| Spend guard | 80 % no new work, 100 % stop | 80 % leaves room for the tasks already running to finish and for mainfix work. |
| Day boundary | 00:00 UTC | One clock for all sessions and Routines. |
| `generic` provider prices | in `.bosun/bosun.toml [prices]` | The estimate needs per-model token prices. S9 fills the defaults from the published price list on the day it lands and records the date here. |

`generic` provider estimates (`src/sessions/usage.rs`):

| Value | Default | Evidence |
| ----- | ------- | -------- |
| Input tokens per turn | 20,000 | Estimate; replace with claude-sessions data after M4. |
| Output tokens per turn | 1,500 | Estimate; replace with claude-sessions data after M4. |
| Price haiku | $0.10 in / $0.50 out per MTok | Claude Haiku 5.5, prompts up to 100K tokens. Source: claude-api skill model table, cached 2026-10-06. |
| Price sonnet | $2.00 in / $10.00 out per MTok | Claude Sonnet 5.5. Same source and date. |
| Price opus | $4.00 in / $20.00 out per MTok | Claude Opus 5.5. Same source and date. |

A model name matches a built-in price when it contains `haiku`, `sonnet` or `opus`; a `[prices]` entry with the exact model name wins.

## Verification

| Value | Default | Evidence |
| ----- | ------- | -------- |
| Approve confidence | >= 0.85 | Estimate (Q4). Below it, a second look is cheaper than a bad merge. Review with mode (a) data. |
| Chat approval (`[approvals] chat`) | true (on) | Owner decision (plan 2026-11-10): one chat answer approves from the first PR, no key. Set `false` to ignore chat records. Known from release 0.12.0. |
| Mode (b) agreement | 18 of 20 | Estimate (Q4). 90 % agreement on recorded PRs before the CI review may replace the subagent. |
| PR run budget | 10 min | A wave of 6 agents waits on CI; 10 min keeps a fix round under an hour. Estimate. |
| Merge queue budget | 15 min | It runs the full test suite and mutants; one queue entry per 15 min still merges a day's tasks. |
| One job budget | 8 min | A job over 8 min should split; the PR budget then still holds with jobs in parallel. |
| Snapshot change marker | `snapshot-change: <reason>` | Makes an intended snapshot change visible to the reviewer and to test-integrity. |

## Lessons

| Value | Default | Evidence |
| ----- | ------- | -------- |
| Store cap | 40 entries | Each context pack carries the matching lessons. 40 short entries stay under about 4k tokens even when all match. |
| Global cap | 8 entries | A global lesson goes into every pack; 8 keeps the fixed cost small. |
| Rule cap | 160 characters | A lesson shows as one line in every claim and pack; 160 characters is about 40 tokens, so 40 entries stay near 1.6k. |
| Retire after | 30 days without a hit | A lesson that never fires costs tokens and gives nothing. |

## Slack

| Value | Default | Evidence |
| ----- | ------- | -------- |
| `may_build` | false | A thread is for asking and requesting; code comes from spawned sessions with the full checks. |
| Message length | at most 500 characters | Steering messages must stay short; long context goes into the issue. |
