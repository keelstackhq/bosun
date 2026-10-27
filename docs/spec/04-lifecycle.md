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
| building, review, fixing | merged | a human merged the PR; `bosun pr`, `bosun reap` or `bosun result --outcome merged --pr N` sees it |
| any | blocked | stop rule, conflict, budget, a child's `bosun result --outcome blocked` |
| any | needs-human | risk high, owner question, tier escalation used up, a child's `bosun result --outcome needs-human` |

Only bosun commands change the state label. A hook guards against accidental or casual label edits; it is not a sandbox. It fails closed: a Bash command that names a state label passes only when every command in it, nested ones included, is on a read-only allowlist (`bosun`, grep/cat/jq and similar text tools, `git show|log|diff|blame|status` with vouched options, `gh issue|pr list|view|status`, `gh pr diff|checks`, `gh search`, `gh label list`) and no `$` expansion appears outside single quotes; the body of a heredoc with a quoted delimiter is data, so prose that names a label passes. Known misses: a label split by quotes or escapes, built at run time or passed through a file, and repo or user config, or files, written by the same or an earlier command (a redirect whose literal target is in `.git/` or a gitconfig is refused; a globbed target, a command that writes a file operand such as `uniq IN OUT`, and other paths are not). A GitHub tool call is refused when its label fields (or, for a label tool, its whole input) name a state label.

`bosun pr` sets claimed -> building -> review (a PR exists only after the first push). An open PR on the branch that does not link the task is refused (exit 3, before any change). `bosun pr` or `bosun reap` sets a building, review, fixing or queued task to merged once its PR merged into the default branch (auto-merge or a human), and ends the leases on the issue with reason `merged`. `bosun pr` and `result` count only a PR merged into the default branch. `reap` and `result` move a building, review or fixing task only when its issue is closed (GitHub closes it when a `Closes #N` PR merges; an open issue means the work is not done, so an old merged PR linking it is ignored and no PR list is read for it); a queued task counts whatever the issue state. `reap` runs this before expiring leases, so a merged task is never reaped back to ready. `bosun result <epic> --outcome merged --pr N` reads PR N; when it is merged, the tasks it links move to merged, this session's leases on them end with reason `merged`, and `data.tasks_merged` lists them; an unmerged PR changes nothing. Before it turns on auto-merge, `bosun pr --auto-merge` posts `{"bosun":"task-pr","session":..,"pr":N}` on the task; `reap` reads the newest such record and that one PR (it must link the task: open means wait, merged into the default branch means merged). Only a task without a usable record (none, as for older tasks; a PR that is gone, does not link the task, is closed unmerged or merged into another branch) costs the list of all PRs.

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
   The label-lock skips the read-only GitHub MCP tools on an explicit allowlist (`READ_ONLY_GITHUB` in `core::guard`; a tool not on it, even `get_*` or `list_*`, is judged like a write), and treats the body of a heredoc as data: in an unquoted body only the command substitutions (`$( )`, backticks) are judged as commands. Anything the guard cannot parse safely is refused as before: a body with `$name` or `${`, a substitution with a quote, `#`, `<<`, a newline, a backslash or `case`, an unterminated substitution, or a line ending in a backslash (bash joins it with the next line before it looks for the delimiter).
   The merge-lock also checks commands nested in `$(..)`, backticks, `( .. )`, `sh|bash|zsh -c` and `eval` (depth 3), `gh api`, `curl`, `wget` and `http` writes that merge, create refs or releases or set auto-merge, and pushes of version-like refs (`v1.2.3`). Its default-branch rules (commit, merge, rebase, reset, cherry-pick, plain push) read the branch where git acts: the hook's directory moved by a `cd dir` joined with `&&`, `;` or a newline, and by `git -C dir`, `--git-dir` and `--work-tree`; a `cd` counts only when the shell surely runs it here (after the start, `;`, a newline, or in an `&&` chain that reaches the git command; never in a pipeline or a `( .. )` group); a directory it cannot read literally (`$VAR`, `~`, `cd -`), a moved target git cannot read (a failed `cd`), or `GIT_DIR`, `GIT_WORK_TREE` or `core.worktree` count as the default branch. The decrypt-lock runs first on every Bash command, at the same nesting depth, and fails closed: `sops` may only print its version or help or encrypt (`sops FILE` is a decrypting editor, so any other argument is refused, flags in any spelling such as `-decrypt` or `-d=true`; `-e` or `encrypt` counts only when no bare word or unknown flag precedes it and it is not the value of a value-taking flag, so `sops --age -e f` is refused), `age`/`rage` may only print info or encrypt (any decrypt flag, also in a short cluster like `-di`, is refused), and `hoist secrets edit|sync|grant|revoke|rotate|rotate-key` is refused. Every word of a segment is tried as the command word (fail closed: `npx`, `flock`, `strace`, `git bisect run`, `coproc`, `case`/`elif` all run what follows), except in readers (grep, rg, cat, echo, ...), file tools (`mv`, `install`, `tar`, `curl`, ...) and package managers with an install-like subcommand (`brew install`, `cargo test`; not `run`/`exec`/`npx`). Command strings in options are judged too: a word with whitespace, and an option or assignment value that has whitespace or starts with `|` or `!` (`git -c core.pager=..`, `LESSOPEN=..`, `man -P`, `ssh host ..`), is judged as a command; only readers, the message options of git and gh (`-m`, `--title`, `--body`, `-f`, `-q`) are data. `bosun` args are data except the `--run` value of `main-triage`, and `rg --pre` is judged as a command run on a file. Also refused: the key directory `sops/age` (also `age/keys.txt`, and `keys.txt` with `sops`, so globs and split quotes are caught) and a `$SOPS_AGE_KEY` expansion (also `$SOPS_AGE_KEY_FILE`) anywhere in the text, `/proc/*/environ` in a command (a grep or rg pattern excepted, not a file); env dumps (`printenv`, `env`, `set`, `export -p`) in a command that names sops in any case; and a shell or interpreter that reads stdin or runs code (`| sh`, `bash <<< ..`, `python3 -c`, `node -e`, `sh -c` with `{}` or `$`) in a command that mentions sops, age, `keys.txt` or `hoist secrets`. The body of a heredoc with a quoted delimiter is data unless a shell or interpreter is on its line, also inside `"$(cat <<'EOF' ..)"` message arguments. Known misses: scripts on disk, names built at run time, quoting that splits a name; known over-denials: any `sops/age` text (`mkdir -p x/sops/age`, quoted prose) and a quoted option value that names `sops -d`. Secrets stay encrypted for agents: the owner runs `hoist secrets edit`, CI decrypts with `hoist secrets exec`. Branch protection stays the real guard, since a hook cannot see every shell trick.

Leases conflict per issue, not per session: wave agents run inside their orchestrator's session, so a
lease on another issue conflicts even when it has the same session id; only a lease of the same session
on the issue being claimed is the same claim (a re-claim is idempotent). Give each wave agent
`BOSUN_SESSION=<session>/<task>` in its berth workspace; `S/x` conflicts with `S/y`, not with `S/x`.
`wave plan` counts the epic's active tasks that hold a live lease against `waves.max_parallel` and
blocks picks that overlap their touch paths. The edit hook keeps the session rule (an agent may edit
paths its orchestrator `S` leased), a known limit: without per-task ids it cannot tell wave agents apart.
The hook sees only the plain session id, so a session `S` may also edit the paths of leases held under `S/<task>`; `S/x` and `S/y` still block each other.
Claim and spawn re-check after writing: they re-read the comments of the claimed issue and the active
issues, and when a live overlapping lease has an earlier comment (creation time, then comment id) they
post `lease-end` with reason `lost-race`, leave the state alone and exit 3 naming the winner.

Trust in records: a `{"bosun":…}` record in a comment is trusted from any commenter with write access to the repo; bosun does not check who wrote it. Checks that do not rely on records: commit statuses (`ci-ok`, `risk-check`, `review-verdict`, `main-open`), and the files read from the base or default branch (`docs/lessons.toml`, `REVIEW.md`, `.bosun/limits.toml`). A `spend` record with a negative or non-finite amount is ignored. A delta review (`since_sha` of `review-pack`) needs the `verdict` record and a successful `review-verdict` status on that sha (records on heads without one are skipped), and the sha must be an ancestor of the head and a commit of this PR; otherwise the review is full.

Touch paths are repo-relative globs (`*` within a segment, `**` across segments). Two touch lists
overlap when a path of one matches a glob of the other, or two globs can match a common path.

`bosun reap` closes an open epic (marker `bosun:epic`; state ready, planned, claimed or none; no live lease: a spawned orchestrator's epic stays `claimed` after `result`, so it closes once its lease ended) whose tasks are all merged or closed and at least one is merged or closed as completed (a task closed with no reason or as not planned counts as dropped), and whose tasks have not changed for 10 minutes (an amend may be adding one): it gets state `merged`, is closed as completed and is listed in `data.epics_closed`. Open plan issues are listed first; with an unleased epic that has no open plan task, every open issue is listed once and an open task of the epic's item (task metadata `item`, whatever its labels or `epic` field: a hand-made issue `plan import` adopted) keeps the epic open, then all plan issues. This is housekeeping: a failure there is logged to stderr and put in `data.epics_error`, and the rest of the reap stands (exit 0). When `plan import` or `plan amend` leaves such a closed epic with an open task (a new task, a reopened or re-added one; decided from what the run made or found, with no new list, since GitHub's list lags), the epic is opened again as `ready` and listed in `data.reopened` (`data.would_reopen` in a dry run), which also repairs an epic that reap closed mid-amend (the quiet period narrows that race, the next import or amend repairs it).

Intake: intake is the only way that new work enters. `bosun intake` (from Slack, a lesson, a flake
or a human) writes a plan amendment as a kind/plan task. Only a merged plan task makes its tasks
ready. After an amendment merges, `bosun plan amend` imports the new work and closes what the plan dropped.
`bosun intake "<text>" --source slack|lesson|flake|human [--item ID] [--title T] [--plan FILE]` opens one
`kind/plan` issue (state ready, `bosun:source/<source>`, `bosun:area/<area>` of `--item`) that asks for the
amendment of the plan file (default `plans/<project>.md`; a missing file or unknown item is exit 2). An open
issue of the same source with the same title is returned instead (`created: false`).
When `plan import` or `plan amend` imports a new item (no `<item>/P` yet), it looks for items that a merged intake added: an intake issue (`kind/plan`, task `intake`, touching this plan file) that is merged or closed as completed, and the merged PR into the default branch that links it (newest intakes first, until every new item has an owner). The plan file is parsed at the merge commit's first parent and at the merge commit (missing at the parent: no ids; present but unparsable at either, or a merge commit GitHub does not know, which it answers with 404 or 422: that intake is skipped with a note on stderr); the item ids only the later one has were added by that PR, and one counts only while the local item (id, title, area, depends, tasks with title, touch, depends, size, kind and risk, UCs) equals the item in the plan file on the default branch (that file missing or unparsable: nothing counts). Their plan task `<item>/P` is made already merged and closed ("Done by intake #N ... PR #M") and the epic and tasks are promoted on the same run, so no second plan PR is needed. Otherwise (intake open, or its PR did not add the item) the plan task is made `ready` as before.
