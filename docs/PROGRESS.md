# Progress

## Status

Seed in progress, plan `docs/superpowers/plans/2026-10-22-bosun-seed.md`. Order: docs, then S1,
S2, S3, S8, then S4-S7, S9, S10. Each task is one PR of at most 400 changed lines; the
orchestrator merges.

## Done

- 2026-10-22 · Docs 1 · `docs/spec.md` (forge spec renamed to bosun, adjustments A1-A14 as D13-D26, seed design decisions D27-D31) and minimal CI · PR #1
- 2026-10-22 · Docs 2 · `docs/defaults.md`, `docs/plan-format.md`, CLAUDE.md, README, this file · PR #2
- 2026-10-22 · Docs 3 · seed plan `docs/superpowers/plans/2026-10-22-bosun-seed.md`: S1-S10 as Tasks 1-19, one PR each · PR #3
- 2026-10-22 · Task 1a · S1: package `keel-bosun`, envelope and exit codes (`src/output.rs`), `deny.toml` (rustls only), CI job `rust` (fmt, clippy, nextest, deny) under `ci-ok` · crates pinned: clap 4.6.7, serde 1.0.229, serde_json 1.0.154, thiserror 2.0.21, toml 1.1.8, assert_cmd 2.2.2, predicates 3.1.4, tempfile 3.27.0 · `cargo nextest run` 5/5, `cargo deny check` ok
- 2026-10-22 · Task 1b · S1: binary, `lib::run`, clap skeleton of join, claim, heartbeat, release, reap, status, hook, doctor; usage errors (also a missing subcommand) exit 2 with one envelope · `cargo nextest run` 12/12 (token canary absent)
- 2026-10-22 · Task 1c · S1 done: the other 14 commands (spawn, wave plan, wait, msg, gate, risk-check, test-integrity, review-pack, verdict post, pr, ci-log, ask, lesson add|hit, budget record); `--help` lists all 22 and each of the 24 calls returns one envelope · `cargo nextest run` 12/12
- 2026-10-22 · Task 3 · S2a: GitHub client (ureq 3 + rustls, retries, Link pagination), `Github` trait + `LiveGithub` for issues, labels, comments and variables, stateful tiny_http fake · `cargo nextest run` 29/29 (12 in tests/github_issues.rs), `cargo deny check` ok

## Blocked

Human-only items. Each has the exact action.

1. **Repo settings (checklist steps 3 and 5).** After S10 adds `hoist.toml`, run from a checkout
   of main with an admin token:
   `hoist plan --area github && hoist apply --area github --yes`. It sets branch protection and
   the merge queue on `main`, auto-merge, delete-branch-on-merge, and the required checks
   `ci-ok`, `risk-check`, `review-verdict`, `main-open` (from `[github]` in `hoist.toml`). hoist
   0.2 has no repo-variable resource (checked in its source on 2026-10-22), so set the kill switch
   by hand: `gh variable set BOSUN_HALT --body 0 --repo keelstackhq/bosun`. Until S10, require
   only `ci-ok`.
2. **Cloud environment setup script (checklist step 6, D23).** Set it in claude.ai/code →
   environment settings → setup script. The text is in `docs/setup-script.sh` (added by S10).
3. **Routines (checklist step 11).** After S9, `bosun doctor` prints the three prompts: janitor
   (every 15 min, Haiku), curator (daily, Sonnet), notify (every 15 min, Haiku).
4. **First publish (D24).** Not part of the seed. When the owner decides on v0.1.0: in a local
   checkout with npm and crates.io logins, `hoist release bootstrap --dry-run`, then
   `hoist release bootstrap`; then run the release workflow within 2 days so the npm trusted
   publishers do not expire.
5. **Org decision D-006 (D26).** Propose in keelstackhq/decisions: "Agents never merge or tag;
   merges happen via auto-merge after the gates." Agents do not edit that repo.
6. **Org decisions channel (D-004).** `splice channel add gh:keelstackhq/decisions//channel
   --track main --piece import-claude` in a checkout, then commit as `chore: sync org decisions`.
