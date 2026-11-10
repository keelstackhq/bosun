## 11. Seed and bootstrap

bosun pins its own release in `.bosun/version`. Its gates run the pinned release, never the binary
built from main, so a bad change to bosun cannot approve itself: job `ci` (step `bosun of the base`, a worktree of
the PR's base commit outside the checkout) reads the pin from the PR's base commit and installs `@keelstackhq/bosun@<pin>` from npm, checking that it reports that
version. A base that pins `unreleased` (the seed, see the seed plan S10) builds bosun from the base
commit instead. A step of the same job, before the gates, checks the pin a PR sets: that release must install and report its version. One
bootstrap exception: when the base has no self-gating CI yet (the PR that adds it, and PRs stacked
on it before it merges), the change builds bosun itself, and CI says so in a warning.

`bosun doctor` reports the 12 steps below, then step 13 `plugins` (local, no network: plugins come from the bosun-hosted marketplace `keelstack` (`.claude-plugin/marketplace.json` on `main`, which pins each plugin source by sha); it is missing when `.claude/settings.json` lacks the `keelstack` marketplace (github `keelstackhq/bosun`), its `ref` is a sha or empty (Claude Code clones a marketplace with `--branch`, so it must be a branch or tag) or differs from `KEELSTACK_MARKETPLACE_REF` of `docs/setup-script.sh`, `superpowers@keelstack` or `caveman@keelstack` is not enabled, the setup script lacks `KEELSTACK_MARKETPLACE_REF` / `SUPERPOWERS_SHA` / `CAVEMAN_SHA`, `installed_plugins.json` (`$CLAUDE_CONFIG_DIR/plugins/`, else `$HOME/.claude/plugins/`) is absent, lacks either plugin or has a `gitCommitSha` off its pin, or the repo's marketplace file pins a plugin `source.sha` off the setup script; old `@superpowers-marketplace` entries do not count, and each problem's fix names the file and value), then step 14 `CI minutes` (local, no network: missing when `bosun gate ci-minutes` finds anything in the workflows, spec §7; ok without workflows), as ok, warn, skipped or missing (exit 5 with the count if any is missing; warn and skipped never fail). Without `GITHUB_TOKEN`/`GH_TOKEN` doctor still runs: the local steps are judged as usual and the steps that need GitHub (1, 2, 5, 8, 9, 10 when configured, 15, and step 3 when its local checks pass) are `skipped` with the fix "export GITHUB_TOKEN (repo and workflow scopes), then rerun `bosun doctor`". Offline doctor never reports a pass: it adds a `skipped` item `remote checks` naming the checks not run (the skipped steps, plus `hoist check` and the workflow environments when `--remote` was asked), sets `data.offline` (`false` with a token), and exits 5 (needs-human) with "N checklist steps missing; GitHub not checked (no token)" and that fix; `data.next_step` is the token fix when no local step is missing, else the first missing local step. `data.next_step` is `{step, fix}` of the first missing step (`next` is for tool calls only). Fix texts name the real `owner/repo`, never bosun's own docs. Slack and release are optional (F11): without a `[slack]` table in `.bosun/bosun.toml`, step 7 is skipped, `data.notify` is `{skipped: true, reason}`, the janitor prompt leaves out `bosun notify` and there is no notify Routine; a `[slack]` table with an empty `channel` is missing. Release counts as configured when hoist.toml has `[release]` or `[publish]`, a workflow is `release.yml`/`release.yaml` or names a `release*` environment, the repo has a publishable package (a Cargo `[package]`, at the root or a workspace member, without `publish = false` or `publish = []`, or a `package.json` without `"private": true`), or the repo has a release; under `--remote`, an item of hoist's release, npm or crates areas (hoist inferred a publish target) also counts, and `data.hoist.release_inferred` says so. Else step 10 is skipped and `--remote` puts hoist's other release items (a `release*` environment, a secret such as `HOIST_RELEASE_TOKEN`) that are not ok into `data.hoist.skipped`, failing no step. It cannot see Routines, so step 11 is missing unless the owner sets `BOSUN_ROUTINES_OK=1`; the Routine prompts are in `data.routines`. Step 6 also returns `run` (`bosun setup-snippet`, D23: it prints the one script to paste, so doctor does not embed it), `content` (the text of `docs/setup-script.sh`, `null` when missing) and `environment` (`keelstack`, the shared cloud environment whose setup script it is), so it can be pasted without opening the file. Step 2 counts a login that starts with `claude`, or an issue, PR or comment whose `performed_via_github_app.slug` does (the app acts as the owner), as the app; step 3 also reads the checks the default branch enforces (rulesets `GET /rules/branches/{b}` plus classic protection) and is missing when a check of hoist.toml is not among them, while 403 on both (a private repo on a free plan has no rulesets) only adds a "cannot verify" note, and so does 403 on one of them (classic protection often answers 403 while rulesets answer 200) when the other lacks a listed check; the note names the unreadable endpoint; step 4 accepts any non-empty `owner`/`owners` key in `governance/owners.toml`. `bosun doctor --remote` also runs `hoist check --json` (binary from `BOSUN_HOIST`, killed with its process group after 120 s, or `BOSUN_HOIST_TIMEOUT_SECS`) and maps its items to steps: 3 for github settings, rulesets and checks, 5 for release and environments (and github environments, secrets, variables), 10 for npm and crates first publish and trusted publishers. Drift makes a step missing and keeps its local problem and fix. `data.hoist` lists `drift`, `unknown` (items hoist could not read, which fail no step) and `notes` (failed verifies, which fail no step) (D17). `--remote` also checks the environment of every workflow job (each `environment:` name, plain, mapping or flow mapping, in `.github/workflows/*.yml`; a job whose environment is an expression or names none warns "cannot read the environment of job J in F"): `GET /environments/{name}`, and `GET /environments/{name}/deployment-branch-policies` when its policy is custom. Only custom branches that are exactly `main` (type `branch`) pass; a missing environment, a null policy (any branch), `protected_branches` or any other rule makes an ok step 5 `warn` (this never fails a step) with the Settings → Environments → <name> → Deployment branches → Selected branches → add `main` steps as the fix, and a 403 warns "cannot verify" (grant the token Environments: read, or Administration: read) and another error says to retry. `data.environments` lists `{name, status, detail, fix}` for each, and `{job, status, detail, fix}` for a job it cannot read.

Human checklist (12 steps):
1. Create the repo.
2. Install the Claude GitHub App on it.
3. Run `hoist apply` for branch protection, auto-merge (the merge queue follows in M3: `risk-check` and `review-verdict` are statuses on the PR head, which a queue commit does not carry), and the required checks
   `ci-ok`, `risk-check`, `review-verdict` and `main-open` (D15, D17).
4. Fill `governance/owners.toml`.
5. Run `hoist apply` for the environments and secrets (D17), and create the kill-switch label: `gh label create bosun:halt --repo <owner>/<repo> --color B60205`. The repo variable `BOSUN_HALT=0` is optional.
6. Create the cloud environment and its setup script (D23).
7. Optional: create the Slack channel, connect Claude Tag and set `[slack] channel`.
8. Start the seed session with the seed brief.
9. Let the seed PRs merge through auto-merge on green gates (D13).
10. Optional, for a repo that publishes: run the first publish with `hoist release bootstrap`; later
    releases run `hoist release` in a workflow, which makes the tag (D13, D24).
11. Create the Routines: janitor (every hour, Haiku), curator (daily, Sonnet), and with Slack notify
    (every hour, Haiku). `bosun doctor` prints their prompts.
12. Approve the plan PR and start the root session.

`bosun init --intake FILE` (zero-touch onboarding, design section 3): the schema check of `bosun intake check` runs first (a missing answer exits 5, a bad value 2), and nothing is written before it passes. `--stack`, `--owner` and `--plan` come from the file (`stack`, `repo.owner`, `plan`) and `--stack` is no longer required; a flag that disagrees with the file exits 2, naming both values. The `repo.name` is the project, the `bosun_pieces` go to splice as `--piece`, and the channel var `intake=1` is set. The file is copied byte for byte to `.bosun/intake.toml` and that path is always excluded from the render. `next` holds one `create_trigger` call per Routine (the prompts of `bosun doctor`, built by the same code): janitor every hour, curator daily, and notify only when `notify = "slack"`; each firing starts a fresh session, and `data.routines` names the model of each. `data.owner_steps` lists only the steps that stay human: the approval of the setup PR and of each splice update, and the first publish of a package (`hoist release bootstrap`) when `publish` has npm or crates.

### With an intake file (zero-touch onboarding)

With `.bosun/intake.toml` (spec §3) and `bosun init --intake`, today's steps become:

| Step | With an intake file |
| --- | --- |
| 1 create the repo, 3 and 5 hoist apply, 4 owners | automated by the provision workflow; init seeds `governance/owners.toml` from the intake |
| 2 Claude GitHub App | one-time org step (Claude App and provisioner app on all repositories) |
| 6 cloud environment and setup script | one-time org step: the shared "keelstack" environment (`bosun doctor` step 6 prints the script) |
| 7 Slack and Claude Tag | intake answer `notify`; connecting Claude Tag stays a one-time org step |
| 8 start the seed session | the orchestrator spawns the root after the setup PR merges |
| 9 seed PRs merge | unchanged (auto-merge, D13) |
| 10 `hoist release bootstrap` | stays human, once per new package |
| 11 Routines | `bosun init --intake` returns the `create_trigger` calls in `next` |
| 12 approve the plan PR | per D-014 (draft): an orchestrator signature or one owner tap; the root starts itself |
| each release | stays human: tap `release` |

Seed contents:
- bosun v0.1 commands: join, claim, heartbeat, release, reap, spawn, wave plan, wait, gate,
  risk-check, test-integrity (rust adapters), review-pack, verdict post, pr, ci-log, status, ask,
  lesson, hook, doctor.
- Governance: `.bosun/bosun.toml`, `limits.toml`, `risk.toml`, `version`,
  `governance/owners.toml`, CODEOWNERS.
- CI: the PR, merge-queue and main-health workflows. Each gate job calls `bosun gate <name>`.
- Claude config: `.claude/settings.json` with the PreToolUse and SessionStart hooks.
- Docs: AGENTS.md, REVIEW.md, `docs/plan-format.md`, `docs/spec.md`, `docs/defaults.md`.
- Plan: `plans/bosun.md` with items M1-M8.

Built later by bosun on itself: intake, answer, notify, budget, progress, main-triage, revert,
plan import and amend, init (the splice channel, D16), the curator, the ts-pnpm profile, `doctor
--remote`.

Seed steps (the seed plan in `docs/superpowers/plans/` splits each into PRs):
- S1: crate, modules, clap skeleton, JSON output, exit codes. Done when `bosun --help` lists all
  v0.1 commands and each command returns JSON.
- S2: GitHub client (labels, issues, comments, PRs, checks). Done when integration tests against a
  recorded fake pass.
- S3: registry and leases (join, claim, heartbeat, release, reap). Done when a claim with
  overlapping touch returns exit 3.
- S4: spawn and the 7 checks, wave plan, wait. Done when each check has a refusal test with exit 6.
- S5: gate runner, rust profile and adapters, test-integrity. Done when `bosun gate test` runs
  nextest on the affected packages only.
- S6: risk-check from the base branch, ask. Done when a PR that edits risk.toml is high, evaluated
  with the base rules.
- S7: review-pack, verdict post, pr (with `--auto-merge`). Done when a verdict below 0.85 does not
  approve.
- S8: hooks (edit-lock, label-lock, session-start). Done when an Edit on a claimed path is blocked.
- S9: lesson (manual add, hit, cap), status, doctor, usage providers. Done when `bosun doctor`
  reports each checklist step as ok or missing.
- S10: governance files, CI, AGENTS.md, REVIEW.md, plan. Done when CI is green on the S10 PR with
  the gates run by bosun.

### Pin (tool pins)

`bosun pin --latest [--tool splice|hoist|berth|bosun]... [--check]` (D35) reads `$BOSUN_NPM_REGISTRY` (default `https://registry.npmjs.org`; https only, or http to 127.0.0.1 or localhost, else exit 1): `@keelstackhq/<tool>/latest` for the version, then each of `<tool>`, `<tool>-linux-x64`, `<tool>-linux-arm64` is downloaded and its sha512 checked against `dist.integrity` (a tarball URL outside the registry, a non-`sha512-` integrity or a mismatch is exit 1). Versions are `X.Y.Z` only, without leading zeros; equal is `unchanged`, older than the pin is exit 3. All checks run before anything is written. It rewrites the `<TOOL>_VERSION`, `_SHA512`, `_LINUX_X64_SHA512`, `_LINUX_ARM64_SHA512` lines of `docs/setup-script.sh` (each must exist once), `.bosun/version` (bosun) and the same lines of the channel template (splice, hoist, berth). `--check` fetches only the `latest` metadata and lists `data.behind`. `data` is `{changed, unchanged, files, branch, title}` (`branch` is `bosun/pin`, or the head of the open pin PR). A repo with `.splice/channels/bosun.toml` is exit 1 (`splice update --channel bosun`). `bosun doctor` runs the same check (5 s, an npm error never fails it) and always reports step 17 `update`: `ok` ("pins and binary match npm latest"), `warn` when a pin is behind or this binary is older than npm's bosun latest (exit stays 0; the moves and the `npm i -g @keelstackhq/bosun@X` line in `detail`; `fix` = `bosun pin --latest --pr`, or the `npm i -g` line when only this binary is behind), or `skipped` with the reason and a fix (`BOSUN_PIN_CHECK=0`, npm unreachable, no `docs/setup-script.sh`, a bosun channel repo with nothing behind: `splice update --channel bosun`). A channel repo still compares the binary; its pins are the channel's. The npm `latest` of bosun is fetched once and reused. `--pr` (not with `--check`) does the rest: before any download it needs the default branch checked out with a clean tree (else exit 3), an open PR from `bosun/pin` or `bosun/pin-*` in this repo (a fork's PR of that name, or one with an unknown head repo, does not count) makes it a no-op naming that PR in `data.pr.number` (`changed` still lists what is behind); else after writing it switches to a new branch (`bosun/pin`, or `bosun/pin-YYYYMMDD` when an earlier pin left that name locally or on origin; both taken is exit 3, before anything is written), commits the changed files with the title, pushes (never force; a failed commit or push rolls back to the default branch and deletes the local branch) and opens the draft PR (`data.pr` = `{number, head_sha}`, `next` empty). `data.left` lists only what a human must do: approve the pin PR, and the `npm i -g @keelstackhq/bosun@X` line for this machine when bosun moved (agents turn on auto-merge; the cloud needs nothing, the next session start runs the merged script).

### Adopt and uninstall (no splice)

`bosun adopt` is the path for a repo without hull, splice or hoist. It writes the channel's core
files into the worktree (`.bosun/{bosun.toml,version,limits.toml,risk.toml,session-rules.toml,approvers.toml,fold-fragments.sh}`,
`governance/owners.toml` with `--owner`, the five bosun workflows, `.claude/settings.json`, `docs/setup-script.sh`) from templates
embedded in the binary, rendered by `core::render` (the Jinja subset the templates use). It never
overwrites: an existing file with the same bytes is `unchanged`, one with other bytes is `kept`
with a fix hint. It does not commit, push or open a PR, and it does not write splice.yml,
hoist.toml, CODEOWNERS, the AGENTS/REVIEW/CLAUDE sections or the stack seed
files. Flags: `--stack rust|ts-pnpm|other` (repeatable; default detected from `Cargo.toml` and
`package.json` + `pnpm-lock.yaml`), `--owner`, `--bosun-version`, `--policy`, `--dry-run`, `--no-remote`
(no token; no labels or registry issue). Unless `--no-remote`, it creates the missing labels
(`bosun:halt`, `bosun:registry`, the task state labels) and the registry issue; `--dry-run` only reads.

`--bosun-version` must be X.Y.Z (else exit 2). `--policy owner|owner-gated|orchestrator|autonomous` sets the template var `policy`, which renders `[approvals] policy = "<value>"` in `.bosun/bosun.toml` (§8; empty, the default, renders the file byte for byte as without the var); a pinned version (`--bosun-version`, else this binary's) older than the first release that parses the value (`autonomous` 0.8.0, the others 0.5.0) is exit 2 with that minimum in `error.fix`, before anything is written. `data.policy` echoes the flag; when `bosun.toml` is `kept` and does not already hold that policy, `data.owner_steps` starts with a step to set it by hand. `bosun init` has no `--policy`: it renders the channel of its release tag through splice, and releases before this var do not declare it. `--dry-run` without a token still lists the files; labels and registry then read `unknown`. An unreadable file is an error, never "absent". The owner step names `bosun-ok` as the required check when the repo has its own `ci-ok` job. `data.owner_steps` lists only what is left: the owners step when `governance/owners.toml` has no `[[owner]]` (rerun with `--owner`; edit the file when it exists), the key step (`bosun keygen --enroll`) when `.bosun/approvers.toml` has no `[[key]]`, then branch protection (with the settings link when origin is on github.com), pasting `bosun setup-snippet` once as the cloud environment's setup script, the setup PR and `bosun doctor --remote`. `data.commands` is the shell sequence that publishes the setup: switch to (or create) `bosun-setup`, `git add` of exactly the paths adopt wrote (plus `.bosun/approvers.toml` when git shows it new or changed, as after `bosun keygen --enroll`), commit, push and `bosun doctor`; with nothing written it is only `bosun doctor`, and a `--dry-run` gets none.

`.bosun/adopt.toml` records the bosun version, each written file with its FNV-1a 64 hash, the labels
adopt created and the registry issue it opened. It is rewritten after the GitHub step and after each file, so a failure midway still leaves a record. A rerun merges into it, so a second run is all
`unchanged`. `bosun uninstall` reads it and acts only on the paths adopt can write (never an absolute path, a `..` path, or one under a symlink; those are `kept` with a fix hint): a file whose hash still matches is deleted, a modified one is
`kept`, a missing one is `gone`; emptied directories go too. The manifest stays, holding only the labels and the registry issue, until they are dealt with, and goes last. `--labels` deletes the labels adopt created and closes its registry issue, but a label still on an open issue is `in-use` and stays (rerun later); `--dry-run` lists only. Without a manifest it exits 1 and points to `docs/adopt-existing-repo.md`.

bosun never reads, writes or deletes through a symlink: adopt reports a file whose path (or a parent) is a symlink as `kept` with a fix hint, and exits 1 when the manifest path is under one; uninstall does the same. The manifest also lists `dirs`, the directories adopt created; uninstall removes only those, and only when empty. `uninstall --labels --dry-run` reads open issues, so it reports `in-use` like a real run (without a token it previews `delete`).
