# Progress

## Status

Seed in progress, plan `docs/superpowers/plans/2026-10-22-bosun-seed.md`. Order: docs, then S1,
S2, S3, S8, then S4-S7, S9, S10. Each task is one PR of at most 400 changed lines; the
orchestrator merges.

## Done

- 2026-10-22 · Docs 1 · `docs/spec.md` (forge spec renamed to bosun, adjustments A1-A14 as D13-D26, seed design decisions D27-D31) and minimal CI · PR #1
- 2026-10-22 · Docs 2 · `docs/defaults.md`, `docs/plan-format.md`, CLAUDE.md, README, this file · PR #2
- 2026-10-22 · Docs 3 · seed plan `docs/superpowers/plans/2026-10-22-bosun-seed.md`: S1-S10 as Tasks 1-19, one PR each · PR #3
- 2026-10-22 · Task 1a · S1: package `keel-bosun`, envelope and exit codes (`src/output.rs`), `deny.toml` (rustls only), CI job `rust` (fmt, clippy, nextest, deny) under `ci-ok` · crates pinned: clap 4.6.7, serde 1.0.229, serde_json 1.0.154, thiserror 2.0.21, toml 1.1.8, assert_cmd 2.2.2, predicates 3.1.4, tempfile 3.27.0 · guard test `tests/workflows.rs` (ci-ok needs and checks every job) · `cargo nextest run` 6/6, `cargo deny check` ok
- 2026-10-22 · Task 1b · S1: binary, `lib::run`, clap skeleton of join, claim, heartbeat, release, reap, status, hook, doctor; usage errors (also a missing subcommand) exit 2 with one envelope · `cargo nextest run` 12/12 (token canary absent)
- 2026-10-22 · Task 1c · S1 done: the other 14 commands (spawn, wave plan, wait, msg, gate, risk-check, test-integrity, review-pack, verdict post, pr, ci-log, ask, lesson add|hit, budget record); `--help` lists all 22 and each of the 24 calls returns one envelope · `cargo nextest run` 12/12
- 2026-10-22 · Task 2a · S1b: `src/time.rs` (`Timestamp`, RFC 3339 parse with offsets, display, `BOSUN_NOW`, `day_start`) · `cargo nextest run --test time` 3/3, total 17/17
- 2026-10-22 · Task 2b · S1b: `src/config.rs` (`.bosun/bosun.toml`, `.bosun/limits.toml` with spec §5 defaults, unknown keys are exit 1 naming file, line and key) · `cargo nextest run --test config` 4/4, total 21/21
- 2026-10-22 · Task 2c · S1b done: `src/ctx.rs` (`Ctx::load`: repo from `BOSUN_REPO` or the origin remote in https/scp/ssh form, session per D31 with `cse_` -> `session_`, origin from `BOSUN_ORIGIN` or `CLAUDE_CODE_CHILD_SESSION`, config from the git top level) · `cargo nextest run --test ctx` 4/4, total 25/25
- 2026-10-22 · Task 3a · S2a: GitHub HTTP client (ureq 3 + rustls, retries, Link pagination, token-free errors) and issue/comment/label types · fixture parse tests and `enc`/Link unit tests pass, `cargo deny check` ok
- 2026-10-22 · Task 3b · S2a: stateful tiny_http fake (issues, labels, comments, variables; 2-per-page Link paging; records requests; fail_next) · tests/github_http.rs 9/9 through Http: 3-page paging, 500 then 200, 3x500, POST not retried, 404/401/403 texts, token canary, headers, 401 without auth
- 2026-10-22 · Task 3c · S2a: trait for issues, labels, comments and variables, with LiveGithub and its own PatchState for updates · tests/github_issues.rs 5/5: every trait method against the fake, 404 on label removal and 422 on label create tolerated
- 2026-10-22 · Task 4a · S2b: pulls on the `Github` trait (get, list by head and state, files with all pages, create, update body, reviews) with `Pull`, `PrFile`, `Review` types and fake routes · tests/github_pulls.rs 6/6, total 50/50
- 2026-10-22 · Task 4b · S2b: commit statuses (newest per context, description cut to 140), check runs (all pages) and auto-merge via GraphQL (errors array becomes an error) with fake routes · tests/github_checks.rs 5/5, total 55/55
- 2026-10-22 · Task 5a · S3a: `src/core/glob.rs` (touch globs: match and overlap, `*` `?` `**`, `dir/`), `src/core/state.rs` (states, labels, transitions), `tests/core_is_pure.rs` · glob table 20 overlap and 13 match cases, total 34/34

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
