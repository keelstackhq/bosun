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
- 2026-10-22 · Task 11a · S5a: `profiles/rust.toml`, `src/gates/profile.rs` (embedded profile, `.bosun/profiles/<name>.toml` override, `{files}`/`{diff}`/`{package}` expansion, budgets) · `cargo nextest run --locked` 28/28
- 2026-10-22 · Task 11b · S5a: `src/gates/mod.rs`, `src/cli/gate.rs` (`bosun gate <name> [--base] [--all]`: stacks from config, base from `--base`/`GITHUB_BASE_REF`/`origin/main`, git diff, command output on stderr, budget flag; works offline; per-package gates use `all` args until 11c) · `cargo nextest run --test cli_gate` 7/7, total 37/37
- 2026-10-22 · Task 11c · S5a done: `src/gates/packages.rs` (cargo-metadata adapter: longest manifest-dir prefix, workspace inputs mean all, docs ignored, `when_no_package = "skip"`) · `bosun gate test` after a change in `b/` runs `nextest run --locked -p b` only; `cargo nextest run --locked` 44/44
- 2026-10-22 · Task 12a · S5b: `src/gates/adapters.rs` (rust adapters as pure text functions: test files, `#[ignore` markers with UC ids as pending, assertion counts without comments) · `cargo nextest run --locked` 48/48
- 2026-10-22 · Task 12b · S5b done: `src/gates/integrity.rs`, `src/cli/test_integrity.rs` (`bosun test-integrity` and `bosun gate test-integrity`: deleted-test, new-skip, fewer-asserts, snapshot-change; renames are not deletions; PR body and allow list from `BOSUN_PR_BODY` and `BOSUN_INTEGRITY_ALLOW` until Task 13+) · `cargo nextest run --locked` 61/61
- 2026-10-22 · Task 5a · S3a: `src/core/glob.rs` (touch globs: match and overlap, `*` `?` `**`, `dir/`), `src/core/state.rs` (states, labels, transitions), `tests/core_is_pure.rs` · glob table 20 overlap and 13 match cases, total 34/34
- 2026-10-22 · Task 5b · S3a done: `src/core/records.rs` (comment records `{"bosun":"<type>"}`: lease, lease-end, join, leave, result, spend, ask, lesson-hit) and `src/core/lease.rs` (live leases, conflicts naming both globs, registry members; a bad glob blocks everything) · total 43/43
- 2026-10-22 · Task 8 · S4a: `src/core/spawn.rs` (checks 1-6 of spec §5 in order; main-red exit 4, budget/depth/live/children exit 6, conflict exit 3; mainfix skips 1-2) and `src/core/wave.rs` (ready, deps merged, no overlap with live leases or the wave, max_parallel; a reason for every waiting task) · total 51/51
- 2026-10-22 · Task 7a · S8: `src/core/guard.rs`, the pure rules of the PreToolUse hook: edit-lock (a path in another session's live lease), label-lock (`bosun:state/` outside `bosun` commands, Bash and GitHub MCP), merge-lock (D13: `gh pr merge`, merge/ref/tag APIs, `gh release create`, force/tag/delete pushes, pushes to the default branch, `git tag`, merge/commit on the default branch, MCP merge and auto-merge) · total 55/55
- 2026-10-22 · Task 3a · S2a: GitHub HTTP client (ureq 3 + rustls, retries, Link pagination, token-free errors) and issue/comment/label types · fixture parse tests and `enc`/Link unit tests pass, `cargo deny check` ok
- 2026-10-22 · Task 3b · S2a: stateful tiny_http fake (issues, labels, comments, variables; 2-per-page Link paging; records requests; fail_next) · tests/github_http.rs 9/9 through Http: 3-page paging, 500 then 200, 3x500, POST not retried, 404/401/403 texts, token canary, headers, 401 without auth
- 2026-10-22 · Task 3c · S2a: trait for issues, labels, comments and variables, with LiveGithub and its own PatchState for updates · tests/github_issues.rs 5/5: every trait method against the fake, 404 on label removal and 422 on label create tolerated
- 2026-10-22 · Task 6a · S3b: `src/app.rs` (App, kill switch, main-red, set_state, notes, active issues, end_lease, task meta), `src/core/meta.rs` (task marker and area label), `records::replace_record`, shared test helpers · unit tests pass, total 79/79
- 2026-10-22 · Task 6b · S3b: `src/registry.rs`, `bosun join` (one registry issue, join record edited in place) and `bosun heartbeat` (leases and join edited in place, no new comment) · tests/cli_join.rs 5/5
- 2026-10-22 · Task 6c · S3b: `bosun claim` (kill switch, main-red unless mainfix, ready only, idempotent re-claim, touch conflicts and area owner exit 3) · tests/cli_leases.rs claim cases pass
- 2026-10-22 · Task 6d · S3b: `bosun release` (lease-end, new state) and `bosun reap` (expired leases and members) · tests/cli_leases.rs 10/10, total 95/95 · S3 done: a claim with overlapping touch returns exit 3
- 2026-10-22 · Task 4a · S2b: pulls on the `Github` trait (get, list by head and state, files with all pages, create, update body, reviews) with `Pull`, `PrFile`, `Review` types and fake routes · tests/github_pulls.rs 6/6, total 50/50
- 2026-10-22 · Task 4b · S2b: commit statuses (newest per context, description cut to 140), check runs (all pages) and auto-merge via GraphQL (errors array becomes an error) with fake routes · tests/github_checks.rs 5/5, total 55/55
- 2026-10-22 · Task 4c · S2b done, S2 done: contents with a small base64 decoder, failed jobs and job logs (the signed log URL is fetched once, https, without the token or GitHub headers) · tests/github_contents.rs 6/6 and 4 b64 unit tests, total 65/65
- 2026-10-22 · Task 11a · S5a: `profiles/rust.toml`, `src/gates/profile.rs` (embedded profile, `.bosun/profiles/<name>.toml` override, `{files}`/`{diff}`/`{package}` expansion, budgets) · `cargo nextest run --locked` 28/28
- 2026-10-22 · Task 11b · S5a: `src/gates/mod.rs`, `src/cli/gate.rs` (`bosun gate <name> [--base] [--all]`: stacks from config, base from `--base`/`GITHUB_BASE_REF`/`origin/main`, git diff, command output on stderr, budget flag; works offline; per-package gates use `all` args until 11c) · `cargo nextest run --test cli_gate` 7/7, total 37/37
- 2026-10-22 · Task 11c · S5a done: `src/gates/packages.rs` (cargo-metadata adapter: longest manifest-dir prefix, workspace inputs mean all, docs ignored, `when_no_package = "skip"`) · `bosun gate test` after a change in `b/` runs `nextest run --locked -p b` only; `cargo nextest run --locked` 44/44
- 2026-10-22 · Task 12a · S5b: `src/gates/adapters.rs` (rust adapters as pure text functions: test files, `#[ignore` markers with UC ids as pending, assertion counts without comments) · `cargo nextest run --locked` 48/48
- 2026-10-22 · Task 12b · S5b done: `src/gates/integrity.rs`, `src/cli/test_integrity.rs` (`bosun test-integrity` and `bosun gate test-integrity`: deleted-test, new-skip, fewer-asserts, snapshot-change; renames are not deletions; PR body and allow list from `BOSUN_PR_BODY` and `BOSUN_INTEGRITY_ALLOW` until Task 13+) · `cargo nextest run --locked` 61/61
- 2026-10-22 · Task 13a · S6a: `src/core/risk.rs` (rules from TOML with compiled globs and regexes, `evaluate` over added lines and old or new paths, new-dependency diff of Cargo.toml and package.json, owners parse), `risk_default.toml` (the five spec rules), `capabilities` in task metadata, `regex` =1.13.1, spec D32 · `cargo nextest run --locked` 161/161
- 2026-10-22 · Task 13b · S6b: `src/cli/risk_check.rs` (`bosun risk-check <pr>`: rules, owners and manifests read at the base SHA, Risk note between markers replaced in place, `risk-check` commit status, exit 5 until an owner approves the head SHA), note and closing-issue helpers in `core::risk` · `tests/cli_risk.rs` 11/11, total 174/174
- 2026-10-22 · Task 13c · S6 done: `src/cli/ask.rs` (`bosun ask <issue> <question>`: kill switch, owners from `governance/owners.toml` on main, comment with `@owner` mentions and the `ask` record, state needs-human; no owners is exit 1 with fix) · `tests/cli_ask.rs` 4/4, `cargo nextest run --locked` 178/178 · S6 done
- 2026-10-22 · Task 9a · S4b: `bosun spawn` (kill switch, the checks 1-6 from registry, spend and lease facts, claims the epic with a touch-less lease, returns one `create_session` with the 3 tags; `[1m]` model is exit 2) and `spawn --record --from -` (child `join`, lease moved to the child) · tests/cli_spawn.rs 12/12, total 165/165, one refusal per check (exit 4, 6, 6, 6, 6, 3), mainfix passes 1-2
- 2026-10-22 · Task 9b · S4b done: `bosun wave plan <epic>` (tasks of the epic from issue metadata, merged set from closed or merged tasks of any epic, live leases on active issues, `waves.max_parallel`; each run entry has issue, title, plan id, touch and the `berth up <item>-<task>` workspace) · tests/cli_wave.rs 4/4, total 169/169
- 2026-10-22 · Task 10a · S4c: `bosun wait` (`--pr`, `--issue`, `--child`; snapshot then poll with backoff, base from `BOSUN_WAIT_BASE_MS`, cap 12x base; data changed, polls, before, after; timeout is exit 0; GET requests only) and `FakeGithub::after_requests` · tests/cli_wait.rs 7/7: PR merged after 2 polls gives changed true, polls 3; total 176/176
- 2026-10-22 · Task 10b · S4c done, S4 done: `bosun msg <session> <text> [--issue N]` (one `send_message` in `next`, prefix `From #N:` or `From <session>:`, over 500 characters or empty is exit 2, no GitHub calls) · tests/cli_msg.rs 3/3, total 179/179; S4 Done when met: every spawn check has a refusal test (exit 4, 6, 6, 6, 6, 3)
- 2026-10-22 · Task 7b · S8: `src/hooks/{mod,pre_tool_use,cache}.rs`, `bosun hook pre-tool-use` (hook protocol on stdout, no envelope; label-lock, merge-lock, edit-lock from a `<git common dir>/bosun/leases.json` cache, fails open) · `cargo nextest run --locked` 160/160
- 2026-10-22 · Task 7c · S8 done: `src/hooks/session_start.rs`, `bosun hook session-start` (registry join, cache refresh, one-paragraph context: session, origin, own leases, BOSUN_HALT, main-red; a context that says how to fix GitHub access when unreachable) · an Edit on another session's claimed path is denied; `cargo nextest run --locked` 162/162

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
