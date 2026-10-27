## 11. Seed and bootstrap

bosun pins its own release in `.bosun/version`. Its gates run the pinned release, never the binary
built from main, so a bad change to bosun cannot approve itself: job `ci` (step `bosun of the base`, a worktree of
the PR's base commit outside the checkout) reads the pin from the PR's base commit and installs `@keelstackhq/bosun@<pin>` from npm, checking that it reports that
version. A base that pins `unreleased` (the seed, see the seed plan S10) builds bosun from the base
commit instead. A step of the same job, before the gates, checks the pin a PR sets: that release must install and report its version. One
bootstrap exception: when the base has no self-gating CI yet (the PR that adds it, and PRs stacked
on it before it merges), the change builds bosun itself, and CI says so in a warning.

`bosun doctor` reports the 12 steps below as ok or missing (exit 5 with the count if any is missing). It cannot see Routines, so step 11 is missing unless the owner sets `BOSUN_ROUTINES_OK=1`; the three Routine prompts are in `data.routines`. Step 2 counts a login that starts with `claude`, or an issue, PR or comment whose `performed_via_github_app.slug` does (the app acts as the owner), as the app; step 3 also reads the checks the default branch enforces (rulesets `GET /rules/branches/{b}` plus classic protection) and is missing when a check of hoist.toml is not among them, while 403 on both (a private repo on a free plan has no rulesets) only adds a "cannot verify" note, and so does 403 on one of them (classic protection often answers 403 while rulesets answer 200) when the other lacks a listed check; the note names the unreadable endpoint; step 4 accepts any non-empty `owner`/`owners` key in `governance/owners.toml`. `bosun doctor --remote` also runs `hoist check --json` (binary from `BOSUN_HOIST`, killed with its process group after 120 s, or `BOSUN_HOIST_TIMEOUT_SECS`) and maps its items to steps: 3 for github settings, rulesets and checks, 5 for release and environments (and github environments, secrets, variables), 10 for npm and crates first publish and trusted publishers. Drift makes a step missing and keeps its local problem and fix. `data.hoist` lists `drift`, `unknown` (items hoist could not read, which fail no step) and `notes` (failed verifies, which fail no step) (D17).

Human checklist (12 steps):
1. Create the repo.
2. Install the Claude GitHub App on it.
3. Run `hoist apply` for branch protection, auto-merge (the merge queue follows in M3: `risk-check` and `review-verdict` are statuses on the PR head, which a queue commit does not carry), and the required checks
   `ci-ok`, `risk-check`, `review-verdict` and `main-open` (D15, D17).
4. Fill `governance/owners.toml`.
5. Run `hoist apply` for the environments and secrets (D17), and create the kill-switch label: `gh label create bosun:halt --repo <owner>/<repo> --color B60205`. The repo variable `BOSUN_HALT=0` is optional.
6. Create the cloud environment and its setup script (D23).
7. Create the Slack channel and connect Claude Tag.
8. Start the seed session with the seed brief.
9. Let the seed PRs merge through auto-merge on green gates (D13).
10. Run the first publish with `hoist release bootstrap`; later releases run `hoist release` in a
    workflow, which makes the tag (D13, D24).
11. Create the Routines: janitor (15 min, Haiku), curator (daily, Sonnet), notify (15 min, Haiku).
    `bosun doctor` prints their prompts.
12. Approve the plan PR and start the root session.

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
