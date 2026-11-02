# Zero-touch onboarding: design

Date: 2026-11-02. The owner approved the design. This spec maps it onto bosun's model: `bosun init`,
plans, spawn, resume and the registry (spec §4, §5, §11, §14). It changes no code. The tasks are
listed in §6.

Goal: an owner answers one chat form, once. After that, no agent asks the owner anything, except a
release tap or one report of a hard blocker.

## 1. Flow

Every repo, new or existing, goes through three phases.

1. **Intake.** The orchestrator asks one batched form (AskUserQuestion). It writes the answers to
   `.bosun/intake.toml`.
2. **Setup.** The provisioner workflow (§3.2) creates or prepares the repo. It renders the hull
   template (§1.1), then runs `bosun init --intake`, which opens the setup PR.
3. **Lifecycle.** The root session runs the wave loop (spec §5) and relays itself (§5 below).

For an existing repo, intake starts with an assessment. The orchestrator runs hull's
`node ready.mjs --assess --json` in the checkout. It shows the result (stack, pieces, unsure
items) on the first form screen, for the owner to confirm. An unsure item becomes a question.
GAP: the output schema of `ready.mjs --assess` is hull's; §6 task H1 fixes it.

### 1.1 Hull template, then bosun

- New repo: `splice new gh:keelstackhq/hull//template --piece <p>...`, one `--piece` per entry
  of `pieces`.
- Existing repo: `ready.mjs --assess`, then `splice adopt gh:keelstackhq/hull//template --piece
  <p>... --dry-run`. Each owned file the dry run refuses becomes an intake question (keep, replace
  or skip the piece). When it cannot be answered, it is the one blocker report. Never `--force`.
  After the answers, `splice adopt` runs without `--dry-run`.
- Only then `bosun init --intake .bosun/intake.toml` adds the bosun channel with `bosun_pieces`.
- Stacks: python and go repos get the decisions channel only (`splice channel add
  gh:keelstackhq/decisions//channel`), until bosun has a generic stack. GAP, known: `--stack`
  takes only `rust` and `ts-pnpm` today (`src/cli/mod.rs`).

## 2. The intake file

`.bosun/intake.toml` is a new seed file: it is written once, then the repo owns it. Add it to `seed`
in `channel/pieces/core/piece.toml`. It holds every upfront answer. Nothing else asks again.

```toml
# Written by `bosun init --intake`. Unknown keys are an error.
version = 1
repo = { name = "app", owner = "keelstackhq", visibility = "private" }   # or "public"
stack = ["rust"]                       # rust | ts-pnpm (spec §3); repeatable
pieces = ["base", "rust-cli"]          # hull template pieces, for example astro-site
bosun_pieces = ["claude-md"]           # bosun channel pieces, for example circleci
goal = "A CLI that ..."                # free text; or plan = "plans/app.md"
publish = ["crates"]                   # npm | crates | cloudflare | none
approvals = "orchestrator"             # owner | orchestrator | owner-gated (§4)
notify = "chat"                        # chat | slack
slack_channel = ""                     # required when notify = "slack"
```

Rules:
- `goal` and `plan` exclude each other. With `goal`, the root session writes `plans/<repo>.md`
  in the plan format (docs/plan-format.md) as the first plan PR.
- `bosun intake check` validates the file: types, allowed values, cross-field rules. It exits 2
  on a bad value and 5 on a missing answer, naming the field in `error.fix`.
- `bosun init --intake FILE` runs the check first. It refuses (exit 5) when an answer is
  missing. It derives `--stack`, `--owner` and `--plan` from the file; a flag that disagrees with
  the file is a usage error (exit 2). It copies the file to `.bosun/intake.toml`.
- `bosun spawn` and `bosun wave plan` refuse (exit 5) when `.bosun/intake.toml` on the default
  branch is missing or invalid, for a repo whose `.bosun/bosun.toml` has `intake = true`.

Release ordering. This is a new file, not a new table, so pinned releases that do not know it
ignore it. The `intake = true` key in `.bosun/bosun.toml` is new; the pinned release rejects
unknown keys. Order: release 0.5.0 reads the key, then a pin PR moves `.bosun/version` to 0.5.0
and adds 0.5.0 to KNOWN (`tests/pinned_config.rs`), and only then may a repo set the key.

### 2.1 Question set

At most two screens of at most four questions each. AskUserQuestion gives every question an
"Other" free-text answer.

| Screen | Question | Options | Field |
| --- | --- | --- | --- |
| 1 | Repo name and visibility? | private (Recommended), public | `repo.name`, `repo.visibility` |
| 1 | Stack? (existing repo: the assessed value is first) | rust, ts-pnpm, rust + ts-pnpm | `stack` |
| 1 | What should it do? | a goal sentence (Other), a plan link | `goal` or `plan` |
| 1 | Publish to? (multi-select) | npm, crates, cloudflare, none | `publish` |
| 2 | Who approves gated PRs after 2026-11-30? | owner-gated (Recommended), owner, orchestrator | `approvals` |
| 2 | How should agents report? | chat (Recommended), slack | `notify` |
| 2 | Hull pieces? (multi-select; existing repo: the assessed ones first) | base, rust-cli, astro-site | `pieces` |
| 2 | bosun pieces? (multi-select) | claude-md, circleci | `bosun_pieces` |

`stack` follows from the hull pieces when they name one (rust-cli means rust). The question then
shows that value first. Each owned file `splice adopt --dry-run` refuses adds one keep, replace or
skip question to screen 2. When that would pass four questions, the extra items go into the one
blocker report. `repo.owner` is the org (D-001). The repo name is taken from the free-text answer.

## 3. Repo setup without per-repo owner steps

### 3.1 One-time org setup (owner, once)

- Org secrets, shared with the selected repos: `SPLICE_BOT_TOKEN` and `CLOUDFLARE_API_TOKEN`.
  Each one is used through an environment, never as a repo secret (D17, D-010).
- npm and crates publish through OIDC trusted publishing, as `hoist release` does. There is no
  `NPM_TOKEN` or `CARGO_REGISTRY_TOKEN`. The first publish of a new package needs no token either:
  `hoist release bootstrap` runs on the owner's computer after `npm login` and `cargo login`, and
  the crates.io trusted publisher is added in its UI (docs/releasing.md steps 4 and 5).
- Org GitHub App "keelstack provisioner", installed on all repos. Permissions: Administration,
  Contents, Environments, Issues, Pull requests and Workflows (read and write); Members (read).
- The app's private key goes in one place only: the environment `provision` of the central repo.

### 3.2 The central repo and its workflow

Proposal: a new repo `keelstackhq/provision`. It is small, owner-merged, and holds only this
workflow and its tests. It does not use `keelstackhq/decisions`, which stays text-only and
owner-merged (D-008), and it does not use a product repo, which agents edit every day.

`.github/workflows/provision.yml`:
- Trigger: `workflow_dispatch` with one input, the intake file contents (base64), plus `repository_dispatch` from the orchestrator.
- Environment `provision`: no required reviewer. Its deployment branch is `main` only.
- The gate is a signature. The intake carries an orchestrator signature in the scheme of
  `bosun approve --sign-key` (key `orchestrator-2026-10`). The workflow verifies it against the
  approvers file of `keelstackhq/provision` (`.bosun/approvers.toml`) and refuses without it. The
  owner's form answer is the approval: zero taps per repo.
- Steps: verify the signature, then validate the intake with the pinned bosun (`bosun intake check
  --from -`). Mint an app
  token (`actions/create-github-app-token`, pinned by sha). Then:
  1. create the repo and render hull (§1.1), or check that an existing one matches and adopt;
  2. run `hoist apply` (rules, auto-merge, required checks ci-ok, risk-check, review-verdict and main-open, environments `release` and `splice-bot`);
  3. create the label `bosun:halt`;
  4. grant the org secrets to the repo;
  5. push a branch with `.bosun/intake.toml` and run `bosun init --intake .bosun/intake.toml`, which opens the setup PR.
- No PAT anywhere. The app token lives for one job and is never printed.

### 3.3 Routines

The orchestrator creates the janitor (hourly, haiku) and curator (daily, sonnet) as scheduled
tasks with `create_trigger`, from the prompts `bosun doctor` prints. It adds notify only when
`notify = "slack"`. bosun itself never calls these tools (D29): `bosun init --intake` returns the
`create_trigger` calls in `next`.

### 3.4 Map of today's steps

| Today (spec §11 / `OWNER_STEPS` in `src/cli/init.rs`) | After |
| --- | --- |
| §11.1 create the repo | automated: provision.yml, gated by the orchestrator signature on the intake |
| §11.2 install the Claude GitHub App | one-time org step (install on all repos) |
| §11.3 hoist apply: rules, auto-merge, checks | automated: provision.yml |
| §11.4 / step 1: fill governance/owners.toml | automated: init seeds it from `repo.owner` and the org owners |
| §11.5 hoist apply environments and secrets; halt label | automated: provision.yml; secrets are org-level |
| step 4: splice-bot environment + SPLICE_BOT_TOKEN | automated environment; org secret (one-time) |
| §11.6 cloud environment + setup script | one-time org step: one shared environment for all repos |
| §11.7 Slack and Claude Tag | intake answer `notify`; connecting Claude Tag stays a one-time org step |
| §11.8 start the seed session | automated: the orchestrator spawns the root after the setup PR merges |
| §11.9 seed PRs merge on green | unchanged (auto-merge, D13) |
| step 5: approve the R-gov setup PR | per D-014: an orchestrator signature, or one owner tap |
| §11.10 hoist release bootstrap | stays human, once per new package: computer, `npm login` + `cargo login`, then the crates.io trusted publisher |
| §11.11 / step 6: create the Routines | automated: create_trigger (§3.3) |
| step 7: approve each weekly splice update | per D-014 (R-gov) |
| §11.12 approve the plan PR, start the root | per D-014; the root starts itself |
| every release: tap `release` | stays human, per repo |

## 4. Approvals after bootstrap mode

Bootstrap mode ends on 2026-11-30 UTC (D-012). The intake field `approvals` picks one of three
policies:

- `owner`: every gated PR waits for an owner approval (chat form, D-012).
- `owner-gated`: the orchestrator approves low-risk and R-ci PRs after green checks and an
  independent review. The owner approves R-gov, `.bosun/`, approval code, workflows, secrets and
  releases.
- `orchestrator`: as in bootstrap mode, without an end date. Environments, tags, releases,
  decisions merges, branch rules and secrets still need the owner.

bosun reads it in `bosun approve` and `risk-check` as `[approvals] policy` (a new key, so it
follows the release ordering in §2). The appendix holds the decision text.

## 5. Lifecycle and relay

### 5.1 Handoff and resume

- `bosun handoff [--from FILE]` writes a `handoff` record to the registry issue: open waves, open PRs with
  heads, leases, next steps, and blockers. At most 40 lines; more is exit 6. Each item is one
  line; the format is in `src/core/records.rs` (new `Record::Handoff`).
- `bosun resume --handoff` reads the newest handoff of this repo's root lineage and returns it in
  `data.handoff`, together with what `resume` returns today.
- A handoff is never a lease. The new root claims again through `resume`, so the lease rules of
  spec §4 hold.

### 5.2 Relay rule

The root relays after each merged milestone (epic) or after 6 waves, whichever comes first. Six is
about the number of waves after which a root's context nears the point where reports start to be
summarized. It sits in `.bosun/limits.toml` as `relay_waves = 6`.

Relay steps:
1. `bosun handoff`.
2. `bosun spawn --root --handoff`, which returns the `create_session` call for a fresh root with a
   prompt of one line: `bosun resume --handoff`.
3. `spawn --record`.
4. `archive_session` on itself; bosun returns that call in `next` too (D29).

### 5.3 Token rules

| Rule | Where it lives |
| --- | --- |
| workers sonnet, search haiku, hard tasks opus | `[models]` in `.bosun/limits.toml` (exists) |
| reports at most 10 lines | the spawn prompt (`core::spawn`) |
| context packs at most 400 lines | spec §4 (exists); `bosun spawn` checks it |
| the orchestrator reads only `bosun status --json` | the root prompt |
| reviews run in separate sessions | review mode, spec §8 (exists) |

## 6. Milestones

Gated per D-013 means: workflows, templates, governance, `.bosun`, approval code, secrets,
environments or releases. Those PRs are at most 400 lines.

| Id | Repo | Task | Risk |
| --- | --- | --- | --- |
| B1 | bosun | `core::intake` parser, `bosun intake check`, `bosun intake sign --sign-key` | gated (approval code) |
| B2 | bosun | `init --intake`: derive flags, copy the file, return create_trigger in `next` | gated |
| B3 | bosun | channel: `.bosun/intake.toml` seed; `intake` key in bosun.toml | gated |
| B4 | bosun | `Record::Handoff`, `bosun handoff`, `resume --handoff` | low |
| B5 | bosun | relay: `relay_waves`, `spawn --root --handoff`, archive in `next` | gated |
| B6 | bosun | `[approvals] policy` in approve and risk-check | gated |
| B7 | bosun | spec §4, §5, §11, §14 updates; release 0.5.0; pin | gated |
| H1 | hull | `ready.mjs --assess --json`: a fixed output schema + tests | low |
| P1 | provision (new) | repo, `provision.yml` (signature check, hull render or adopt, init), environment `provision` (main only) | gated |
| P2 | hoist | `hoist apply` with an app token (no PAT), org secret grants | gated |
| D1 | decisions | file D-014 (appendix) | owner-merged |

Order: B1, H1 and B4 in parallel; then B2, B3; P2, P1; B5, B6; B7 last.

## Appendix: draft D-014

| ID | Date | Scope | Decision |
| --- | --- | --- | --- |
| D-014 | 2026-11-?? | all | After bootstrap mode (D-012) each repo's approval policy is an intake answer in `.bosun/intake.toml`: `owner`, `owner-gated` (default: the orchestrator approves low-risk and R-ci PRs after green checks and an independent review of that head; the owner approves R-gov, `.bosun`, approval code, workflows, secrets and releases) or `orchestrator` (the bootstrap rules, without an end date). In every policy, environments, tags, releases, decisions merges, branch rules and secrets need the owner. Repos are created and configured only by the org provisioner app, through a workflow whose environment deploys from main only and that refuses an intake without a valid orchestrator signature; the owner's intake answer is the approval, with no per-repo tap; no PAT is used. npm and crates publish through OIDC trusted publishing. |
