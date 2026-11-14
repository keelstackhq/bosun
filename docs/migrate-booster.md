# Migrate a repo to booster mode

Audience: Claude (an agent) that moves a repo from `gatekeeper` or `steady` to `booster`, and the
owner, who does a few steps. The repo already runs hull and bosun.

Facts come from [quickstart.md](quickstart.md), [update.md](update.md),
[prerequisites.md](prerequisites.md), the
[modes design](superpowers/specs/2026-11-11-modes-booster-design.md), `AGENTS.md` and `bosun <command> --help`.
A line marked **TODO** is not in these sources. Do not guess it. Ask the owner.

Rules for the whole page:

- Run one command. Read the one JSON object it prints. Check `ok`, `exit_code`, `data`, `error.fix`, `next`.
- On a refusal, do what `error.fix` says. Never work around it.
- bosun never calls Claude Code tools (D29). Make each call it returns in `next`. Nothing else.
- Agents never merge, tag or force-push (D-008). Auto-merge only.
- Owner steps are **[phone]** or **[computer]** (D-009). Do them in the order given.
- Go one step at a time: `gatekeeper` → `steady` → `booster`. Never skip `steady`. Never go lighter in a red week.

## 1. Before you start

### 1.1 Check

| Check | Command | Pass |
| ----- | ------- | ---- |
| bosun is 0.14.0 or newer on npm | `bosun pin --latest --check` | `data.behind` lists no tool, or the latest bosun is `>= 0.14.0` |
| Your binary is current | `bosun --version` | `0.14.0` or newer |
| Tree is clean, on the default branch | `git status --short`; `git branch --show-current` | no output; the default branch |
| Repo is healthy | `bosun doctor` | no `missing` step you cannot explain |
| No halt, main is green | `bosun status` | no halt, no main-red |

If bosun 0.14.0 is not on npm, stop. **[computer]** The owner publishes the release ([releasing.md](releasing.md)).
An older release refuses `[mode]` as an unknown key.

### 1.2 What booster changes

Booster is steady plus a fast lane. Values from the mode table:

| Knob | steady | booster | What it means |
| ---- | ------ | ------- | ------------- |
| `[approvals] policy` | repo value | `autonomous` (when the table leaves it unset) | Fast-lane approval. See below. |
| `push_gates` | `push` | `push` | Same: a `git push` needs gate stamps. |
| `heal_pr` | `notify` | `spawn` | `bosun heal` also returns a `create_session` fixer when the lease holder is dead. |
| `heal_main` | `revert-pr` | `revert-pr` | Same: a draft revert PR on red main. |
| `approvals_batch` | `daily` | `daily` | Same: one digest a day. |
| `spawn_auto` | `off` | `on` | `bosun wave tick <epic>` returns `create_session` calls. |
| `budget_feature` | `warn` | `ask` | At 100 % of a feature budget, `claim` and `spawn` exit 5 until the owner answers. |
| `brake` | `on` | `on` | Same. |
| `curator_loosen` | `off` | `propose` | The curator Routine may open one draft PR a week. |

Fast lane: the orchestrator approves a PR whose paths are on the D-017 allowlist and not on its deny list
(see [spec §8](spec/08-risk.md)). All of these must hold: every verdict on the exact head is clean, one verdict is
from an independent reviewer session, and the PR closes an open task that its author leases. A mode never sets
the `orchestrator` policy (it needs no verdict). An explicit `[approvals] policy` in the file wins.

An explicit key wins over the preset. `[approvals] policy` set in the file stays. `[mode]` overrides
take knobs only, never `policy`.

Targets: red-CI rate under 20 %, at most 3 owner touches a day.

### 1.3 What still needs the owner (in every mode)

- Every PR that touches `.bosun/**` or `.claude/**` (R-gov). That includes the `[mode]` PR and the registry pin PR.
- A lighter mode is R-weaken. The owner approves it. **[phone]** The chat form or `/bosun approve <head sha>` is enough.
- Workflows, releases, tags, secrets, environments, owner and key changes.
- Gate code (R-gate), plans (`**/plans/**`), and `docs/setup-script.sh` (the pin PR).
- Routines. They are human-only. **[computer]** claude.ai/code → Routines.
- The first-time owner steps for `bosun keygen --enroll`: the `.bosun/approvers.toml` PR (R-gov).

List these as `## Prerequisites` in the plan ([prerequisites.md](prerequisites.md), "Modes: owner steps"). Use `verify: owner`.
Then run `bosun prereqs <plan> --post` and send `data.message` to the owner once.

## 2. The common sequence

Do the steps in order. Use one PR per step. Follow the repo's own PR rules (for example fragments).

### Step 1. Survey

```bash
bosun mode          # offline
bosun metrics --days 30
bosun doctor
```

Expect:

- `bosun mode`: `data.mode` and `data.mode_source` (`"default"` when `[mode]` is absent), and `data.knobs.<knob>.{value,source}`.
- `bosun metrics`: red-CI rate, lead time, cost, owner touches, and `data.brake`.
- `bosun doctor`: each step `ok`, `warn`, `skipped` or `missing`, each with its `fix`. Step 17 `update` shows what is behind.

Write the `metrics` numbers on the first PR. It is the baseline. If the repo is on `gatekeeper`, do the whole
sequence for `steady` first (quickstart §5). Return here when `bosun mode` shows `steady`.

If `bosun mode` exits non-zero or `bosun doctor` shows a `missing` step: do its `fix`.

### Step 2. Pin the new release

```bash
# Local only: give bosun your GitHub login from `gh auth token` (a cloud session needs none).
bosun pin --latest --pr
```

Expect `ok: true`, `data.pr` (`number`, `head_sha`), and `data.left` (what a human must do).
It makes the branch `bosun/pin`, or `bosun/pin-<date>`. It opens a draft PR.

Then:

1. Mark the PR ready.
2. **[phone]** The owner approves the exact head (chat form: repo, PR, head SHA). The PR edits `docs/setup-script.sh` and `.bosun/version` (R-gov).
3. Turn on auto-merge. Never merge.
4. After the merge: `cat .bosun/version` shows the new version. `bosun mode` exits 0.

A repo that uses the bosun splice channel (`.splice/channels/bosun.toml`) does not run `bosun pin`. Run
`splice update --channel bosun`, or let the splice update bot open the PR.

If it refuses: use the table in [update.md](update.md) ("If verification fails"). Never install a package that failed a check.
Never edit a sha512 by hand.

### Step 3. Pin the registry

Do this first, before any budget knob turns on.

1. Find the live registry issue number. The registry is one issue with the label `bosun:registry`
   (`src/registry.rs`). Locally: `gh issue list --label bosun:registry --state all --json number,state,createdAt`;
   pick the open one. In a cloud session `gh` is absent: run `bosun join`. It prints the number as `data.registry_issue`.
   Check that it is the real registry before you pin it. Until a pin exists, `bosun doctor` warns with advice but no number.
2. Commit `.bosun/registry` with only that number. Open a PR. It is R-gov. **[phone]** The owner approves.

Without a pin, bosun takes the lowest open issue with the label. After a rotation (old closed, new opened) that is
the new one. The real risk is a swap: another labelled issue or a changed registry drops the indexes until
`bosun plan amend` runs. The pin removes that risk. It is the trust root.

Expect after merge: `bosun doctor` no longer warns about the registry.

### Step 4. Epic markers

Booster sets `budget_feature = "ask"`. Every open epic needs the `bosun:epic` marker in its body. The marker names
the plan and the item. Without it, `claim` and `spawn` refuse a task of that epic.

For each plan file:

```bash
bosun plan check <plan>
bosun plan amend <plan> --dry-run
```

Expect from `plan check`: `ok: true`. On an error it names the line and the fix. Fix the plan. A plan PR is R-gov: the owner approves it.

Do not run `bosun plan amend <plan>` (without `--dry-run`) yet. It writes the bind and epic-index records only
when `budget_feature` is not `off` (`steady` already sets `warn`). Do it in step 7.

If `claim` or `spawn` exits non-zero with a missing marker, read `error.fix`. It names the step.

### Step 5. Steady PR, then booster PR (or direct)

Two ways. Take the first unless the owner says otherwise.

**Two steps (recommended).** First a PR with `[mode] name = "steady"`. Merge it. Watch it for two weeks:
`bosun metrics --days 7` shows red-CI under 30 % and at most 5 owner touches a day. Then a second PR:

```toml
# .bosun/bosun.toml
[mode]
name = "booster"
# Optional overrides: knobs only, never policy. Example:
# heal_main = "off"
```

**Direct (gatekeeper to booster in one PR).** The quickstart says never skip a step. Do it only when the owner
says so in the chat form. Write that answer in the PR body. The PR is R-gov and R-weaken, so the owner still approves
the exact head. Without that answer, use the two steps.

For each PR:

1. Open the PR. It is R-gov and R-weaken. Say so in the body.
2. **[phone]** The owner approves the exact head (chat form: repo, PR, head SHA).
3. Turn on auto-merge. Never merge. The orchestrator merges when checks are green, per D-008.

`booster` also sets `[approvals] policy = "autonomous"` when the table leaves it unset. The pinned release
must know `autonomous` (step 2 does that). An explicit `policy` in the file wins.

**[computer]** Cloud and Claude Tag sessions hold no keys, so under `autonomous` they ask the CI signer
(`bosun approve <pr>@<head> --via-signer`). One owner step per repo makes that work:
`bosun keygen --role signer --id signer-ci --out <pem outside the repo> --enroll`, merge that
`.bosun/approvers.toml` PR (R-gov), create the GitHub environment `bosun-signer` (deployment branches:
the default branch only) with the secret `BOSUN_SIGNER_KEY` (the PEM), then delete the PEM. The channel's
`core` piece ships `.github/workflows/signer.yml`. `bosun doctor` warns until the workflow and a key are there; it cannot see the
environment's secret. The signer key signs only the autonomous form, so it never approves anything D-017 does not allow.

Expect after merge: `bosun mode` → `data.mode: "booster"`, `data.mode_source: "explicit"`.

If the config fails to parse (`unknown field`): the pin is older than the key. Go back to step 2.

### Step 6. After the merge: join

The orchestrator runs `bosun join` first. It registers this session in the registry; the plan amend in step 7 needs that.

```bash
bosun join          # origin from BOSUN_ORIGIN, or --origin local|slack|routine|spawned
```

Expect `ok: true`. If it says GitHub is unreachable or the config is unknown: set `GH_TOKEN` or `GITHUB_TOKEN` and
`BOSUN_REPO=owner/name`, then run it again.

### Step 7. Amend the plans

Now `budget_feature` is not `off`. For every plan:

```bash
bosun plan amend <plan> --dry-run
bosun plan amend <plan>
```

Expect `ok: true`. It writes the bind and epic-index records. Until it does, `claim` and `spawn` refuse
plan tasks. If it exits 5 (the epic index names another epic): the owner deletes the wrong index record.

### Step 8. Verify

```bash
bosun mode
bosun metrics --days 7
```

Check:

- `data.mode` is `"booster"`. `data.mode_source` is `"explicit"`.
- `data.knobs.spawn_auto.value` is `"on"`. `data.knobs.heal_pr.value` is `"spawn"`. `data.knobs.budget_feature.value` is `"ask"`.
- `bosun metrics`: `data.brake`. When it is on, there is no auto spawn and `heal_pr` is at most `notify`.

Watch two weeks. Targets: red-CI under 20 %, at most 3 owner touches a day, `data.brake` off.

### Step 9. Hull: `splice update` to v0.9.0

Source: [hull docs/updating.md](https://github.com/keelstackhq/hull/blob/main/docs/updating.md). Normally the
weekly splice bot opens this PR. Do it by hand only when the owner asks. Needs splice 0.3.0 or newer
(`pnpm dlx @keelstackhq/splice …` if it is not installed). From the repo root, on a clean branch:

1. `splice check --json`. Exit 1 means an update, drift or markers.
2. `splice diff --json`. Read `changes`. A `conflict` or `delete` needs a decision. Add `--to 0.9.0` to pin.
3. `splice update --all --json`. It updates the template, the decisions channel and the rigging.
4. Resolve any markers. Then `splice check` must exit 0.
5. `pnpm install && pnpm run ci`.
6. Commit `chore: splice update to hull v0.9.0`. One update per PR.

Never edit owned files to fix an update. hull v0.9.0 moves vite-plus to 1.5.1 (with a `vite-plus-core` override).
The PR touches `.splice/**`: it is R-gov, and the owner approves. A repo with the bosun channel
(`.splice/channels/bosun.toml`) gets bosun pins from `splice update --channel bosun`, not `bosun pin`.

### Step 10. Owner Routine prompts

Routines are human-only. **[computer]** claude.ai/code → Routines.

1. Janitor (hourly). `bosun doctor` prints it in `data.routines`. Add this sentence at the end:

```text
Then run `bosun heal` and make only the calls in its `next`; report data.prs.
```

2. Curator (daily, Sonnet). Replace its prompt with the exact text in [quickstart.md](quickstart.md) §4.
   It sets up `curator_loosen = "propose"`. The first half is a no-op without a `bosun:lesson` issue.
3. Digest. Daily, `bosun digest`. **[phone]** The owner answers the form in chat. Or **[computer]** the key holder runs
   `bosun digest --answer --sign-key ~/.bosun/approver.pem --from -`. Each approval comes back as
   `bosun approve <pr>@<head> --sign-key <pem>` in `next`.

Verify: no bosun command reads Routines back. The owner confirms in chat that each Routine is saved.

### Step 11. Run

The orchestrator, per epic:

```bash
bosun wave tick <epic>
```

It returns `create_session` and `archive_session` calls in `next`. Make each call. For each `create_session`
result, run `bosun spawn <n> --record --from -`. Run `wave tick` again when children finish. Also run
`bosun heal` hourly and `bosun digest` daily.

If `data.held_by` says `spawn_auto is off` or `brake on: <reasons>`: nothing is wrong. Read the reason.

## 3. Environment A: Claude Code CLI on your machine

Setup, once ([adopt-existing-repo.md](adopt-existing-repo.md) §1):

```bash
npm i -g @keelstackhq/bosun
gh auth login                           # once, if gh is not signed in
bosun doctor                            # each missing step with its fix
bosun trailers install                  # the SessionStart hook does it too
```

Login. Set the environment variable `GITHUB_TOKEN` to the output of `gh auth token` (scopes: repo and workflow).
Only if bosun cannot read the origin remote, also set `BOSUN_REPO` to `owner/name`. Never write either in a file in the repo.

Hooks. `.claude/settings.json` holds `bosun hook session-start|stop|pre-tool-use`. They refuse a
`git push` without gate stamps (`push_gates = "push"`) and every merge, tag or force-push. Do not edit that file.
It is R-gov.

Session id. Set `BOSUN_SESSION=<session>/<task>` for each task, so your lease is told apart from your
orchestrator's and from other agents'. Run `bosun heartbeat` at least every 20 minutes.

Update the binary after the pin PR merges: `npm i -g @keelstackhq/bosun@<new>`, then `bosun --version`.

Gates before every push:

```bash
for g in fmt lint test deps test-integrity ci-minutes; do bosun gate "$g" || break; done
# TypeScript stack (ts-pnpm profile): add `types` to the list
```

Run the push as its own command: `git push -u origin <branch>`.

## 4. Environment B: Claude Code on the web (cloud container)

Do these once per repo. The environment is shared. **[computer]** The owner does steps 1 and 3 to 5.

Settings live in the cloud environment menu (session title bar → Edit). Source:
<https://code.claude.com/docs/en/cloud-environments>.

1. **Setup script.** Run `bosun setup-snippet`. Paste `data.script` into the environment's Setup script field.
   Paste it once. It does not change between releases. How it runs (Claude Code cloud docs): as root, on each fresh
   container, after the repos are cloned and before Claude starts; it starts in the directory that holds the
   checkouts; it cannot read the environment's variables or secrets; a failing script stops the session. The
   snippet is idempotent and always exits 0. It runs each checkout's own `docs/setup-script.sh`, which pins the
   tools (Node 24, pnpm, splice, hoist, berth, bosun, cargo-deny, the Rust toolchain of `rust-toolchain.toml`).
   The pin PR (step 2) sets `BOSUN_VERSION` and its sha512 lines there to the new release. Add nothing else to the snippet.
2. **Network hosts.** Choose Network access: Limited, with "Allow package managers" ticked (npm, PyPI, crates and
   similar are then reachable). GitHub goes through the session's GitHub proxy and the Claude GitHub App
   (<https://github.com/apps/claude>), not the host list. Add only hosts the repo's `docs/setup-script.sh` downloads
   from and that package managers do not cover. In this repo's script they are:
   - `registry.npmjs.org`: the pinned tools (`npm pack`, `npm install`). Covered by package managers.
   - `deb.nodesource.com`: Node 24, only when the container has no Node 24 yet.
   - `github.com`: the plugin marketplace clone (`claude plugin marketplace add`). It only warns on failure.
   - `tuf-repo-cdn.sigstore.dev`: only when the checkout has no repo script (the bootstrap checks npm signatures).
   If your own setup script installs other tools, add exactly the hosts it downloads from. A denied host shows as a
   proxy refusal that names the host.
   A repo script that only uses npm needs no extra host. Read the repo's own script before you add one.
3. **Environment variables.** Non-secret only. The docs name one for bosun: `CLAUDE_CODE_AUTO_COMPACT_WINDOW`
   (`data.autocompact.cloud_env` of `bosun setup-snippet`). Nothing else is required.
   No GitHub PAT, no owner credential and no age key in the environment. An API key is added as a network credential
   in the environment settings (the session can ask for it). GitHub comes from the connected GitHub app.
   Never put a secret in the script, a variable or a file.
4. **Tell bosun the environment.** Open a PR (R-gov, the owner approves; needs the pinned release from step 2):

   ```toml
   # .bosun/bosun.toml
   [cloud]
   environment_id = "env_..."     # the id of the environment the children run in
   ```

   `bosun wave tick` and `bosun spawn` put this id in each `create_session` call.
5. **Janitor Routine.** See step 10.

What a cloud session cannot do:

- Write commit statuses (the proxy answers 403). Use `bosun verdict comment --from -`, not `bosun verdict post`.
- Approve with `BOSUN_OWNER_TOKEN`. It exits 5. Approve by chat form.
- Use GraphQL for ready or auto-merge. bosun falls back to a `ccr` route by itself. If `bosun pr <issue> --ready`
  returns `data.ready: "pending"`, make the `mcp__github__update_pull_request` call in `next`
  (`draft: false`), then run `bosun pr <issue> --ready` again.
- Merge, tag or force-push. The hooks refuse it.

Each child session asks for its own permissions. An orchestrator's go does not carry over. Open a blocked child
in the Claude app.

Start: the orchestrator runs `bosun wave tick <epic>` and makes the calls in `next`.

## 5. Environment C: Claude Tag in Slack

**[computer]** Once: a private channel, no guests, not shared. Put your Slack ID in `governance/owners.toml`.
Then open a PR (R-gov):

```toml
# .bosun/bosun.toml
[slack]
channel = "C0123456789"
spawners = "owners"
may_spawn = true
```

`may_build` stays false. Code comes from spawned sessions.

The Claude Tag session labels itself:

```bash
bosun join --origin slack --slack-channel C0123456789 [--slack-thread <ts>]
bosun notify --channel-info -     # posts due: halt, main-red, asks
```

Start the migration from a thread. An owner posts the request. The session:

1. Runs `bosun status`.
2. Runs `bosun spawn --slack-thread <ts> --slack-user <U...> --request -` for work that has no epic yet.
   The user must be an owner's Slack ID in `governance/owners.toml`.
3. Makes the `create_session` call in `next`, then `bosun spawn <epic> --record --from -`.

A thread spawn inherits the caller's environment. `wave tick` uses `[cloud]`'s.

The steps of §2 run in the spawned session, in environment A or B. Slack only starts and reports.

Approvals never come from Slack. bosun answers needs-human in the thread. **[phone]** The owner approves
by chat form or on GitHub. `bosun answer <issue> --slack-user <U...>` records an owner's answer from Slack on a needs-human task.

## 6. Prompts for Claude

Copy a block. Send it to Claude in the right place. They name no repo. Claude reads this page.

### Environment A (CLI)

```text
Migrate this repo to bosun booster mode. Read docs/migrate-booster.md. You work in my local Claude Code CLI.
Rules: follow error.fix on every refusal; never merge, tag or force-push; make only the calls bosun returns in next.
1. Do section 1 (checks). Stop and tell me if bosun 0.14.0 is not on npm.
2. Give me ONE checklist of the steps only I can do, each marked phone or computer, computer first.
3. Do section 2, steps 1 to 8, one PR per step. After each PR ask me with a chat form: repo, PR, head SHA. Turn on auto-merge when I approve.
4. Mark TODO items and ask me. Do not guess flags, hosts or settings.
5. At the end report: PR numbers, bosun mode output, bosun metrics --days 7 data.brake.
```

### Environment B (cloud)

```text
Migrate this repo to bosun booster mode. Read docs/migrate-booster.md. You run in a Claude Code cloud environment.
Rules: follow error.fix on every refusal; never merge, tag or force-push; no PAT; use bosun verdict comment, not verdict post.
1. Do section 1. Stop and tell me if bosun 0.14.0 is not on npm.
2. Check this environment against section 4: setup script pasted, network hosts, [cloud] environment_id. Tell me what is missing.
   I set the environment in claude.ai/code. Do not put a secret in the script or in an environment variable.
3. Give me ONE checklist of my steps, each marked phone or computer.
4. Do section 2, steps 1 to 8, one PR per step. Ask for each approval with a chat form: repo, PR, head SHA.
5. Mark TODO items and ask me. Do not guess hosts or settings.
6. At the end report: PR numbers, bosun mode output.
```

### Environment C (Slack)

```text
Migrate this repo to bosun booster mode. Read docs/migrate-booster.md, sections 1, 2 and 5.
Join with: bosun join --origin slack --slack-channel <channel id> --slack-thread <ts>.
Run bosun status. Then spawn one orchestrator session for the migration with bosun spawn --slack-thread <ts> --slack-user <my id> --request -.
Make the create_session call in next, then record it. Post progress in this thread through bosun notify.
Ask me for approvals in the thread; I answer by chat form or on GitHub, never in Slack.
Mark TODO items and ask me. Never merge, tag or force-push.
```

## 7. Rollback

A tighter mode is one PR. It is no R-weaken finding. It is R-gov, so the owner still approves. **[phone]**

```toml
# .bosun/bosun.toml
[mode]
name = "steady"        # or "gatekeeper"
```

To go back to today's behaviour, delete the `[mode]` table. Gatekeeper without `[mode]` is byte for byte the old behaviour.

Rules:

- Keep `.bosun/registry`, the epic markers and the bind records. They do no harm in a tighter mode.
- Overrides: a loosening override (for example `push_gates = "ready"`) is R-weaken. A tightening one is not.
- Remove the heal line from the janitor Routine and put back the old curator prompt only if you go to `gatekeeper`. **[computer]**
- Do not unpin bosun. Pins only move up. A downgrade exits 3.
- Brake. If `data.brake` is on, do not lift it by hand. It ends by itself when the 7-day window is clean.

Find a session's commits to undo: `bosun trailers list <session>`. Revert a merged PR: `bosun revert`
(it opens a `kind/mainfix` PR). Never force-push.

## 8. Troubleshooting

Always read `error.fix` first. Exit codes: 0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit.

| You see | Cause | What to do |
| ------- | ----- | ---------- |
| `bosun could not reach GitHub (… unknown field …)` at session start | The pin is older than a key in `.bosun/bosun.toml` | Step 2. Pin the new release. Set `GH_TOKEN` or `GITHUB_TOKEN` and `BOSUN_REPO`, then `bosun join`. |
| `unknown field` for `[mode]` | The pinned release does not read `[mode]` | Step 2. Never set `[mode]` before the pin merged. |
| `[approvals] policy = "autonomous"` is a parse error | The pinned release is older than 0.8.0 | Step 2. |
| exit 4 | The repo is halted | The owner closes the `bosun:halt` issue. |
| exit 4 on `wave plan` or `wave tick` | Main is red | Wait for the revert PR or the fix. Do not claim work. |
| exit 3 on `claim` | Another session holds an overlapping lease | Pick another task: `bosun wave plan <epic>`. |
| exit 3 on `pin` | Dirty tree, wrong branch, `bosun/pin` and `bosun/pin-<date>` both left, or npm latest is older than the pin | [update.md](update.md), "If verification fails". |
| exit 5 on `claim` or `spawn` | A task needs the owner, or a budget ask is open (`budget_feature = "ask"`) | Budget ask: the owner answers `raise <epic> +N` in the digest reply; then the orchestrator runs `bosun budget raise --from -` with the `bosun digest --answer` output. Caps: one raise per digest day, 7 days at most the base budget, all raises at most 2x the base (more is an R-gov plan PR). Other needs-human: `bosun ask <issue> "<question>"`. |
| `claim` or `spawn` refuse a plan task, no marker | The epic has no `bosun:epic` marker, or the bind records are missing | Step 4, then step 7. |
| exit 6 | A limit or a cap | Read `error.fix`. At the lesson cap: mechanize, combine or retire a lesson first. |
| `bosun pr --ready`: `missing_gates` (exit 1) | No gate stamps for this tree | Run the loop in `error.fix`. |
| `bosun pr --ready`: exit 3, unpushed head | Local HEAD is not the PR head | Push, or pull. |
| `bosun pr --ready`: `data.ready: "pending"` | The cloud proxy refuses GraphQL | Make the `next` call, run it again. |
| `git push` is refused | `push_gates = "push"` and a gate has no stamp | Run the gate loop. Push alone, as `git push -u origin <branch>`. |
| `bosun approve` with a PAT exits 5 in a cloud session | A cloud session cannot use `BOSUN_OWNER_TOKEN` | Approve by chat form. |
| `bosun doctor` warns about the registry | No `.bosun/registry` pin | Step 3. |
| `bosun wave tick`: `held_by` `spawn_auto is off` | The mode is below booster, or an override sets it off | `bosun mode`. |
| `bosun wave tick`: `brake on: …` | Over 7 days: main red twice, two PRs reverted, or red-CI 10 points over target | Wait. It ends by itself. |
| `bosun heal` returns an empty `next` | Both `heal_pr` and `heal_main` are `off`, or nothing is red | `bosun mode` to check the knobs. |
| `bosun join` reaches no GitHub | No token or no `BOSUN_REPO` | Set `GITHUB_TOKEN` (or `GH_TOKEN`) and `BOSUN_REPO=owner/name`. |

If a step is not covered here, stop. Run `bosun doctor`. Read its `fix`. Ask the owner.
