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

## Layout

```
src/main.rs                parse (usage errors -> 2), run, print one JSON object, exit code
src/cli/                   clap Cli and one module per command
src/output.rs              Envelope {version, command, ok, exit_code, data, error, next}; Exit
src/core/                  pure logic: leases, globs, limits, spawn checks, waves, risk, lessons
src/github/                ureq client and the Github trait (LiveGithub)
src/sessions/              next-call requests, get_session results, usage providers
src/gates/ src/plan/ src/hooks/   per the seed plan
tests/common/              fake GitHub (tiny_http, stateful), bosun() command builder
.github/workflows/ci.yml   job ci + ci-ok (the one required check of this repo until S10)
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
- One plan task per PR of at most 400 changed lines, with its PROGRESS line.
- Ask (in the PR or to the orchestrator) before adding a crate not in the plan's Tech Stack.
- Show real command output before claiming something works.
- Human-only items (repo settings, releases, Routines) go to Blocked in `docs/PROGRESS.md`.

## Gotchas

- `TMPDIR` may be `/`; set it to a writable dir before `cargo test`.
- The package is `keel-bosun`; the binary and the lib are `bosun`.
