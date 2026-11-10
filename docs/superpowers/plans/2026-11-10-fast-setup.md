# Fast cloud setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a fresh cloud container finishes bosun's setup script in seconds, not minutes.

**Architecture:** drop cargo-nextest from the default rust profile (use `cargo test`), make the
setup script skip stacks the checkout does not have, and ship cargo-deny's prebuilt binary as
`@keelstackhq/cargo-deny` npm packages installed by the existing pinned `install_tool`.

**Tech Stack:** bash, GitHub Actions, npm. No new crates.

**Spec:** `docs/superpowers/specs/2026-11-10-fast-setup-design.md`

## Global Constraints

- Each task is its own PR from `origin/main`, at most 400 changed lines (tests excluded), with
  `changelog.d/<branch>.md` and `docs/progress.d/<branch>.md` (branch with `/` as `-`).
- A failing setup script blocks every session: every new step warns and continues, never exits non-zero.
- Spec files updated in the same PR (§7 gates, §11 seed, `docs/setup-script.sh` header rules).
- Fast checks before every commit: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo nextest run --locked`.
- Agents never merge, tag or publish; first npm publish is an owner step (Blocked in `docs/PROGRESS.md`).

## Review Focus

- A repo with `.bosun/profiles/rust.toml` that still names nextest keeps working unchanged.
- A checkout with no `Cargo.toml` (ts-pnpm only) gets no rustup or cargo step at all.
- A checkout with both stacks gets both.
- Empty cargo-deny pin: script falls back to `cargo install --locked cargo-deny` and still never fails.
- `cargo test` runs doctests: a repo whose doctests were never run may go red; the spec says so.

---

### Task 1: default test gate is `cargo test`

**Files:** Modify `profiles/rust.toml` (`[gates.test] cmd = ["cargo", "test", "--locked"]`),
`src/gates/profile.rs:345` test, `tests/cli_gate.rs` (calls `test --locked -p b` etc.),
`tests/uc_m7.rs:189,203`, `channel/pieces/core/files/.github/workflows/bosun.yml.jinja:329`
(`tool: cargo-deny`) and `:565` (drop the nextest install step), `docs/spec/07-gates.md:34`,
plus other spec lines naming nextest as the default (`grep -rn nextest docs/spec`). Keep bosun's
own `ci.yml`, CLAUDE.md and `tests/gates_circleci.rs` behaviour unless their assertions break.

- [ ] Update the expected calls in the tests above to `test --locked ...`; run, watch them fail.
- [ ] Change the profile and the workflow template; rerun `tests/cli_gate.rs`, `tests/uc_m7.rs`, `tests/channel_render.rs`, `tests/workflows.rs`: PASS.
- [ ] Add a test: a repo override `.bosun/profiles/rust.toml` with the nextest cmd still calls `nextest run --locked`.
- [ ] Spec §7: default is `cargo test`; nextest by override; note doctests. Fragments. Commit, push `claude/test-gate-cargo-test`, open PR (R-weaken: owner approves).

### Task 2: setup script skips absent stacks and drops nextest

**Files:** `docs/setup-script.sh`, `channel/pieces/core/files/docs/setup-script.sh.jinja`,
`tests/channel_render.rs:1764-1779` and any setup-script tests (`tests/cli_doctor_setup_script.rs`).

- [ ] Tests: rendered/real script run with fake `cargo`/`rustup`/`npm` on PATH (pattern at `tests/channel_render.rs:1764`):
  (a) checkout with `Cargo.toml` → calls `cargo install --locked cargo-deny` and no `cargo-nextest`;
  (b) no `Cargo.toml` → no rustup and no cargo call; (c) no `pnpm-lock.yaml` and no `"packageManager": "pnpm@` in package.json → no pnpm install. Run: FAIL.
- [ ] Implement: `HAS_RUST` = `[ -f Cargo.toml ] || [ -f rust-toolchain.toml ]`; `HAS_PNPM` as above; guard the rustup/cargo block and the pnpm block. Add `CARGO_DENY_VERSION`/`CARGO_DENY_*_SHA512` variables (empty); when set, `install_tool cargo-deny ...`, else the `cargo install --locked cargo-deny` fallback. `install_tool`'s `bosun --version`-style checks unchanged.
- [ ] Same edit in the channel jinja (keep its `{% if "rust" in pieces %}` structure). Tests PASS. Spec §11 / setup-script docs. Fragments. Commit, push `claude/setup-skip-stacks`, open PR (gated).

### Task 3: `@keelstackhq/cargo-deny` npm packages

**Files:** Create `npm/cargo-deny/{package.json,bin/cargo-deny}` (launcher like `npm/bosun/bin`,
resolves `@keelstackhq/cargo-deny-linux-{x64,arm64}` and execs its `bin/cargo-deny`),
`npm/cargo-deny-linux-x64/package.json`, `npm/cargo-deny-linux-arm64/package.json`
(`os`/`cpu` fields, version = upstream cargo-deny version, e.g. `0.20.2`),
`.github/workflows/cargo-deny-npm.yml`, `docs/releasing.md` section, `docs/PROGRESS.md` Blocked item, test in `tests/workflows.rs` or a new `tests/npm_cargo_deny.rs`.

- [ ] Test: workflow is `workflow_dispatch` only with input `version`, job-level `environment: release`, `permissions: id-token: write, contents: read`, downloads `cargo-deny-<v>-{x86_64,aarch64}-unknown-linux-musl.tar.gz` and its `.sha256` from the EmbarkStudios release, runs `sha256sum -c`, and `npm publish --provenance --access public` for the two platform packages then the launcher; actions pinned by SHA like `release.yml`. Test the launcher resolves the platform binary (node test like `npm/bosun/test`). FAIL.
- [ ] Implement; PASS. Blocked item: owner creates the npm packages / trusted publisher for `cargo-deny-npm.yml` and runs it once. Fragments. Commit, push `claude/cargo-deny-npm`, open PR (gated: workflow).

### Task 4 (after the owner's first publish): pin cargo-deny in the setup scripts

- [ ] Set `CARGO_DENY_VERSION` and the three sha512 values (`npm view @keelstackhq/cargo-deny@<v> dist.integrity`, same for both platform packages) in both scripts; test that the install path is taken. PR `claude/pin-cargo-deny`.
