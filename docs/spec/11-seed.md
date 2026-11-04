## 11. Seed and bootstrap

bosun pins its own release in `.bosun/version`. Its gates run the pinned release, never the binary
built from main, so a bad change to bosun cannot approve itself: job `ci` (step `bosun of the base`, a worktree of
the PR's base commit outside the checkout) reads the pin from the PR's base commit and installs `@keelstackhq/bosun@<pin>` from npm, checking that it reports that
version. A base that pins `unreleased` (the seed, see the seed plan S10) builds bosun from the base
commit instead. A step of the same job, before the gates, checks the pin a PR sets: that release must install and report its version. One
bootstrap exception: when the base has no self-gating CI yet (the PR that adds it, and PRs stacked
on it before it merges), the change builds bosun itself, and CI says so in a warning.

`bosun doctor` reports the 12 steps below, then step 13 `plugins` (local, no network: `.claude/settings.json` enables `superpowers@superpowers-marketplace` and `caveman@akramsystems-plugins`, each marketplace `ref` equals the setup script pin `SUPERPOWERS_MARKETPLACE_SHA` / `CAVEMAN_SHA`, and an installed plugin in `$HOME/.claude/plugins/installed_plugins.json` has `gitCommitSha` equal to `SUPERPOWERS_SHA` / `CAVEMAN_SHA`; a missing install file is only a note), as ok, warn, skipped or missing (exit 5 with the count if any is missing; warn and skipped never fail). Slack and release are optional (F11): without a `[slack]` table in `.bosun/bosun.toml`, step 7 is skipped, `data.notify` is `{skipped: true, reason}`, the janitor prompt leaves out `bosun notify` and there is no notify Routine; a `[slack]` table with an empty `channel` is missing. Release counts as configured when hoist.toml has `[release]` or `[publish]`, a workflow is `release.yml`/`release.yaml` or names a `release*` environment, the repo has a publishable package (a Cargo `[package]`, at the root or a workspace member, without `publish = false` or `publish = []`, or a `package.json` without `"private": true`), or the repo has a release; under `--remote`, an item of hoist's release, npm or crates areas (hoist inferred a publish target) also counts, and `data.hoist.release_inferred` says so. Else step 10 is skipped and `--remote` puts hoist's other release items (a `release*` environment, a secret such as `HOIST_RELEASE_TOKEN`) that are not ok into `data.hoist.skipped`, failing no step. It cannot see Routines, so step 11 is missing unless the owner sets `BOSUN_ROUTINES_OK=1`; the Routine prompts are in `data.routines`. Step 6 also returns `content` (the text of `docs/setup-script.sh`, `null` when missing) and `environment` (`keelstack`, the shared cloud environment whose setup script it is), so it can be pasted without opening the file. Step 2 counts a login that starts with `claude`, or an issue, PR or comment whose `performed_via_github_app.slug` does (the app acts as the owner), as the app; step 3 also reads the checks the default branch enforces (rulesets `GET /rules/branches/{b}` plus classic protection) and is missing when a check of hoist.toml is not among them, while 403 on both (a private repo on a free plan has no rulesets) only adds a "cannot verify" note, and so does 403 on one of them (classic protection often answers 403 while rulesets answer 200) when the other lacks a listed check; the note names the unreadable endpoint; step 4 accepts any non-empty `owner`/`owners` key in `governance/owners.toml`. `bosun doctor --remote` also runs `hoist check --json` (binary from `BOSUN_HOIST`, killed with its process group after 120 s, or `BOSUN_HOIST_TIMEOUT_SECS`) and maps its items to steps: 3 for github settings, rulesets and checks, 5 for release and environments (and github environments, secrets, variables), 10 for npm and crates first publish and trusted publishers. Drift makes a step missing and keeps its local problem and fix. `data.hoist` lists `drift`, `unknown` (items hoist could not read, which fail no step) and `notes` (failed verifies, which fail no step) (D17). `--remote` also checks the environment of every workflow job (each `environment:` name, plain, mapping or flow mapping, in `.github/workflows/*.yml`; a job whose environment is an expression or names none warns "cannot read the environment of job J in F"): `GET /environments/{name}`, and `GET /environments/{name}/deployment-branch-policies` when its policy is custom. Only custom branches that are exactly `main` (type `branch`) pass; a missing environment, a null policy (any branch), `protected_branches` or any other rule makes an ok step 5 `warn` (this never fails a step) with the Settings → Environments → <name> → Deployment branches → Selected branches → add `main` steps as the fix, and a 403 warns "cannot verify" (grant the token Environments: read, or Administration: read) and another error says to retry. `data.environments` lists `{name, status, detail, fix}` for each, and `{job, status, detail, fix}` for a job it cannot read.

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
