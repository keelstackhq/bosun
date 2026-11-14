# Quickstart: modes

Four modes in `.bosun/bosun.toml`. Pick one, set it, run the commands for your environment.
Spec: [spec §18](spec/18-modes.md). Every command below: `bosun <command> --help`.

| Mode | What bosun does for you | Owner touches |
| ---- | ----------------------- | ------------- |
| `gatekeeper` | Today. No helpers. The default without `[mode]`. | as needed |
| `steady` | Gates before push, red-PR notices, revert PRs on red main, daily digest, feature budget warnings. | ≤ 5 a day |
| `booster` | Steady plus fast-lane approval (`autonomous` policy), auto spawn, fixer sessions, budget asks (advisory until budget bindings land; a raise is a plan PR), curator proposals. | ≤ 3 a day |
| `autonomous` | Booster plus auto-merge of exact-inverse reverts (also needs a clean verdict, and the reverted PR needed no owner); helpers cut the owner's touches to about one a day (the digest); owner-class PRs still need an approval. | ≤ 1 a day |

Owner steps are marked **[phone]** or **[computer]** (D-009). Everything else an agent does.

## 0. Set the mode (all modes, all environments)

**[computer]** Needs a published bosun release that reads `[mode]` (owner release). An agent then pins it
(`bosun pin --latest --pr`, see [update.md](update.md)) and you approve that PR **[phone]**: an older release
refuses `[mode]` as an unknown key. Then open a PR:

```toml
# .bosun/bosun.toml
[mode]
name = "steady"        # gatekeeper | steady | booster | autonomous
# optional overrides (knobs only, never policy), for example:
# heal_main = "off"
```

The PR is R-gov. A lighter mode (toward `autonomous`) is R-weaken: it needs your approval ([phone] is enough: the
chat form or `/bosun approve <head sha>` on GitHub). Check what is in force (offline):

```bash
bosun mode          # data.mode, data.knobs.<knob>.{value,source}, data.brake (null offline)
bosun metrics       # red-CI rate, lead time, cost, owner touches, data.brake (GitHub)
```

Knobs per mode:

| Knob | gatekeeper | steady | booster | autonomous |
| ---- | ---------- | ------ | ------- | ---------- |
| `push_gates` | `ready` | `push` | `push` | `push` |
| `heal_pr` | `off` | `notify` | `spawn` | `spawn` |
| `heal_main` | `off` | `revert-pr` | `revert-pr` | `revert-auto` |
| `approvals_batch` | `per-pr` | `daily` | `daily` | `daily` |
| `spawn_auto` | `off` | `off` | `on` | `on` |
| `budget_feature` | `off` | `warn` | `ask` | `ask` |
| `brake` | `off` | `on` | `on` | `on` |
| `curator_loosen` | `off` | `off` | `propose` | `propose` |

Limits and money (`.bosun/limits.toml`, `[budget]`) are never part of a mode.

## 1. Local Claude Code CLI

Setup, once ([adopt-existing-repo.md](adopt-existing-repo.md) steps 1-5):

```bash
npm i -g @keelstackhq/bosun
# set GITHUB_TOKEN to the output of `gh auth token`
bosun doctor                      # each missing step with its fix
bosun trailers install            # the SessionStart hook does it too
```

The hooks in `.claude/settings.json` (`bosun hook session-start|stop|pre-tool-use`) are what
refuse a `git push` without gate stamps (`push_gates = "push"`), and every merge, tag or force-push.

| Mode | Run in the session |
| ---- | ------------------ |
| gatekeeper | `bosun claim <task>` → work → `bosun pr <task>` → local gates → `bosun pr <task> --ready` |
| steady | Same, but run the gates **before the push** (loop below), then push. Hourly: `bosun heal`. Daily: `bosun digest`. |
| booster | Orchestrator, per epic: `bosun wave tick <epic>` (it returns `create_session` / `archive_session` calls in `next`; you make them, then run `bosun spawn <n> --record --from -` with each result). Plus `bosun heal`, `bosun digest`. |
| autonomous | Same as booster; `bosun heal` also readies exact-inverse reverts of red main (clean verdict in, reverted PR needed no owner). |

```bash
for g in fmt lint test deps test-integrity ci-minutes; do bosun gate "$g" || break; done
# TypeScript stack (ts-pnpm profile): add `types` to the list
```

Notes:
- bosun never calls Claude Code tools (D29). Read `next`, make each call, nothing else.
- Digest: **[phone]** you answer the form in chat. **[computer]** the key holder (`~/.bosun/approver.pem`; `bosun keygen --enroll` once, and it counts only after its `.bosun/approvers.toml` PR merged: R-gov, owner approves) runs:
  paste the digest's `data.header` as line 1, your reply under it, to
  `bosun digest --answer --sign-key ~/.bosun/approver.pem --from -`. Each approval comes back as
  `bosun approve <pr>@<head> --sign-key <pem>` in `next`.
- Check the brake: `bosun metrics` → `data.brake`. On: no auto spawn, heal at most `notify`.

## 2. Web / cloud sessions

**[computer]** Once per repo (the environment is shared):

1. Environment setup script: paste `bosun setup-snippet`'s `data.script` (claude.ai/code →
   Environments → Setup script). Never changes again; each checkout's `docs/setup-script.sh`
   pins the tools.
2. Tell bosun the environment, in `.bosun/bosun.toml` (needs a release that reads it; R-gov, owner approves):

```toml
[cloud]
environment_id = "env_..."     # the id of the cloud environment the children run in
```

3. No PAT in the environment. Verdicts: `bosun verdict comment --from -` (statuses are blocked).
   `bosun approve` with `BOSUN_OWNER_TOKEN` refuses in a cloud session; approve by chat form.

| Mode | Start |
| ---- | ----- |
| gatekeeper, steady | Start an orchestrator session on the repo. `bosun spawn <epic>` → make the `create_session` call in `next` → `bosun spawn <epic> --record --from -`. |
| booster, autonomous | The orchestrator runs `bosun wave tick <epic>`; its `create_session` calls use `environment_id` from `[cloud]`. Run it again when children finish: it returns `archive_session` for done children. |

The janitor Routine (hourly, **[computer]**, claude.ai/code → Routines; prompts: `bosun doctor`,
`data.routines`) keeps leases and spend honest in every mode. For `steady` and lighter, add this
sentence at the end of the janitor prompt (owner step):

```text
Then run `bosun heal` and make only the calls in its `next`; report data.prs.
```

## 3. Claude in Slack (Claude Tag)

**[computer]** Once: private channel, no guests, not shared; your Slack ID in
`governance/owners.toml`; then

```toml
# .bosun/bosun.toml
[slack]
channel = "C0123456789"
spawners = "owners"
may_spawn = true
```

(`may_build` stays false: code comes from spawned sessions.) The Claude Tag session labels itself:

```bash
bosun join --origin slack --slack-channel C0123456789 [--slack-thread <ts>]
bosun notify --channel-info -     # posts due: halt, main-red, asks; the digest comes from bosun digest
```

| Mode | In a thread |
| ---- | ----------- |
| gatekeeper, steady | Ask `bosun status`; request work: `bosun spawn --slack-thread <ts> --slack-user <U...> --request -`. |
| booster, autonomous | Same. A thread spawn inherits the caller's environment; `wave tick` uses `[cloud]`'s. |

Approvals never come from Slack: bosun answers needs-human in the thread; you approve by chat
form or on GitHub **[phone]**.

## 4. Owner Routine prompts (owner-held)

Routines are human-only. **[computer]** claude.ai/code → Routines. Janitor and notify: `bosun doctor`
prints them. For `booster` and `autonomous` (`curator_loosen = "propose"`) replace the curator
Routine prompt (daily, Sonnet) with exactly this text:

```text
List the open issues labelled `bosun:lesson` that have no `bosun:lesson/*` label. For each one run `bosun lesson triage <n> --as blocking|mechanizable|advice` as in docs/spec.md section 6: blocking when the process must stop until it is fixed; mechanizable when a gate, a hook or a check can prevent it (add `--touch GLOB` for the paths it changes and `--area A`); advice only as the last choice (add `--rule TEXT` and `--scope GLOB`). Prefer mechanizable over advice. Add other rules with `bosun lesson add "<rule, at most 160 characters>" --evidence #<issue>` (the lesson issue or PR that proved it); at the cap (exit 6) mechanize, combine or retire a lesson first, and retire lessons with no hit in 30 days. Close a lesson issue when its mechanism has landed. Then, once a week at most (skip when you already opened such a PR in the last 7 days), run `bosun metrics --propose`. When data.curator_loosen is "propose" and data.proposals is not empty, take the first proposal: open ONE draft PR with its data.proposals[0].title as the title and its body as the body, changing only that rule in `.bosun/risk.toml` (loosen that one rule, nothing else). Never merge it, never mark it ready, never approve it; it is the owner's decision.
```

Without a `bosun:lesson` issue to triage the first half is a no-op; the second half is the only
thing that needs the Routine. `bosun metrics --propose` itself is read-only.

## 5. Migration: a repo that already runs bosun

One command, one owner approval ([migrate-booster.md](migrate-booster.md): steps and a prompt for the CLI,
web/cloud and Claude Tag in Slack; the manual path is its appendix).

```bash
bosun migrate --to booster --dry-run   # survey: version, mode, registry, unmarked epics, hull lock, baseline
bosun migrate --to booster             # one governance PR: version pin, registry pin, [mode]
bosun pr --pr <n> --ready              # after the local gates: ready + auto-merge (no task needed)
bosun migrate --finish                 # after the merge: join, plan amend (idempotent)
```

- One rung per PR (gatekeeper → steady → booster → autonomous) unless the owner says `--direct`; the PR body
  records that choice. Never move lighter on a red week.
- **[phone]** The owner approves the exact head: the PR is R-gov, and R-weaken when lighter. risk-check waits
  for it; nothing else needs a GitHub click.
- **[computer]** Routines stay owner-only: the PR body lists them (janitor heal line from `steady`, curator
  prompt from `booster`, section 4).
- Targets before the next rung: `steady` red-CI < 30 % and ≤ 5 owner touches a day for two weeks; `booster`
  < 20 %, ≤ 3; `autonomous` < 15 %, ≤ 1 (`bosun metrics --days 7`, `data.brake`).
- Going tighter is the same command (`--to steady`): no R-weaken, still R-gov.

Gatekeeper without `[mode]` is byte-for-byte today's behaviour; deleting the table goes back.
What mode never changes: R-weaken, R-gov, R-secrets, R-prod-write, workflows, releases and tags
stay with the owner; agents never merge, tag or force-push.
