# Modes booster Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a mode ladder (`gatekeeper` → `steady` → `booster` → `autonomous`) in
`.bosun/bosun.toml [mode]`, plus the boosters it switches on: gates before push, self-heal,
daily approval digest, auto spawn/archive, feature budget, metrics with brake, curator proposals.

**Spec:** `docs/superpowers/specs/2026-11-11-modes-booster-design.md`

**Tech Stack:** Rust, existing crates only. No workflow changes.

## Global Constraints

- Default stays `gatekeeper`: a repo without `[mode]` behaves byte for byte as today. Every PR
  has a test that proves it for the knob it adds.
- Gated PRs (any risk path, R-gate included) are one task and at most 400 non-test lines
  (D-016). Low-risk PRs have no cap (D-016; up to 1,500 lines while D-013 bootstrap runs).
- Each PR: `changelog.d/<branch>.md` and `docs/progress.d/<branch>.md` (branch with `/` as `-`),
  and the spec section it changes. Plan ticks go in one batched docs PR by the orchestrator
  (this file is R-gov).
- bosun never calls remote tools: spawn, archive, message and notify are calls in `next` (D29).
- Fast checks before every commit; the full gate loop (AGENTS.md step 4) before the one push.

## PRs in order

Size = non-test changed lines, estimated. "Owner" = needs an owner approval in every mode: it
matches R-weaken or R-gov, or it is gated `src/**` in bosun (critical floor,
`APPROVAL_CODE_PATHS`, turns every delegate match into an owner match). "Gated" = risk-check
high; one task, at most 400 lines.

### PR 1a — `[mode]` parse and registrations (gated, owner)

Touch: `src/config.rs`, `src/core/mod.rs`, `src/core/mode.rs` (new), `src/lib.rs`,
`src/cli/mod.rs`, `src/cli/mode.rs` (new), stubs `src/cli/{heal,digest,metrics}.rs` and
`src/core/{heal,digest,metrics}.rs`, `tests/pinned_config.rs`, `docs/spec/18-modes.md` (new),
`docs/spec.md`. Size ~350.

- [ ] `[mode]` parses (`name` + helper-knob overrides); unknown mode or knob, and `policy`
  under `[mode]`, are parse errors.
- [ ] `core::mode`: presets table, resolve (explicit > preset > default) with source per knob,
  `brake_active(labels)` helper (used by PRs 3 and 6, so they need not touch mode.rs).
- [ ] Register `mode`, `heal`, `digest`, `metrics` and `wave tick` in `src/lib.rs` and
  `src/cli/mod.rs` with their final arg structs (`--days`, `--propose`, `--answer`, ...), so
  PRs 3-8 never touch either file; stubs exit 1 with a `fix` naming the plan PR.
- [ ] `bosun mode` prints effective knobs, sources, brake.

### PR 1b — policy resolution, weaken, deny set, rules (gated, owner)

Touch: `src/config.rs`, `src/core/weaken.rs`, `src/core/risk.rs`, `src/core/risk_default.toml`,
`.bosun/risk.toml`, `docs/spec/08-risk.md`, `docs/defaults.md`. Size ~250. After 1a.
R-gov (`.bosun/**`).

- [ ] `Config::parse` fills `approvals.policy` from the preset when the key is unset;
  `data.policy_source` in risk-check and `approve` names explicit or preset.
- [ ] weaken: mode down the ladder, `push_gates` push → ready, `brake` on → off are findings.
- [ ] `AUTONOMOUS_DENIED_RULES` gains R-gov (test: an allowlisted R-gov file goes to the owner).
- [ ] R-gate (both rule files) gains `src/core/mode.rs`, `src/core/digest.rs`,
  `src/cli/digest.rs` (with `/**`); `tests/governance.rs` keeps the files equal.

### PR 2 — gates before push (gated, owner)

Touch: `src/hooks/pre_tool_use.rs`, `src/core/guard.rs` (push parsing: `cd x &&`, `-C`,
scripts), `docs/spec/07-gates.md`. Size ~200.

- [ ] `push_gates = "push"`: refuse `git push` when the pushed tree lacks a passing stamp for a
  CI gate of the profile; `fix` is the gate loop. `ready`: no change.

### PR 3 — self-heal: red PR and red main

Touch: `src/core/heal.rs`, `src/cli/heal.rs`, `docs/spec/09-main-health.md`. Size ~450.

- [ ] `bosun heal`: failing `ci-ok` heads → `send_message` (notify) or `create_session` fixer
  for a dead lease (spawn); at most 2 rounds; existing stop rules apply; then needs-human.
- [ ] Open main-red → first bad PR (main-triage or the only merge since green) → `bosun revert`;
  `revert-auto` marks it ready with auto-merge once a verdict is in. Brake caps at notify.

### PR 4 — daily approval digest (gated, owner)

Touch: `src/core/digest.rs`, `src/cli/digest.rs`, `docs/spec/14-prereqs.md`. Size ~350.

- [ ] One D-012 form: owner-waiting PRs (repo, PR, head SHA, rule, why), open asks, budget asks.
- [ ] Parse the answer ("approve 12 15", "raise 7 +20"); a moved head drops out. Mainfix and
  release taps bypass. Reuses the `approve --list` query (if a helper must go pub, that line
  goes in PR 1a). Gate code after 1b (R-gate). A "raise" counts only with a verified
  enrolled-owner signature (as `answer --chat`).

### PR 5 — metrics and brake

Touch: `src/core/metrics.rs`, `src/cli/metrics.rs`, `docs/spec/18-modes.md`, `docs/defaults.md`.
Size ~450. Run early: it measures the gatekeeper baseline.

- [ ] Lead time, red-CI rate, cost per merged PR, owner touches per day, blocks per rule with
  outcome (`caught`/`passed`).
- [ ] Brake: set/remove `bosun:brake` on the registry issue (thresholds in defaults.md).

### PR 6 — auto spawn and archive

Touch: `src/core/wave.rs`, `src/cli/wave.rs`, `src/core/spawn.rs`, `src/cli/spawn.rs`,
`docs/spec/05-spawn.md`. Size ~350.

- [ ] `bosun wave tick <epic>`: `create_session` per ready entry while spawn checks pass and no
  brake; `archive_session` for children with a recorded result and a merged or closed PR.

### PR 7 — budget per feature (gated, owner)

Touch: `src/core/plan.rs`, `src/core/meta.rs`, `src/core/budget.rs`, `src/cli/budget.rs`,
`src/cli/claim.rs`, `src/cli/spawn.rs`, `docs/plan-format.md`, `docs/spec/09-main-health.md`.
Size ~350. After PR 3 (spec 09) and PR 6 (`src/cli/spawn.rs`).

- [ ] `budget_usd:` per item; `plan import` estimates when missing from the budget report;
  `plan check` (no GitHub) uses the `per_task_usd` fallback. `warn` at 80 %; `ask` also
  exits 5 at 100 % until an owner "raise" answer.

### PR 8 — curator loosening proposals

Touch: `src/core/metrics.rs`, `src/cli/metrics.rs`, `docs/spec/06-learning.md`. Size ~250.
After PR 5.

- [ ] `bosun metrics --propose`: rules with ≥ 10 blocks and no catch in 30 days, never the floor
  rules; draft PR body text with evidence. The PR it leads to is R-weaken (owner).
- [ ] Blocked (owner): curator Routine prompt runs it weekly when `curator_loosen = "propose"`.

### PR 9 — bosun moves to `steady` (owner)

Touch: `.bosun/bosun.toml`, `docs/PROGRESS.md`. Size ~5. After a release with PRs 1a-6 is pinned
in `.bosun/version` (owner release). R-gov and R-weaken. `booster` only after two weeks of
`steady` metrics on target.

### O1 (optional, owner) — non-gate command registry

Touch: `src/lib.rs`, `src/cli/mod.rs`, `src/cli/commands.rs` (new), `.bosun/risk.toml`,
`src/core/risk_default.toml`. Size ~200. Moves non-gate command registration out of the R-gate
file so new features stop being gated. Changes the gate boundary: R-gov and R-weaken; only if
the owner takes spec open point 2.

## Parallel waves

| Wave | PRs | Why no shared file |
| ---- | --- | ------------------ |
| A | 1a, then 1b | 1a registers every module, command and arg; 1b shares `src/config.rs` |
| B | 2, 3, 4, 5, 6 | disjoint touch: hooks; heal; digest; metrics; wave/spawn. Spec files 07, 09, 14, 18, 05 |
| C | 7 (after 3, 6), 8 (after 5) | 7 shares spec 09 and `cli/spawn.rs`; 8 shares metrics files |
| D | 9, O1 | owner; 9 after a release |

Count: 10 PRs (+ O1 optional). Owner approval: 1a, 1b, 2, 4, 7, 9, O1. Low risk: 3, 5, 6, 8.
