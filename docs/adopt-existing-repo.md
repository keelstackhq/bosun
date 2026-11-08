# Adopt bosun on an existing repo

This guide sets up bosun alone on a repo that already exists: no hull template, no keelstack
provisioning. Do the steps in order. Every command below exists in bosun 0.7.0; check any of them
with `bosun <command> --help`.

What bosun is: one binary that agent sessions (Claude Code) call. It prints one JSON object per
command and keeps all shared state in GitHub issues, labels, comments and PRs. Agents write the
code; bosun decides what may run, checks the gates and risk, and turns on auto-merge. bosun never
merges, tags or force-pushes.

## 1. Before you start (owner, once)

Do these on a computer first; the phone steps come after.

1. **Tools.** `bosun init` runs `splice` (it renders bosun's files), and the repo settings come
   from `hoist`. Install both, plus bosun:
   `npm i -g @keelstackhq/bosun @keelstackhq/splice @keelstackhq/hoist`. A cloud session gets
   them from the setup script (section 9).
2. **Claude GitHub App.** Install it on the repo (GitHub → Settings → GitHub Apps). Cloud
   sessions reach GitHub through it.
3. **Approval key.** Signed approvals need an ed25519 key. It signs for you, but the
   orchestrator session that posts approvals holds it: make it in that session's container (or
   copy the PEM there), never in the repo:

   ```bash
   openssl genpkey -algorithm ed25519 -out owner.pem
   openssl pkey -in owner.pem -pubout -outform DER | tail -c 32 | base64   # the public key
   ```

   You put the public key in `.bosun/approvers.toml` (section 3). Without a key, you approve with
   your own GitHub comment instead (section 6).
4. **Labels.** bosun makes its own labels on first use (`bosun:state/*`, `bosun:registry`,
   `bosun:kind/*` and others). Make the kill-switch label yourself:
   `gh label create bosun:halt --repo <owner>/<repo> --color B60205`.

## 2. Run `bosun adopt` (no splice) or `bosun init`

**Without hull, splice or hoist:** `bosun adopt` writes the bosun files into your worktree and
creates the labels and the registry issue. It commits nothing; you commit and open the setup PR.

```bash
bosun adopt --dry-run                          # see what would change; writes nothing
bosun adopt --owner <your-github-login>        # stacks are detected; or --stack rust|ts-pnpm|other
```

The JSON gives the exact commands in `data.commands`: `git switch bosun-setup 2>/dev/null || git switch -c bosun-setup`, a `git add`
of the paths adopt wrote, `git commit -m "chore: set up bosun"`, `git push -u origin bosun-setup`,
then `bosun doctor`.

Existing files are never overwritten (`kept` in `data.changes`). `--no-remote` needs no token and
skips the labels and the registry issue. `.bosun/adopt.toml` records what adopt wrote; a rerun is a
no-op. `data.owner_steps` lists only the steps still left for you (no owners step once `--owner` wrote the file; no key step once `.bosun/approvers.toml` has a key). Undo it with `bosun uninstall` (section 7).
The rest of this section is the splice path.

### With splice: `bosun init`

From a clean checkout of the default branch:

```bash
bosun init --stack rust --owner <your-github-login>        # or --stack ts-pnpm; repeat for both
```

It adds bosun's splice channel, commits on a branch (`claude/bosun-init` in a Claude cloud
session, else `bosun/init`), pushes and opens the setup PR "chore: set up bosun". It needs a
`GITHUB_TOKEN` that can push and open PRs (in a cloud session the proxy provides it). Useful flags:

- `--plan FILE`: copy your first plan to `plans/`.
- `--no-pr`: commit only, no token needed; the PR title and body come back in the JSON.
- `--bosun-version X.Y.Z`: pin another release (default: this binary's version).

The JSON lists the owner steps that are left in `data.owner_steps`. If init fails, it rolls the
branch back.

**Not for a bosun-only repo:** `bosun init --intake FILE` (the zero-touch path) needs hull piece
names in the intake file, and `bosun intake sign --mode adopt` only signs an intake for the
external provision workflow. Neither is a bosun-only adopt mode, and bosun has no `provision`
command. Use plain `bosun init`.

## 3. What lands in the repo

Seeds are written once and then belong to you; owned files are replaced by each channel update.
A seed file that already exists is kept; init only adds the required checks (and
`merge_queue = false`) to an existing `hoist.toml`.

| File | Kind | What it is |
| --- | --- | --- |
| `.bosun/bosun.toml` | shared | project, stacks, review mode, `[approvals]`, optional `[slack]` |
| `.bosun/limits.toml`, `.bosun/risk.toml`, `.bosun/session-rules.toml` | seed | caps and budget, risk rules, session rules |
| `.bosun/version` | owned | the bosun release your CI runs |
| `.bosun/fold-fragments.sh` | owned | folds changelog fragments before a release |
| `governance/owners.toml` | seed | owner GitHub logins (and Slack IDs) |
| `.github/CODEOWNERS`, `hoist.toml` | seed | code owners; branch rules and required checks |
| `.github/workflows/{bosun,risk,verdict,main-open,main-health}.yml` | owned | section 4 |
| `.github/workflows/splice.yml` | seed | weekly channel update |
| `deny.toml`, `rust-toolchain.toml` (rust), `.npmrc` (ts-pnpm) | seed | stack config |
| `.claude/settings.json` | shared | bosun's hooks (skipped if another template owns the file) |
| `docs/setup-script.sh` | owned | the cloud environment setup script |
| `AGENTS.md`, `REVIEW.md`, `CLAUDE.md` | section | a bosun section in each (CLAUDE.md only if no template owns it) |
| `.splice/channels/bosun.toml` | record | the channel record splice keeps |

Add the approvers file yourself, in the setup PR or a follow-up PR. Approve that PR with your own
GitHub review or `/bosun approve <sha>` comment: no signed approval counts on a PR that changes
`.bosun/approvers.toml`, `governance/owners.toml` or CODEOWNERS. risk-check reads the file from
the default branch only, so a PR can never enroll its own key:

```toml
# .bosun/approvers.toml
[[key]]
id = "owner-2026-11"
ed25519 = "<base64 public key from step 1>"
role = "owner"   # "delegate" counts only on rules with approval = "delegate"; the seeded risk.toml has none
```

If `.claude/settings.json` already existed, init leaves it alone and says so: add the bosun hooks
(`bosun hook session-start`, `bosun hook stop`, `bosun hook pre-tool-use`; copy them from the
channel's `channel/pieces/core/files/.claude/settings.json`) there yourself.

## 4. Workflows that get added

| Workflow | Runs on | Does |
| --- | --- | --- |
| `bosun.yml` | pull_request, push, merge_group | every gate (`bosun gate <name>`) in one job, check `ci-ok` (`bosun-ok` if the repo has its own `ci-ok`) |
| `risk.yml` | pull_request_target, `/bosun approve` comments | `risk-check` status |
| `verdict.yml` | PR comments with the verdict marker | `review-verdict` status |
| `main-open.yml` | pull_request_target | `main-open` status |
| `main-health.yml` | CI runs on main, weekly | opens or closes the main-red issue |
| `splice.yml` | weekly | channel update PRs |

All but `splice.yml` run the release pinned in `.bosun/version`, never code from the PR. Draft PRs fail
`ci-ok` in seconds instead of running the gates, which saves CI minutes; see
[ci-minutes.md](ci-minutes.md).

## 5. Finish setup (owner)

1. Fill `governance/owners.toml` if you did not pass `--owner`.
2. Review the setup PR. While main has no bosun workflows, `risk-check`, `review-verdict` and
   `main-open` cannot run on it: merge it yourself.
3. Then turn on branch protection, auto-merge and the required checks (`ci-ok`, `risk-check`,
   `review-verdict`, `main-open`, plus `bosun-ok` if the repo has its own `ci-ok`). From a
   checkout of main with an admin token: `hoist plan --area github`, then
   `hoist apply --area github --adopt --yes` (`--adopt` only adds, never removes or loosens your
   existing rules).
4. Optional, so splice update PRs run CI: an environment `splice-bot` (deployment branches: main
   only) with the secret `SPLICE_BOT_TOKEN`.
5. Optional: the Routines (janitor hourly, curator daily). `bosun doctor` prints their prompts.

Check the result with `bosun doctor` (add `--remote` to also run `hoist check`). It lists each
step as ok, warn, skipped or missing.

## 6. The first plan and the task loop

Write a plan in the [plan format](plan-format.md) at `plans/<project>.md`. Human-only steps go in
its `## Prerequisites` section ([prerequisites.md](prerequisites.md)). Then:

```bash
bosun plan check plans/app.md            # parse; every error has its line and fix
bosun prereqs plans/app.md --post        # one checklist issue of the human steps
bosun plan import plans/app.md           # epics and tasks as issues (--dry-run first)
bosun wave plan <epic>                   # tasks that can run in parallel now
```

An orchestrator session runs `bosun spawn <epic>` and makes the `create_session` call bosun returns
in `next`. Each worker session then loops:

```bash
bosun claim <task>                       # lease the task's paths; exit 3 if another lease overlaps
# ...write the code and tests, commit, push...
bosun pr <task>                          # opens the PR as a draft: no CI runs on it yet
for g in fmt lint test deps test-integrity ci-minutes; do bosun gate "$g" || break; done   # add types for ts-pnpm
bosun pr <task> --ready                  # needs the pushed HEAD and the stamps; ready + auto-merge
bosun review-pack <pr>                   # input for a fresh reviewer subagent
bosun result <epic> --outcome merged --pr <pr>
```

`--auto-merge` is an old alias of `--ready`. GitHub merges once every required check is green.
A gated PR (`.bosun/`, `governance/`, `plans/`, `.claude/`, `.splice/`, `hoist.toml`, workflows;
so the first plan PR and each weekly splice update PR too) also needs your approval of its head
SHA:

- **Chat form (default):** the session asks you in the Claude Code chat (repo, PR, head SHA). On
  approve, the orchestrator posts the signed comment:
  `bosun approve <pr>@<head> --sign-key <pem>`.
- **Your own comment:** `/bosun approve <full 40-character head sha>` on the PR, from your own
  GitHub login (web or mobile app). A new push needs a new approval.

To stop all work, open an issue with the `bosun:halt` label; every command that starts or
changes work exits 4 until you close it.

## 7. Undo

**Adopted with `bosun adopt`:** run `bosun uninstall --dry-run`, then `bosun uninstall`. It deletes
the files in `.bosun/adopt.toml` whose bytes are still the ones adopt wrote, keeps the ones you edited
(`kept`) and removes the directories it emptied. The manifest stays while labels or the registry
issue are left; `bosun uninstall --labels` deletes the labels adopt created (not one still on an open
issue, which is `in-use`) and closes the registry issue it opened, then removes the manifest. Commit the deletions in a gated PR,
and remove the required checks `risk-check`, `review-verdict`, `main-open` (and `ci-ok` if it was
bosun's) from the branch rules. Without `.bosun/adopt.toml` it exits 1: nothing was adopted here.

**Installed with `bosun init` (splice):** there is no uninstall command. To remove it:

1. Open a PR that deletes the files the setup PR added (see its file list; keep files you had
   before, such as your own CODEOWNERS or hoist.toml), the bosun sections of `AGENTS.md`,
   `REVIEW.md` and `CLAUDE.md`, and `.splice/channels/bosun.toml`. It is a gated PR, so approve
   it.
2. Remove the required checks `risk-check`, `review-verdict`, `main-open` (and `ci-ok` if it was
   bosun's) from the branch rules, or edit `hoist.toml` and run `hoist apply`.
3. Close the `bosun registry` issue and any open task issues; delete the `bosun:*` labels if you
   want.

`bosun revert <pr>` is not an uninstall: it opens a PR that reverts one merged PR.

## 8. Where things are

Design: [spec.md](spec.md). Defaults: [defaults.md](defaults.md). Plans:
[plan-format.md](plan-format.md). Human steps: [prerequisites.md](prerequisites.md).

## 9. Cloud sessions (Claude Code on the web)

1. **Setup script: paste once.** Paste the bootstrap from the README ("Cloud sessions: paste
   once"), or the `data.script` of `bosun setup-snippet`, into the cloud environment's setup
   script (claude.ai → environment settings). You never paste again: before each session it runs
   every checkout's own `docs/setup-script.sh`, which installs pinned Node, pnpm, splice, hoist,
   berth, bosun, the Rust toolchain of `rust-toolchain.toml` and the superpowers and caveman
   plugins. A bosun upgrade is a PR that moves the pins in that file. One line to send a client
   with it: "Paste this script once as the setup script of your Claude Code cloud environment
   (claude.ai/code > Environments > Setup script). You never change it again: each repo's own
   docs/setup-script.sh pins its tools."
2. **No PAT in the environment.** The session reaches GitHub through the Claude GitHub App and
   the agent proxy. The proxy rewrites every GitHub write (app attribution, a Claude footer), so a
   PAT there would not count as you anyway: `bosun approve` with `BOSUN_OWNER_TOKEN` refuses in a
   cloud session (exit 5). Keep `BOSUN_OWNER_TOKEN` on your own computer only.
3. **Review verdicts.** Cloud sessions cannot write commit statuses (the proxy answers 403). A
   reviewer runs `bosun verdict comment --from -` with the verdict JSON; the `verdict` workflow
   checks the comment and posts `review-verdict`. `bosun verdict post` works only outside the
   cloud.
4. **Approvals.** By chat form (section 6), signed with a key in `.bosun/approvers.toml`. Who may
   approve after bootstrap mode is `[approvals] policy` in `.bosun/bosun.toml`: `owner`
   (default), `owner-gated` (the orchestrator, only when R-ci alone matches and no workflow
   changes, e.g. `deny.toml`) or `orchestrator`. Workflow changes always need you. Bootstrap mode
   (D-012) ends on 2026-11-30; init writes `[approvals]` only without `--bosun-version`.
   Autonomous mode (D-017): `policy = "autonomous"` under an older pinned release is a parse
   error (signatures do not count); set it only after pinning a release that knows it. The
   orchestrator then approves PRs that change only `src/`, `tests/`, `docs/` or `changelog.d/`
   (not `docs/agents/`, plans, `*.jinja`, `build.rs`, `docs/lessons.toml`, `docs/setup-script.sh`, nor R-gate code, module roots or `#[path]` changes),
   when every verdict on the exact head is clean and one is from an independent reviewer session, for a PR that
   closes an open task leased by its author. Everything else (workflows, `.bosun`, `channel/`,
   root files such as `Cargo.toml`, secrets, releases, tags, key or owner changes) still needs you.
5. **Permission checks (Claude Code behaviour, not bosun's).** Each cloud session asks for its
   own permissions. An orchestrator's go
   does not carry over to a child it spawns: a child that pushes or opens a PR can wait for your
   go in that child's own session. Open the child in the Claude app when it blocks.

## 10. Claude Tag (Claude in Slack)

Support is thin. What works today:

- **Origin.** `bosun join --origin slack` labels a Claude Tag session; no `BOSUN_ORIGIN` and no
  environment of its own are needed. Only `slack` and `local` sessions may start work from Slack.
- **Seat.** `bosun join --origin slack --slack-channel C… [--slack-thread <ts>]` records where the
  session sits (channel or thread). A plain re-join keeps the seat.
- **Routing.** `bosun notify` reads the seat: a channel seat sends thread posts with
  `send_message` (`to: thread`), a thread seat lists its own thread's results and asks plus the channel posts
  (halt, main-red, budget-halt, unthreaded asks) and answers its thread with `thread_reply`.
  Only a writer's live `join` record counts as a seat. An owner question on a thread task goes to that thread.
  `bosun doctor` (step `claude-tag`) checks the seat and warns about a PAT in a cloud session.
- **Start work from a thread.** `bosun spawn [epic] --slack-thread <ts> --slack-user <U…>`
  runs the 7 spawn checks; the user must be an owner's `slack` ID in `governance/owners.toml`.
  Without an epic, `--request` sends the ask to intake. The thread ts is kept on the lease.
  `[slack]` and `may_spawn` are read from the default branch.
- **Answer a question.** `bosun answer <issue> --slack-user <U…>` records an owner's answer on a
  needs-human task.
- **Posts.** `bosun notify --channel-info -` lists the posts that are due (halt, main-red,
  budget-halt, owner-question, thread-result) as `slack_post` calls in `next`; the caller posts
  them and runs `bosun notify --ack KEY`. bosun never calls Slack and holds no Slack token.
- **Setup.** A `[slack]` table in `.bosun/bosun.toml` (`channel`, `spawners = "owners"`,
  `may_spawn`, `may_build = false`, `notify`), a private channel with no guests and not shared, and your Slack ID in
  `governance/owners.toml`.

Gaps:

- bosun cannot check the channel facts or the Slack user ID itself; it trusts the caller.
- `bosun ask` itself mentions you on GitHub; the Slack post comes only through `bosun notify`
  (owner-question), so it needs the notify Routine or a live thread.
- The `send_message` `to`/`thread_ts` routing is known only from the session tool schema, and
  whether Claude Tag sets `CLAUDE_CODE_CHILD_SESSION` is not checkable from a session.
- A Slack approval form does not exist; approvals never come from Slack.
