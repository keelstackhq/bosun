## bosun: agents in this repo

Agents write all the code here. bosun owns the process: you call a `bosun` command and act on
the one JSON object it prints (`ok`, `exit_code`, `data`, `error.fix`, `next`). Exit codes:
0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit. Do what `error.fix` says;
never work around a refusal.

### The loop for one task

1. `bosun status` (after a container restart too: it shows your leases).
2. `bosun claim <issue>` — exit 3: another session holds an overlapping lease; pick another task
   (`bosun wave plan <epic>`).
3. Work in your own workspace (`berth up <task>`) with `BOSUN_SESSION=<session>/<task>` set; run
   `bosun heartbeat` at least every 20 minutes.
4. Test first. Before every push run what CI runs, locally:
   `for g in fmt lint test deps test-integrity ci-minutes; do bosun gate "$g" || break; done`
   (a passing gate stamps the tree; step 8 needs the stamps). CI minutes are scarce: push once
   per PR, done and green locally; do review rounds locally and push their fixes together; never
   edit the PR body or title just to retry CI. Push as its own command
   (`git push -u origin <branch>`).
5. `bosun pr <issue>` — after the push: opens or reuses the draft PR, moves the task to review.
6. Review locally: a fresh subagent reads `bosun review-pack <pr>` and returns the verdict JSON;
   post it with `bosun verdict post --from -` (cloud session: `bosun verdict comment --from -`).
   Fix request-changes findings, review again.
7. `bosun risk-check <pr>` runs in CI. A high-risk PR waits for an owner: `bosun ask <issue> "<question>"`.
8. `bosun pr <issue> --ready` once the gates are green locally on the pushed head and the review
   is clean: it marks the PR ready and turns on auto-merge; do not wait for checks. `missing_gates`
   (exit 1) or an unpushed head (exit 3) change nothing: run what the fix says. `data.ready` =
   `pending`: make the `next` call (`mcp__github__update_pull_request`), rerun. Exit 5 with
   `already_clean`: hand the PR to the orchestrator (D-008). `bosun hold` and `bosun ask` put the
   PR back to draft. You never merge, tag or force-push (hooks refuse it).
9. `bosun release <issue>` if you stop before the task is done.

### Sessions

- An orchestrator starts a child only with `bosun spawn <epic>`, makes the `create_session` call from
  `next`, then runs `bosun spawn <epic> --record --from -` with the result.
- `bosun wait --child <session>` waits for a child's result; `bosun msg` returns a `send_message`
  call. `bosun wait --pr N --once` returns at once; `--full` lists everything.

### Rules

- Change `bosun:state/*` labels only through bosun commands.
- Never append to CHANGELOG.md or docs/progress-done.md (agents never read it): add a one-line
  fragment `changelog.d/<name>.md` or `docs/progress.d/<name>.md` (name: the branch with `/` as
  `-`). Before a release, a docs PR to main runs `bash .bosun/fold-fragments.sh`.
- Run `git checkout -b <branch>` as its own command, before any command that commits.
- Stay inside your task's `touch:`. Put new tests in a new file; on a lockfile conflict take
  main's copy and regenerate it (`git checkout origin/main -- <lockfile>`, `cargo update -w` or
  `pnpm install`). Planning rules:
  https://github.com/keelstackhq/bosun/blob/main/docs/plan-format.md#cutting-tasks-for-parallel-work.
- Never skip, delete or weaken a test to get green; test-integrity refuses it.
- Human-only steps (secrets, settings, accounts, keys) go in the plan's `## Prerequisites`
  (https://github.com/keelstackhq/bosun/blob/main/docs/prerequisites.md); the orchestrator runs
  `bosun prereqs <plan> --post` once. Never wait for a human: `bosun hold <issue> "<step>"` and
  take other work. Never tick, edit or close the `bosun:prereqs` issue.
- Owner approvals (D-012): ask with a chat form (`AskUserQuestion`: repo, PR, head SHA; approve
  or hold); a child sends the request to the orchestrator, which posts the signed
  `bosun approve <pr>@<head> --sign-key <pem>`. Never ask the owner to write a GitHub comment
  (their `/bosun approve <sha>` is only a fallback). Batch with `bosun approve --list`; only the
  latest message (`data.message_pairs`) is answerable.
- The PreToolUse hook refuses destructive commands (`rm -rf /` or `~`, `mkfs`, `dd` to a device,
  `terraform destroy`, `DROP DATABASE`) and the same Bash command 5 times in a row: change approach.
- bosun's rules and reasons: https://github.com/keelstackhq/bosun/blob/main/docs/spec.md.
