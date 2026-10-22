# bosun seed (S1-S10) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** bosun v0.1: the commands of spec §11 (join, claim, heartbeat, release, reap, spawn, wave plan, wait, gate, risk-check, test-integrity, review-pack, verdict post, pr, ci-log, status, ask, lesson, hook, doctor), plus msg and budget record, the governance files and the gate CI that runs bosun on itself.

**Architecture:** One package `keel-bosun` (D27). `src/core` is pure logic tested as units. `src/github` is the only network module (ureq, D28) behind a `Github` trait. Each command in `src/cli/` loads a `Ctx`, calls core and GitHub, and returns `Result<Outcome, BosunError>`; `main` prints one envelope and exits with its code. Tool calls the session must make come back in `next` (D29). Integration tests run the binary against a stateful fake GitHub (tiny_http) in `tests/common`.

**Tech Stack:** Rust 1.97 (`rust-toolchain.toml`), edition 2024. Crates, pinned with `=` (versions as in hoist and berth, checked with `cargo search` on 2026-10-22): `clap 4.6.7` (derive), `serde 1.0.229` (derive), `serde_json 1.0.154`, `thiserror 2.0.21`, `toml 1.1.8`, `ureq 3.4.2` (feature `platform-verifier`; rustls, no OpenSSL), `regex 1.13.1`. Dev: `assert_cmd 2.2.2`, `predicates 3.1.4`, `tempfile 3.27.0`, `tiny_http 0.12.0`. No tokio, no chrono (RFC 3339 by hand, as hoist `timeutil.rs`), no git2 (git CLI). Ask before adding any other crate.

**Spec:** `docs/spec.md`. Amendments ride in the PR of the task that needs them.

## Global Constraints

- One task = one PR of at most 400 changed lines (tests included, `Cargo.lock` excluded). A task that grows splits into `a`/`b` PRs.
- Exit codes and envelope as spec §1.1, §1.2: `{"version":1,"command","ok","exit_code","data","error","next"}`. `error` is `{"kind","message","fix"}` or `null`; `kind` is the exit name (`error`, `usage`, `conflict`, `halted`, `needs-human`, `limit`); `fix` is never empty. Usage errors (clap) exit 2 and still print one envelope with `command` = the first known subcommand in argv or `"unknown"`. `--help`/`--version` print clap text and exit 0.
- Commands that start or change work (claim, heartbeat, spawn, pr, verdict post, ask, lesson add/hit, budget record) first check the kill switch: repo variable `BOSUN_HALT` = `1` -> exit 4. A failed variable read is not a halt (logs a warning). `release` and `reap` are never halted (they free work).
- Main-red: an open issue with label `bosun:main-red`. claim and spawn refuse non-mainfix work with exit 4 while one is open.
- Identity, time and base URL as D30, D31. Tests set `BOSUN_REPO=acme/widget`, `BOSUN_SESSION=session_test`, `BOSUN_NOW=2026-10-22T10:00:00Z`, `BOSUN_GITHUB_API=<fake>`, `GITHUB_TOKEN=fake-token-canary`, remove `GH_TOKEN`. A test asserts the canary never reaches stdout or stderr.
- Machine records in comments are one line of JSON starting with `{"bosun":"<type>"`, the first such line of the body. Types: `lease`, `lease-end`, `join`, `leave`, `result`, `spend`, `ask`, `lesson-hit`.
- Issue metadata is one HTML comment in the issue body: `<!-- bosun:task {json} -->` with `{"item":"M1","task":"T1","epic":12,"touch":["src/core/**"],"depends":["M1/T2"],"kind":"feature","risk":"normal","allow":[]}` or `<!-- bosun:epic {"item":"M1","area":"lifecycle"} -->`. Plan import (M2) writes it; seed tests write it directly.
- Labels exactly as spec §4 (`bosun:state/ready` …). Only `core::state::set_state` changes a state label: it removes every other `bosun:state/*` label and adds the new one.
- The registry is the one open issue with label `bosun:registry` (created on first `join`). Join records live there; heartbeat edits the record in place (PATCH comment), never adds a comment.
- Status posts (D32, added to the spec by Task 13): `risk-check` and `review-verdict` are commit statuses (`POST /repos/{o}/{r}/statuses/{sha}`), not check runs, because only a GitHub App can create check runs and the session token may be a user token.
- `src/core` imports no `std::fs`, `std::process`, `std::net` and no `crate::github`; `tests/core_is_pure.rs` greps for them.
- Fast checks before each PR: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo nextest run --locked && cargo deny check`.
- Each PR adds its Done line to `docs/PROGRESS.md` (`- 2026-10-.. · Task N · <what> · <evidence>`) and ticks its boxes here.

## Order and dependencies

Docs (3 PRs) -> T1 -> T2 -> T3 -> T4 -> {T5 -> T6, T7 (S8)} -> {T8 -> T9 -> T10 (S4), T11 -> T12 (S5), T13 (S6), T14 -> T15 (S7), T16 -> T17 (S9)} -> T18 -> T19 (S10). Braces run in parallel when their base is merged.

---

### Task 1 (S1a): scaffold, envelope, exit codes, clap skeleton

Split into three PRs to stay near 400 lines: **1a** scaffold, `output.rs`, `deny.toml`, CI; **1b** `main.rs`, `lib::run`, the clap skeleton of the S3, S8 and S9 commands, `tests/cli_basics.rs`; **1c** the other commands.

- [x] 1a: scaffold, envelope with unit tests, deny, CI.
- [x] 1b: binary, usage envelope, 8 commands, `cli_basics.rs`.
- [x] 1c: the other 14 commands; S1 Done when holds.

**Files:** `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` (1.97, rustfmt, clippy), `deny.toml`, `.gitignore`, `src/main.rs`, `src/lib.rs`, `src/output.rs`, `src/cli/mod.rs`, `tests/common/mod.rs`, `tests/cli_basics.rs`, `.github/workflows/ci.yml` (job `rust`: fmt, clippy, nextest, deny; `ci-ok` needs `docs` and `rust`).

**Interfaces:**
- `output.rs`: `enum Exit { Ok, Error, Usage, Conflict, Halted, NeedsHuman, Limit }` with `code() -> u8` (0-6) and `name()`; `struct BosunError { exit, message, fix, data: Option<Value> }` with constructors `error/usage/conflict/halted/needs_human/limit(message, fix)`; `struct Next { tool: String, args: Value }`; `struct Outcome { data: Value, next: Vec<Next> }`; `fn envelope(command, &Result<Outcome, BosunError>) -> Value`.
- `cli/mod.rs`: `Cli { dir: Option<PathBuf> /* -C */, json: bool /* no-op */, verbose: bool, command: Command }`; `Command` has every v0.1 command (subcommand groups `wave plan`, `verdict post`, `lesson add|hit`, `hook pre-tool-use|session-start`, `budget record`); `Command::name()`.
- `lib.rs`: `pub fn run(cli: Cli) -> Result<Outcome, BosunError>`; unimplemented commands return `BosunError::error("<cmd> is not implemented yet", "it lands in seed task <N>; see docs/superpowers/plans/2026-10-22-bosun-seed.md")`.

- [x] Test first (`cli_basics.rs`): `--help` lists all 22 command names; each command (with minimal valid args) prints exactly one JSON object with all 7 keys; a bad flag exits 2 with `error.kind == "usage"`; `--version` exits 0; envelope unit tests for each `Exit`.
- [x] Implement; fast checks; PR.

**Done when:** `bosun --help` lists all v0.1 commands and each command returns JSON (S1).

### Task 2 (S1b): context, config, identity, time

Split into three PRs: **2a** `time.rs`, **2b** `config.rs`, **2c** `ctx.rs`.

- [x] 2a: time.
- [x] 2b: config and limits.
- [x] 2c: ctx and identity.

**Files:** `src/ctx.rs`, `src/config.rs`, `src/time.rs`, `tests/config.rs`.

- `time.rs`: `Timestamp(i64 /* unix secs */)`, `parse_rfc3339`, `Display` as `YYYY-MM-DDTHH:MM:SSZ`, `now()` from `BOSUN_NOW` else system time, `day_start()` (00:00 UTC).
- `config.rs`: `.bosun/bosun.toml` (`project`, `[[stack]] path/profile`, `[review] mode`, `[slack]`, `[prices]`), `.bosun/limits.toml` (spec §5 defaults when absent, missing keys take defaults), unknown keys are an error naming file and key (exit 1, fix names the key).
- `ctx.rs`: `Ctx { dir, repo: RepoId {owner, name}, session: String, origin: Origin, now, config, limits }`, `Origin { Spawned, Slack, Local, Routine }` from `BOSUN_ORIGIN` (default `local`, or `spawned` when `CLAUDE_CODE_CHILD_SESSION=1`); repo from `BOSUN_REPO` else `git remote get-url origin` (https, scp, ssh forms); D31 session rules.
- [ ] Tests: each origin form of the remote URL; `cse_` -> `session_`; limits defaults equal spec §5; unknown key error text; RFC 3339 round trip and `day_start`.

### Task 3 (S2a): GitHub HTTP client, issues, labels, comments, fake GitHub

**Files:** `src/github/{mod,http,types}.rs` (`LiveGithub` lives in `mod.rs`), `tests/common/fake.rs`, `tests/github_issues.rs`, `tests/fixtures/github/*.json` (recorded response shapes from the GitHub REST docs, trimmed).

- `http.rs`: ureq agent, base from `BOSUN_GITHUB_API` (default `https://api.github.com`), token from `GITHUB_TOKEN`/`GH_TOKEN` (missing -> exit 1, fix "export GITHUB_TOKEN"), `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, pagination through `Link: rel="next"`, retry 3 times on 5xx and secondary rate limits (sleep capped by `BOSUN_HTTP_MAX_SLEEP_MS` in tests), errors carry method, path, status and the API `message`, never headers or the token.
- `mod.rs`: `trait Github` (issues: `list_issues(labels, state)`, `get_issue`, `create_issue`, `update_issue`, `add_labels`, `remove_label`, `ensure_label`; comments: `list_comments`, `add_comment`, `update_comment`; `variable(name) -> Option<String>`); `LiveGithub` implements it.
- `fake.rs`: `FakeGithub::start()` serves the routes from in-memory state (`Arc<Mutex<State>>`: issues, labels, comments, pulls, files, statuses, checks, variables, contents by ref, reviews, actions runs), records every request (method, path, body), pages lists at 2 per page so pagination is tested. Seeds: `issue(n, title, labels, body)`, `variable(..)`, … State readable for asserts.
Split into three PRs: **3a** http and types, **3b** fake GitHub, **3c** Github trait.

- [x] 3a: `http.rs` and `types.rs`; fixtures parse; `enc` and Link unit tests.
- [x] 3b: fake GitHub; through `Http`: pagination across 3 pages, 500 then 200 retries, 3x500 gives up, POST not retried, 404 and 401 messages, documented headers, token canary never in an error.
- [x] 3c: `Github` trait and `LiveGithub`; each trait method against the fake.

### Task 4 (S2b): PRs, files, statuses, checks, contents, reviews, auto-merge, logs

**Files:** `src/github/{pulls,checks,contents}.rs`, `tests/github_pulls.rs`, fake routes.

- Trait additions: `get_pull(n) -> Pull {number, node_id, head_sha, head_ref, base_sha, base_ref, body, state, merged, draft, labels}`, `list_pulls(head: Option<&str>)`, `pull_files(n) -> Vec<PrFile {filename, status, additions, deletions, patch, previous_filename}>`, `create_pull`, `update_pull_body`, `list_reviews(n)`, `create_status(sha, context, state, description)`, `statuses(sha)` (combined), `check_runs(sha)`, `file_at(path, git_ref) -> Option<String>` (contents API, base64), `enable_auto_merge(node_id, method)` (GraphQL `enablePullRequestAutoMerge`, `POST /graphql`), `failed_jobs(sha)` and `job_log(job_id, tail)` (Actions API).
- Split into three PRs: **4a** pulls, files, reviews; **4b** statuses, checks, auto-merge; **4c** contents, jobs, logs.
- [x] 4a: `get_pull`, `list_pulls`, `pull_files`, `create_pull`, `update_pull_body`, `list_reviews` against the fake.
- [x] 4b: `create_status`, `statuses`, `check_runs`, `enable_auto_merge` (GraphQL `errors` array becomes an error with its message), each against the fake.
- [x] 4c: `file_at` (base64), `failed_jobs`, `job_log` (the signed log URL is fetched without GitHub headers), each against the fake.

**Done when (S2):** integration tests against the recorded fake pass for labels, issues, comments, PRs and checks.

### Task 5 (S3a): leases and touch globs (pure)

Split into two PRs: **5a** globs, states, purity guard; **5b** comment records, leases, registry members.

- [x] 5a: globs and states.
- [x] 5b: records, leases, members.

**Files:** `src/core/{mod,glob,lease,state}.rs`, `tests/core_is_pure.rs`.

- `glob.rs`: `Glob::parse` (`*` within a segment, `**` across, `?`; no braces), `matches(path)`, `fn overlaps(a: &[Glob], b: &[Glob]) -> Option<(String, String)>` (exact pair that overlaps; two globs overlap when some path matches both, decided segment by segment with `**` as a wildcard run).
- `lease.rs`: `Record` enum for the comment types; `parse_record(body) -> Option<Record>`; `Lease {issue, session, origin, expires, touch, comment_id}`; `fn live_leases(comments_by_issue, now) -> Vec<Lease>` (newest lease per session and issue with no later `lease-end`, not expired); `fn conflicts(touch, &[Lease], me) -> Vec<Conflict>`; `const TTL_SECS = 1800`.
- `state.rs`: `enum State` (spec §4 list), label names, `fn transition_allowed(from, to)` per the spec table, `fn state_of(labels)`.
- [ ] Unit tests: glob table (at least 20 cases incl. `src/**` vs `src/a/*.rs`, `a/*/c` vs `a/b/*`, disjoint); lease parsing ignores human text; expired and ended leases are not live; transitions.

### Task 6 (S3b): join, claim, heartbeat, release, reap

Split into four PRs: **6a** app helpers and task metadata; **6b** registry, join, heartbeat; **6c** claim; **6d** release and reap.

- [x] 6a: app helpers, `core/meta.rs`.
- [x] 6b: registry, join, heartbeat.
- [x] 6c: claim.
- [x] 6d: release, reap.

**Files:** `src/cli/{join,claim,heartbeat,release,reap}.rs`, `src/registry.rs`, `tests/cli_leases.rs`.

- `join [--origin O] [--area A] [--parent S] [--depth N]`: posts or refreshes this session's `join` record on the registry issue; `leave` on `release --all`.
- `claim <issue> [--touch GLOB]...`: kill switch, main-red (not for `kind: mainfix`), issue state must be `ready` (else exit 3 "issue is <state>"), touch defaults to the issue metadata, conflicts with live leases of other sessions -> exit 3 naming issue, session, both globs and the expiry; area owned by another live `join --area` session -> exit 3. On success: lease comment, state `claimed`, data `{issue, lease}`.
- `heartbeat`: PATCHes every live lease and the join record of this session to `now + TTL`.
- `release <issue> [--to ready|blocked|review]`: `lease-end` (`released`), state change.
- `reap`: every expired lease: `lease-end` (`reaped`), state back to `ready` if it was `claimed|building`; expired joins get `leave`. data lists what was reaped.
- [x] Tests: claim success; overlapping touch -> exit 3 (S3 Done when); same session re-claim is idempotent; expired lease does not conflict; area owner conflict; halt -> exit 4; main-red -> exit 4 unless mainfix; reap after `BOSUN_NOW` moves 31 min; heartbeat edits in place (comment count unchanged).

**Done when (S3):** a claim with overlapping touch returns exit 3.

### Task 7 (S8): hooks: edit-lock, label-lock, merge-lock, session-start

Split into three PRs: **7a** the pure rules in `src/core/guard.rs`; **7b** `hook pre-tool-use` with the lease cache; **7c** `hook session-start`.

- [x] 7a: guard rules.
- [x] 7b: pre-tool-use hook and lease cache.
- [x] 7c: session-start hook.

**Files:** `src/hooks/{mod,pre_tool_use,session_start,cache}.rs`, `tests/cli_hook.rs`, spec §4 note.

- `hook pre-tool-use` reads the Claude Code hook JSON on stdin (`session_id`, `tool_name`, `tool_input`, `cwd`). Deny = stdout `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"<why and fix>"}}`, exit 0; allow = no stdout, exit 0.
  - edit-lock: `Edit`, `Write`, `MultiEdit`, `NotebookEdit` on a path (made repo-relative) inside a live lease of another session -> deny with the issue, session and expiry.
  - label-lock: a `Bash` command or a GitHub MCP tool call (`mcp__github__*`) whose input contains `bosun:state/`, unless the Bash command starts with `bosun ` -> deny.
  - merge-lock (D13): `gh pr merge`, `git merge` into the default branch, `git push --force`/`-f`/`--force-with-lease`, `git tag`, `git push --tags`, `mcp__github__merge_pull_request` -> deny with "merges happen through auto-merge: run `bosun pr --auto-merge`".
- Lease cache: `<git common dir>/bosun/leases.json` with fetch time; refreshed when older than `BOSUN_LEASE_CACHE_SECS` (default 60). If GitHub is unreachable and the cache is older than the TTL, edit-lock allows and warns on stderr (fail open; label- and merge-lock need no network and always apply).
- `hook session-start`: runs the join logic, refreshes the cache, prints `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"<session, origin, own leases, halt and main-red state, next step: bosun status>"}}`.
- [ ] Tests: Edit on a claimed path of another session is denied (S8 Done when); own lease allowed; unrelated path allowed; label edit via Bash and MCP denied; `bosun release …` allowed; each merge-lock form denied; cache reuse (one GitHub read for two hook calls); stale cache with dead GitHub allows with a warning.

**Done when (S8):** an Edit on a claimed path is blocked.

### Task 8 (S4a): limits, spawn checks and wave selection (pure)

**Files:** `src/core/{limits,spawn,wave}.rs`.

- `spawn.rs`: `SpawnInput {main_red, kind, spent_today_usd, daily_usd, parent_depth, max_depth, live_sessions, max_live, children, max_children, conflict: Option<Conflict>}`; `fn check(&SpawnInput) -> Result<(), Refusal {check: u8, name, exit, message, fix}>` in spec §5 order (1 main-red -> exit 4 like claim; 2-5 -> exit 6; 6 -> exit 3). Mainfix skips 1 and 2.
- `wave.rs`: `fn plan(tasks: &[TaskInfo], leases, max_parallel) -> Wave {run: Vec<u64>, waiting: Vec<(u64, reason)>}`; a task runs when `ready`, all `depends` merged, no overlap with live leases or with tasks already picked; issue number order.
- [x] Unit tests: one refusal test per check with its exit code; mainfix exemption; wave with a dependency chain, an overlap pair and the parallel cap.

### Task 9 (S4b): spawn and wave plan commands

**Files:** `src/cli/{spawn,wave}.rs`, `src/sessions/{mod,request}.rs`, `tests/cli_spawn.rs`.

- `spawn <epic> [--model M] [--prompt-file F]`: gathers `SpawnInput` (registry joins, spend records of today on the registry issue, main-red, epic leases), runs the checks, claims the epic for this session, returns `next: [{"tool":"create_session","args":{"title","prompt","model","tags":["bosun:<project>","bosun:issue/<n>","bosun:parent/<session>"]}}]`. A model containing `[1m]` is a usage error.
- `spawn --record <epic> --from -`: reads the `create_session` result (`{"id": "session_..."}` or `{"session_id": …}`), posts the child's `join` (origin spawned, parent, depth + 1, area of the epic) and moves the epic lease to the child.
- `wave plan <epic>`: data `{run: [{issue, title, touch, workspace: "berth up <item>-<task>"}], waiting: [...]}` (D18).
- [x] Tests: each of the 7 checks refuses through the CLI with its exit (S4 Done when); success returns one `next` call with the 3 tags; `--record` registers the child.

Split into 2 PRs: **9a** spawn and `spawn --record`, **9b** wave plan.

- [x] 9a: `src/cli/spawn.rs`, `registry::write`, `tests/cli_spawn.rs`.
- [x] 9b: `src/cli/wave.rs`, `tests/cli_wave.rs`.

**Done when (S4, with Task 10):** each check has a refusal test with exit 6 (exit 4 and 3 where spec §5 says so).

### Task 10 (S4c): wait and msg

**Files:** `src/cli/{wait,msg}.rs`, `tests/cli_wait.rs`.

- `wait (--pr N | --issue N | --child SESSION) [--timeout SECS]`: snapshot (PR: state, merged, head SHA, statuses and check conclusions; issue: state label, open/closed, comment count; child: newest `result` record of the session) then poll with backoff 5 s doubling to 60 s (`BOSUN_WAIT_BASE_MS` scales it in tests) until it differs or the timeout passes. data `{changed, before, after, polls}`; timeout is exit 0 with `changed: false`. No send_later, no send_message (D14).
- `msg <session> <text> [--issue N]`: text over 500 characters is a usage error; returns `next: [{"tool":"send_message","args":{"session_id","message":"From #N: …"}}]`.
- [x] Tests: a fake that flips the PR state after 2 polls returns `changed: true, polls: 3`; timeout; child result; msg limits (in `tests/cli_msg.rs`).

Split into 2 PRs: **10a** wait (with a request hook in the fake), **10b** msg.

- [x] 10a: `src/cli/wait.rs`, `tests/cli_wait.rs`, `FakeGithub::after_requests`.
- [x] 10b: `src/cli/msg.rs` and its tests.

### Task 11 (S5a): gate runner and the rust profile

**Files:** `profiles/rust.toml`, `src/gates/{mod,profile,packages}.rs`, `src/cli/gate.rs`, `tests/cli_gate.rs`.

- Profile TOML: `[gates.<name>] cmd = [...]`, `each_package = ["-p", "{package}"]`, `all = ["--workspace"]`, `when_no_package = "skip"`, `budget_secs`; `[adapters] packages = "cargo-metadata"`, `pending = "rust-ignore"`, `assertions = "rust"`. Built-in profiles are embedded with `include_str!`; `.bosun/profiles/<name>.toml` overrides.
- `gate <name> [--base REF] [--all]`: the stack from `.bosun/bosun.toml` (default `rust` at `.`), the changed files `git diff --name-only <base>...HEAD`, affected packages (cargo-metadata adapter: `cargo metadata --no-deps --format-version 1`, longest manifest-dir prefix; a change to the root `Cargo.toml`, `Cargo.lock` or a non-package path means all), runs the command with stdio to stderr, data `{gate, stack, command, packages, exit_status, secs, over_budget}`; a failing command is exit 1 with fix "fix the failures above, then rerun `bosun gate <name>`".
- [x] Tests with a fake `cargo` on `PATH` that records argv: change in one of two packages -> `nextest run -p a` only (S5 Done when); root change -> `--workspace`; no package change -> skipped; failing command -> exit 1.

Split into three PRs: **11a** profile, **11b** runner, **11c** packages adapter.

- [x] 11a: `profiles/rust.toml`, profile parsing, repo override, placeholder expansion, unit tests.
- [x] 11b: runner, base and changed files, `bosun gate`, integration tests without the packages adapter.
- [x] 11c: cargo-metadata packages adapter and its tests (the box above).

**Done when (S5, with Task 12):** `bosun gate test` runs nextest on the affected packages only.

### Task 12 (S5b): test-integrity with the rust adapters

**Files:** `src/gates/{integrity,adapters}.rs`, `src/cli/test_integrity.rs`, `tests/cli_integrity.rs`.

- Rust adapters: test files are `tests/**/*.rs` and any `.rs` with `#[test]`; pending = `#[ignore` lines (a `#[ignore = "UC-…"]` marker counts as pending, not as a skip); assertions = count of `assert!`, `assert_eq!`, `assert_ne!`, `debug_assert*!`, `prop_assert*!`, `insta::assert_*!`.
- `test-integrity [--base REF] [--pr N]` (also `gate test-integrity`): compares base and head per test file: deleted test file, new non-UC `#[ignore`, lower assertion count -> refusal (exit 1, data lists each finding with file and counts) unless the task metadata of the PR's linked issue has the matching `allow` (`delete-test`, `skip`, `fewer-asserts`). A changed `*.snap` needs `snapshot-change: <reason>` in the PR body (from `--pr N`, or `BOSUN_PR_BODY`).
- [x] Tests on temp git repos: each refusal; the allowed case; UC marker removal passes; snapshot rule.

Split into two PRs: **12a** rust adapters; **12b** comparison and command.

- [x] 12a: `src/gates/adapters.rs` and its unit tests.
- [x] 12b: `integrity.rs`, `bosun test-integrity` and `gate test-integrity`, `tests/cli_integrity.rs` (the box above).

### Task 13 (S6): risk-check from the base branch, ask

**Files:** `src/core/risk.rs`, `src/cli/{risk_check,ask}.rs`, `.bosun/risk.toml` default text in `src/core/risk_default.toml`, `tests/cli_risk.rs`, spec D32.

- `core::risk`: rules `{id, why, paths, content, new_dependency, capability}`; `fn evaluate(rules, files: &[PrFile], new_deps: &[String], capabilities) -> Vec<Match>`; only escalates.
- `risk-check <pr>`: rules from `.bosun/risk.toml` **at the PR's base SHA** (`file_at`), built-in defaults when absent; new dependencies from `Cargo.toml`/`package.json` dependency tables at base vs head; capabilities from the task metadata. High: writes the Risk note between `<!-- bosun:risk -->` markers in the PR body, and passes only with an `APPROVED` review on the head SHA by a login in `governance/owners.toml` (read at base). Posts status `risk-check` (success / failure with the matched rule ids). Exit 0 normal or approved, exit 5 high and not approved.
- `ask <issue> <question>`: comment mentioning the owners with an `ask` record, state `needs-human`; exit 0.
- [x] Tests: a PR that edits `.bosun/risk.toml` (and weakens it at head) is high, judged by the base rules (S6 Done when); new crate -> R-deps; secret pattern; owner approval on an older SHA does not count; ask.

Split into three PRs: **13a** rules and evaluation (`core::risk`, owners, `capabilities` in task metadata); **13b** `risk-check`; **13c** `ask`.

- [x] 13a: `src/core/risk.rs`, `risk_default.toml`, `regex` pinned, spec D32.
- [x] 13b: `src/cli/risk_check.rs`, `tests/cli_risk.rs` (the box above, except ask).
- [x] 13c: `src/cli/ask.rs`, `tests/cli_ask.rs` (mentions, `ask` record, needs-human, no owners, halt).

**Done when (S6):** a PR that edits risk.toml is high, evaluated with the base rules.

### Task 14 (S7a): review-pack and verdict post

**Files:** `src/core/verdict.rs`, `src/cli/{review_pack,verdict}.rs`, `tests/cli_review.rs`.

- `review-pack <pr>`: data `{pr, head_sha, base_sha, since_sha, files (with patches), task, ucs, lessons, review_md}`; `since_sha` is the SHA of the last `review-verdict` status, so a second review is a delta (only files changed since; spec §7).
- `verdict post --from -`: validates the JSON (spec §7 shape); a stale `head_sha` (not the PR head) is exit 3; effective verdict is approve only with `verdict == "approve"`, `confidence >= 0.85` and no `blocker` finding; posts status `review-verdict` (success or failure, description with the confidence and the finding counts); `escalate_risk: true` sets the issue to `needs-human`. State label: approve -> `queued` is not set here (Task 15 does it after auto-merge), request-changes -> `fixing`.
- Split into 2 PRs: **14a** `core/verdict.rs`, `verdict post`, the `verdict` record; **14b** `review-pack` with delta reviews and `compare_files`.
- [x] 14a: 0.84 approve does not approve (S7 Done when); 0.85 approves; blocker blocks; stale SHA; escalate_risk.
- [x] 14b: lessons by scope, REVIEW.md from the base; delta `since_sha`.

**Done when (S7):** a verdict below 0.85 does not approve.

### Task 15 (S7b): pr with --auto-merge, ci-log

**Files:** `src/cli/{pr,ci_log}.rs`, `tests/cli_pr.rs`.

- `pr <issue> [--title T] [--auto-merge]`: finds the open PR for the current branch or creates it (body: `Closes #<issue>`, task marker), state `review`. `--auto-merge`: required statuses and checks on the head (`ci-ok`, `risk-check`, `review-verdict`, `main-open`) all success, else exit 1 listing what is missing with fix `bosun wait --pr N`; then `enable_auto_merge` (squash) and state `queued`. bosun never merges (D13).
- `ci-log <pr> [--tail N]`: failed jobs of the head SHA with the last N (default 80) log lines each.
- [ ] Tests: create vs reuse; auto-merge refused while `review-verdict` is pending; enabled when all are green (GraphQL call recorded); ci-log.

### Task 16 (S9a): lessons and status

**Files:** `src/core/lessons.rs`, `src/cli/{lesson,status}.rs`, `tests/cli_lesson.rs`.

- `lesson add <text> [--scope GLOB]... [--why W] [--evidence REF]...` writes `docs/lessons.toml` (next free `L-NNN`); at 40 entries or 8 globals -> exit 6 with fix "mechanize, merge or retire a lesson (spec §6)". `lesson hit <id> [--issue N]` increments `hits`, sets `last_hit`, and posts a `lesson-hit` record when `--issue`.
- `status`: data `{session, origin, halted, main_red, process_red, live_sessions, leases, issues_by_state, spend_today_usd, budget_pct, integrity: "full"|"reduced"}`.
- [ ] Tests: add, hit, cap at 40, global cap at 8, status with a seeded fake.

### Task 17 (S9b): doctor and usage providers

**Files:** `src/cli/{doctor,budget}.rs`, `src/sessions/usage.rs`, `tests/cli_doctor.rs`, `docs/defaults.md` prices.

- `doctor`: one item per checklist step (spec §11) with `ok` or `missing` and a fix: 1 repo readable; 2 Claude app (a PR or comment by `claude[bot]`/`claude` exists, else missing with the install link); 3 required checks named in `hoist.toml` and in workflows; 4 `governance/owners.toml` has an owner; 5 `BOSUN_HALT` variable exists; 6 setup script file present and the running binary matches `.bosun/version` (or no release yet); 7 `[slack] channel` set; 8 a registry issue exists; 9 auto-merge on in the repo settings; 10 a release exists; 11 the three Routine prompts (printed in data); 12 a `plans/*.md` and an open or merged plan PR. Exit 0 all ok, else exit 5 with the missing steps.
- `budget record <issue> --provider claude-sessions --from -` reads a `get_session` result and its `usage.cost_usd` (D20); `--provider generic --turns N --model M` estimates from `[prices]`. Posts `spend` records on the task issue and the registry issue.
- [ ] Tests: every step reported as ok or missing (S9 Done when); both providers; missing `usage.cost_usd` is exit 1 with the fix to use `generic`.

**Done when (S9):** `bosun doctor` reports each checklist step as ok or missing.

### Task 18 (S10a): governance files, Claude config, agent docs

**Files:** `.bosun/{bosun.toml,limits.toml,risk.toml,version}`, `governance/owners.toml`, `.github/CODEOWNERS`, `.claude/settings.json` (PreToolUse and SessionStart hooks to `bosun hook …`), `AGENTS.md`, `REVIEW.md`, `hoist.toml` (`[github] required_checks`, `merge_queue = true`), `docs/setup-script.sh` (D23).

- [ ] `bosun doctor` on the checkout reports steps 4, 6, 8 as expected; `.claude/settings.json` hook commands run against the fake in a test.

### Task 19 (S10b): gate CI on itself and the plan

**Files:** `.github/workflows/{ci,main-health}.yml`, `plans/bosun.md` (items M1-M8 in the plan format), PROGRESS.

- `ci.yml`: job `bosun-bin` builds bosun from the PR's **base** SHA (seed: no release yet, spec §11, Q5) and uploads it; jobs `gate-fmt`, `gate-lint`, `gate-test`, `gate-deps`, `gate-test-integrity` download it and run `bosun gate <name>`; Cargo caches through `keelstackhq/hoard@v0` (D19); `ci-ok` needs every `gate-*` job; jobs `risk-check` and `main-open` run `bosun risk-check` and `bosun gate main-open`; `merge_group` trigger.
- `main-health.yml`: on push to main and nightly; opens or closes the `bosun:main-red` issue.
- [ ] CI is green on this PR with the gates run by bosun (S10 Done when).

**Done when (S10):** CI is green on the S10 PR with the gates run by bosun.
