# Agents in this repo

Agents write all the code here. bosun owns the process: you call a `bosun` command and act on
the one JSON object it prints (`ok`, `exit_code`, `data`, `error.fix`, `next`). Exit codes:
0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit. Always do what
`error.fix` says; never work around a refusal.

## The loop for one task

1. `bosun status` — see sessions, leases, halt and main-red.
2. `bosun claim <issue>` — exit 3 means another session holds an overlapping lease; pick
   another task (`bosun wave plan <epic>`).
   When the run entry has a `bundle`, the whole item is one PR: claim the lead issue with
   `bosun claim <lead>` and one `--touch <glob>` per glob of the entry's `touch`, do every task
   of the bundle, then open the PR with `bosun pr <lead> --bundle <issue> ... --auto-merge` (one
   `--bundle` per other issue in `closes`): its body gets a `Closes #N` line for each and the
   marker; run it again later and it reuses the PR (by branch, or by the marker with `Closes
   #<lead>`; a PR made by hand needs a `Closes` per bundled issue). Gated tasks (`risk: high`
   or touching a risk path) run alone in their own entry and are never bundled; a bundled
   issue is not claimable while the lead holds the bundle, and `--touch` never adds a risk path to it.
3. Work in your own workspace (`berth up <task>`) with `BOSUN_SESSION=<session>/<task>` set, so
   your lease is told apart from your orchestrator's and from other wave agents'. Run `bosun heartbeat` at least every
   20 minutes while you work.
4. Test first. Before every push run what CI runs, locally:
   `for g in fmt lint test deps test-integrity ci-minutes; do bosun gate "$g" || break; done`
   (a passing gate leaves a stamp for the tree it ran on; step 8 needs them).
   CI minutes are scarce: push once per PR, when it is done and green locally; do review
   rounds locally (step 6) and push their fixes together. Do not edit the PR body or title
   just to retry CI.
5. `bosun pr <issue>` — right after the push it opens or reuses the PR as a draft (no CI runs
   on a draft) and moves the task to review.
6. Review locally: a fresh subagent reads `bosun review-pack <pr>` and returns the verdict JSON, listing
   every chunk number of the pack in `chunks_reviewed` (a missing one is refused); post it
   with `bosun verdict post --from -` (in a cloud session, which cannot write statuses:
   `bosun verdict comment --from -`). Fix request-changes findings, then review again.
7. `bosun risk-check <pr>` runs in CI. A high-risk PR waits for an owner; ask with
   `bosun ask <issue> "<question>"`.
8. `bosun pr <issue> --ready` once the gates are green locally on the pushed head and the
   review is clean: it marks the draft ready and turns on auto-merge; do not wait for the
   checks. GitHub merges it when the required checks are green. Missing gates (exit 1,
   `missing_gates`) or an unpushed head (exit 3) change nothing: run what the fix says. If
   `data.ready` is `pending`, make the `next` call (`mcp__github__update_pull_request`) and
   rerun. If bosun exits 5 with `already_clean`, the PR was already mergeable: hand it to the
   orchestrator, which merges it with `bosun merge <pr>` (D-008, D-019). `bosun hold` and `bosun ask` put the task's PR back to draft. You never merge, tag or force-push (the
   hooks refuse it).
9. `bosun release <issue>` if you stop before the task is done.

## Sessions

- An orchestrator starts a child only with `bosun spawn <epic>`, makes the `create_session`
  call from `next`, then runs `bosun spawn <epic> --record --from -` with the result.
- `bosun wait --child <session>` waits for a child's result; `bosun msg` returns a
  `send_message` call for steering.
- After a container restart, read your leases with `bosun status` and continue.

## Rules

- Every commit carries `Bosun-Session`, `Bosun-Task` and `Bosun-Model` trailers: `bosun trailers install` sets the hook up (the SessionStart hook does it). `bosun trailers list <session>` finds a session's commits for rollback.
- Change `bosun:state/*` labels only through bosun commands.
- Never append to CHANGELOG.md or the Done list of docs/PROGRESS.md in a PR: add a one-line
  fragment `changelog.d/<name>.md` or `docs/progress.d/<name>.md` instead (name: the branch with
  `/` as `-`), so merging main never conflicts. Before a release, a docs PR to main runs
  `bash scripts/fold-fragments.sh`; the release PR then moves Unreleased under the version.
- Run `git checkout -b <branch>` as its own command before any compound command that commits: the merge-lock reads the branch before the whole command runs, so `git checkout -b X && git commit` on the default branch is refused.
- Stay inside your task's `touch:`. Put new tests in a new file instead of appending to a
  shared one, and on a lockfile conflict take main's copy and regenerate it
  (`git checkout origin/main -- Cargo.lock` then `cargo update -w`), so parallel PRs never
  conflict. Planning rules: "Cutting tasks for parallel work" in `docs/plan-format.md`.
- Never skip, delete or weaken a test to get green; test-integrity refuses it.
- Lessons are not in this file. A review pack carries the lessons that match your files.
- Read the spec section you touch: `docs/spec/NN-*.md`, one file per section (index `docs/spec.md`).
- Front-load human steps: a plan lists every step only a human can do (secrets, settings,
  accounts, keys) in `## Prerequisites` (docs/prerequisites.md). Before work starts the
  orchestrator runs `bosun prereqs <plan> --post` and relays `data.message` to the owner once.
- Never wait mid-run for a human: run `bosun hold <issue> "<step>"` and take other work. Never
  tick, edit or close the owner checklist issue (`bosun:prereqs`).
- Owner approvals (D-012): the session asks the owner with a form in the Claude Code chat
  (`AskUserQuestion`: repo, PR, head SHA; approve or hold); a child session sends the request to
  the orchestrator. On approve the orchestrator runs `bosun approve <pr>@<head> --chat --question <id>` (the chat record
  for that head; a signed `--sign-key` form still works). Only an orchestrator posts it; never
  write the `bosun:chat-approval` marker yourself. Never ask the owner to write a GitHub
  comment; their own `/bosun approve <sha>` comment is only a fallback. With `[ci] provider =
  "none"` the orchestrator merges with `bosun merge <pr>`.
- Orchestrators relay approvals in batches: `bosun approve --list` gives `data.message`; keep its
  `data.message_pairs`, only the latest message is answerable, numbers come only from the owner.
- The PreToolUse hook refuses destructive commands (`rm -rf /` or `~`, `mkfs`, `dd` to a device,
  `terraform destroy`, `DROP DATABASE`) and the same Bash command 5 times in a row (it warns at 3):
  change the approach instead of retrying.
