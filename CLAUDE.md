# bosun

Orchestration helper for repos where agents write all the code. A session calls `bosun <command>`
and acts on the one JSON object it prints. One static Rust binary. Part of the keelstackhq family
(splice, hull, hoard, hoist, berth).

- Spec: `docs/spec.md` — source of truth. Read only the section you touch (§1 decisions and
  output, §4 lifecycle and leases, §5 spawn and limits, §7 gates, §8 risk); update it in the same
  PR when behaviour changes.
- Defaults and their evidence: `docs/defaults.md`. Plan format: `docs/plan-format.md`.
- Plans: `docs/superpowers/plans/` (the seed plan splits S1-S10 into PRs). Tick tasks as they land.
- Progress: `docs/PROGRESS.md` — Status, Done (one line per task), Blocked (human-only items).
- Never append to CHANGELOG.md or the Done list of `docs/PROGRESS.md` in a PR: add one-line fragments
  `changelog.d/<name>.md` and `docs/progress.d/<name>.md`, name = the branch with `/` as `-`
  (two PRs never edit the same file, so merging main never conflicts). Before a release, a docs
  PR to main runs `bash scripts/fold-fragments.sh`.

## Layout

```
src/main.rs lib.rs          parse (usage errors -> 2), run, print one JSON object (hooks print the hook protocol)
src/output.rs               Envelope {version, command, ok, exit_code, data, error, next}; Exit; BosunError
src/ctx.rs config.rs time.rs   identity (D31), .bosun/*.toml, RFC 3339 and BOSUN_NOW
src/app.rs registry.rs      App {ctx, gh}; kill switch, main-red, set_state, leases; the registry issue
src/cli/                    one module per command
src/core/                   pure logic: glob, state, records, lease, meta, spawn, wave, guard, risk, verdict, review, lessons
src/github/                 ureq client, Github trait, LiveGithub, base64
src/gates/                  profiles (profiles/rust.toml), runner, packages adapter, rust adapters, test-integrity
src/hooks/                  PreToolUse (edit-, label-, merge-lock), SessionStart, lease cache
src/sessions/               usage providers (claude-sessions, generic)
tests/common/               fake GitHub (tiny_http, stateful), bosun() command builder
.bosun/ governance/ .claude/ AGENTS.md REVIEW.md hoist.toml   bosun runs on itself
.github/workflows/ci.yml    one job `ci` (every gate) + ci-ok; risk.yml, main-open.yml, main-health.yml (S10)
```

## Commands

```bash
export PATH=/root/.cargo/bin:$PATH
cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo nextest run --locked
cargo deny check
cargo nextest run --test cli_claim   # one file
```

Run the fast checks before every commit.

## Rules that must never break

- **Exit codes:** 0 ok · 1 error · 2 usage · 3 conflict · 4 halted · 5 needs-human · 6 limit.
- **stdout is exactly one JSON object** `{version:1, command, ok, exit_code, data, error, next}`,
  also for errors and usage errors. `error.fix` is never empty. Logs go to stderr. Only `bosun hook`
  writes the Claude Code hook protocol instead.
- **Agents never merge, tag or force-push** (spec D13). bosun only turns on auto-merge.
- **bosun never calls claude-code-remote tools** (D29): it returns them in `next`.
- **`src/core` does no I/O** (no `std::fs`, `std::process`, `std::net`, no `crate::github`).
- **Tests never reach a real API.** Use `tests/common`: it clears `GITHUB_TOKEN`, `GH_TOKEN`
  and sets `BOSUN_GITHUB_API` to the fake. Time comes from `BOSUN_NOW`.
- **Never print tokens.** Tokens come from env, never argv.
- **rustls only**, no OpenSSL; `#![forbid(unsafe_code)]`; crates pinned exactly (`=x.y.z`).

## How to work here

- Test first: failing test, watch it fail, implement, watch it pass.
- One plan task per PR of at most 400 changed lines, with its progress fragment.
- Ask (in the PR or to the orchestrator) before adding a crate not in the plan's Tech Stack.
- Show real command output before claiming something works.
- Human-only items (repo settings, releases, Routines) go to Blocked in `docs/PROGRESS.md`.

## Gotchas

- `TMPDIR` may be `/`; set it to a writable dir before `cargo test`.
- The package is `keel-bosun`; the binary and the lib are `bosun`.

<!-- splice:decisions:begin — managed by `splice update --channel decisions`; edit the source -->

## Org decisions

Cross-repo decisions from keelstackhq/decisions; follow them. To change one, open a PR there.
`splice update --channel decisions` refreshes this snapshot; commit it alone as `chore: sync org decisions`.

@docs/agents/org-decisions.md

<!-- splice:decisions:end -->
