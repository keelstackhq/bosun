# Plan: zero-touch onboarding

project: bosun

Implements docs/superpowers/specs/2026-11-02-zero-touch-onboarding-design.md (spec §6, tasks
B1-B7, H1, P1, P2, D1). Gated per D-013 (workflows, templates, governance, .bosun, approval code,
secrets, environments, releases) is marked `risk: high` and stays at most 400 lines; the rest is
low risk. Cross-repo tasks name their repo in the title. Their PR is opened in that repo, and the
bosun task only records it in `docs/cross-repo/<id>.md` (the plan format has no repo field).

## Prerequisites
- H1: SPLICE_BOT_TOKEN and CLOUDFLARE_API_TOKEN as secrets of environment provision in keelstackhq/provision
  on: computer
  steps: keelstackhq/provision > Settings > Environments > provision > Add secret (both); provision.yml copies them into each repo's environment secrets, never repo-level
  blocks: ZX/T3
- H2: Claude GitHub App on all repositories
  on: phone
  steps: GitHub > keelstackhq > Settings > GitHub Apps > Claude > Configure > Repository access: All repositories
  blocks: ZX/T3
- H3: keelstack provisioner app, installed on all repositories, key in provision
  on: computer
  steps: GitHub > keelstackhq > Settings > Developer settings > GitHub Apps > New (permissions per spec §3.1); install on All repositories; add the private key as secret PROVISIONER_KEY in environment provision of keelstackhq/provision
  blocks: ZX/T3
- H4: shared cloud environment keelstack with bosun's setup script
  on: computer
  steps: claude.ai/code > Environments > New "keelstack"; paste docs/setup-script.sh from main; no secrets
  blocks: ZH/T2
- H5: repo keelstackhq/provision with environment provision (deployment branch main only, no reviewer)
  on: computer
  steps: GitHub > New repository keelstackhq/provision (private); Settings > Environments > New provision > Deployment branches: main
  blocks: ZX/T3
- H6: merge D-014 in keelstackhq/decisions
  on: phone
  steps: review and merge the D-014 PR from ZX/T4 in keelstackhq/decisions
  blocks: ZA
- H7: release tap for bosun 0.5.0
  on: phone
  steps: GitHub > keelstackhq/bosun > Actions > release > Review deployments > release > Approve
  blocks: ZR/T3

## ZI: Intake file

area: onboarding
depends: seed

`.bosun/intake.toml`, its parser and signature, and `bosun init --intake` (spec §2, §3.3).

### Tasks

- T1: B1 core::intake parser, schema-only `bosun intake check`, `bosun intake sign` (detached .bosun/intake.sig) and `bosun intake verify`
  touch: src/core/intake.rs, src/core/mod.rs, src/cli/intake.rs, src/cli/mod.rs, tests/cli_intake_file.rs, docs/spec/03-layout.md
  risk: high
  size: 380
- T2: B3 channel seed `.bosun/intake.toml` and the `intake` key in bosun.toml
  touch: channel/pieces/core/piece.toml, channel/pieces/core/files/.bosun/intake.toml.jinja, src/config.rs, tests/channel_render_intake.rs
  depends: T1
  risk: high
  size: 150
- T3: B2 `bosun init --intake FILE` derives flags, copies the file, returns create_trigger in next
  touch: src/cli/init.rs, tests/cli_init_intake.rs, docs/spec/11-seed.md
  depends: T1, T2
  risk: high
  size: 300

### Done when

- UC-ZI-1: `bosun intake check` exits 2 on a bad value and 5 on a missing answer, naming the field in error.fix.
- UC-ZI-2: an unknown key in `.bosun/intake.toml` is an error.
- UC-ZI-3: `bosun intake verify` refuses a changed intake byte, another repo or mode, an expired signature (expiry at most 1 hour) and a key not enrolled in the given approvers file; `bosun intake check` reads no signature.
- UC-ZI-4: `bosun init --intake` with a missing answer exits 5 and writes nothing.
- UC-ZI-5: `bosun init --intake` with a flag that disagrees with the file exits 2.
- UC-ZI-6: `bosun init --intake` returns the janitor and curator create_trigger calls in next, and notify only when notify = "slack".
- UC-ZI-7: the existing `bosun intake <plan> <why>` form still opens a kind/plan issue.

## ZH: Handoff and relay

area: relay
depends: seed

`bosun handoff`, `bosun resume --handoff` and the relay rule (spec §5). T1 depends on ZI/T1
because both register a command in `src/cli/mod.rs`, which one task owns at a time. Both tasks
are gated: records and spawn are approval code (APPROVAL_CODE_PATHS).

### Tasks

- T1: B4 Record::Handoff, `bosun handoff` and `bosun resume --handoff`
  touch: src/core/records.rs, src/cli/handoff.rs, src/cli/resume.rs, src/cli/mod.rs, tests/cli_handoff.rs, docs/spec/05-spawn.md
  depends: ZI/T1
  risk: high
  size: 350
- T2: B5 relay: relay_waves, `bosun spawn --root --handoff`, environment_id and archive in next
  touch: src/cli/spawn.rs, src/cli/spawn_relay.rs, src/cli/mod.rs, src/cli/resume.rs, src/core/handoff.rs, src/core/records.rs, src/config.rs, .bosun/limits.toml, tests/cli_spawn_relay.rs, docs/spec/05-spawn.md, docs/defaults.md
  depends: T1, ZI/T2
  risk: high
  size: 300

### Done when

- UC-ZH-1: `bosun handoff` with more than 40 lines exits 6 and writes nothing.
- UC-ZH-2: `bosun resume --handoff` returns the newest handoff of the root lineage in data.handoff.
- UC-ZH-3: a handoff never creates or renews a lease.
- UC-ZH-4: `bosun spawn --root --handoff` returns one create_session call with the keelstack environment_id and the prompt `bosun resume --handoff`, plus archive_session for the caller, and calls neither itself.
- UC-ZH-5: relay_waves defaults to 6 and is read from `.bosun/limits.toml` on the default branch.

## ZA: Approval policy

area: approvals
depends: ZI

`[approvals] policy` after bootstrap mode (spec §4, D-014).

### Tasks

- T1: B6 `[approvals] policy` (owner, owner-gated, orchestrator) in approve and risk-check
  touch: src/cli/approve.rs, src/cli/risk_check.rs, src/core/approve.rs, tests/cli_approve_policy.rs, docs/spec/08-risk.md
  depends: ZH/T2
  risk: high
  size: 300

### Done when

- UC-ZA-1: with policy owner-gated, an orchestrator approval counts for a low-risk PR and not for an R-gov PR.
- UC-ZA-2: with policy owner, no orchestrator approval counts after bootstrap_until.
- UC-ZA-3: in every policy, a PR that touches environments, tags, releases, branch rules or secrets needs the owner.

## ZR: Doctor, release 0.5.0 and pin

area: release
depends: ZI, ZH, ZA

### Tasks

- T1: B7 spec §4 and §14 updates; doctor prints the setup script
  touch: src/cli/doctor.rs, tests/cli_doctor_setup_script.rs, docs/spec/04-lifecycle.md, docs/spec/14-prereqs.md
  risk: high
  size: 200
- T2: B7 release bosun 0.5.0 (version bump only)
  touch: Cargo.toml, Cargo.lock, npm/*/package.json, pnpm-lock.yaml, CHANGELOG.md
  depends: T1
  kind: bookkeeping
  risk: high
  size: 40
- T3: B7 pin 0.5.0 (.bosun/version, setup-script sha512, KNOWN)
  touch: .bosun/version, docs/setup-script.sh, tests/pinned_config.rs
  depends: T2
  kind: bookkeeping
  risk: high
  size: 30

### Done when

- UC-ZR-1: `bosun doctor` prints the content of docs/setup-script.sh in its setup-script step.
- UC-ZR-2: KNOWN in tests/pinned_config.rs lists 0.5.0 with the intake key, and the pinned-config test passes.

## ZX: Cross-repo work

area: cross-repo
depends: seed

Each task's PR is in the named repo. The bosun task adds `docs/cross-repo/<id>.md` with that
PR's link and merge commit once it merges.

### Tasks

- T1: [keelstackhq/hull] H1 `ready.mjs --assess --json` with a fixed output schema and tests
  touch: docs/cross-repo/H1.md
  kind: bookkeeping
  size: 10
- T2: [keelstackhq/hoist] P2 `hoist apply` with an app token (no PAT) and org secret grants
  touch: docs/cross-repo/P2.md
  kind: bookkeeping
  risk: high
  size: 10
- T3: [keelstackhq/provision] P1 provision.yml: verify, used-nonce record, mode rules, protected repos, hull, init
  touch: docs/cross-repo/P1.md
  depends: T1, T2, ZI/T3
  kind: bookkeeping
  risk: high
  size: 10
- T4: [keelstackhq/decisions] D1 file D-014 from the spec appendix
  touch: docs/cross-repo/D1.md
  kind: bookkeeping
  size: 10

### Done when

- UC-ZX-1: docs/cross-repo/H1.md, P1.md, P2.md and D1.md each name a merged PR in their repo.
- UC-ZX-2: provision.yml refuses an intake without a valid, unexpired orchestrator signature, and a nonce it already used, shown by tests in keelstackhq/provision.
- UC-ZX-3: provision.yml in mode new refuses an existing repo; in mode adopt it never removes or loosens a rule, environment or check; it refuses the protected repos and any repo with .bosun/.
- UC-ZX-4: provision.yml sets secrets only as environment secrets on splice-bot and release (main only), per publish target, never repo-level.
