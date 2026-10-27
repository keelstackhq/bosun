## 9. Main-health, cost and spend guard

Main-health:
- The main-health workflow runs on each push to main and weekly. It opens one main-red issue on
  failure and closes it on recovery.
- The required check main-open fails every non-mainfix PR while main is red.
- Both workflows run the bosun release pinned in `.bosun/version` of the base commit (main-open, `pull_request_target`) or the default branch head (main-health), never PR code; while the pin lacks the command they run a shell fallback that posts the same status or issue. The main-open fallback is stricter than `bosun gate main-open`: it reads only the PR's own labels from the event, not linked issues. The main-open job is named `judge`; `bosun gate main-open` posts the `main-open` status. main-health ignores `workflow_run` events that are not pushes of this repo.
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
- `bosun main-health --result R [--run-url U]` does this: `failure` opens the main-red issue (if none is open) and captures a `main-red` lesson; `success` comments and closes it; `timed_out` and `startup_failure` count as `failure`; any other result changes nothing. `success` closes every open main-red issue; two parallel failures keep the oldest issue. On every success or failure it re-judges every open PR (`main-open` status; a failed re-judge makes the exit 1), so queued PRs cannot auto-merge into a red main. Repeated main-red episodes hit the same open lesson (same signature, §6 dedupe). On `success` it also checks the queued tasks (auto-merge on). Review tasks are out of scope: `bosun wait --pr` does not report conflicts (CI does not run on a conflicting PR, so the wait times out). For each open `bosun:state/queued` issue it takes the PR of the newest `task-pr` record (§4) from a writer (never a PR found through a body link) and reads that PR, which must be open and link the task. One whose `mergeable_state` is `dirty` gets one `{"bosun":"conflict","pr":N,"head_sha":..,"base_sha":..}` record (no session) on the task and one comment on the PR (head and base sha, "merge main into <branch>, resolve, push; auto-merge stays on"), which wakes the session subscribed to the PR. Both are posted once per PR and head sha (the base sha moves with every update of main, so it is only information in the record), judged by the record (any user's record counts: a forged one only suppresses the comments). The task state does not change: auto-merge lands the PR once the branch is fixed and `reap` syncs queued to merged. `data.conflicts` lists `{pr, issue, posted, fix}`. GitHub computes `mergeable_state` lazily: `null` and `unknown` are skipped, never retried, and listed in `data.skipped_unknown`, so a conflict can be seen one run late. A failure while checking a task goes to `data.conflict_errors` (`{issue, error}`; a failed issue listing has no `issue`); it does not stop the other tasks and never changes the exit code. A `failure` run makes none of these calls. It checks no kill switch: it starts no work.

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
