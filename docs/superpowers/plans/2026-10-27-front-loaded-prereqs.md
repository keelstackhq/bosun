# Front-loaded prerequisites: Implementation Plan

> **For agentic workers:** use superpowers:subagent-driven-development and
> superpowers:test-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** every human-only step of a request is asked for up front, as one phone-friendly
checklist; tasks it blocks stay planned until it is verified; owner approvals come in batches.

**Design:** `docs/superpowers/specs/2026-10-27-front-loaded-prereqs-design.md` (F1-F8).
Seed constraints hold: one task per PR of at most 400 changed lines, `src/core` does no I/O, tests
never reach a real API, one envelope on stdout, `error.fix` never empty, fragments instead of
CHANGELOG/PROGRESS edits. No workflow, `.bosun/` or governance change.

Stays out of the parallel-conflicts plan's files (`src/core/plan.rs`, `docs/plan-format.md`,
`AGENTS.md`, `docs/spec.md` until its split): docs that need those land last (Q4).

## Tasks

- [ ] **Q0: design, plan, format reference** (docs only)
  touch: the design, this plan, docs/prerequisites.md (new), fragments
- [ ] **Q1a: `core::prereqs` parser (F1)**
  touch: src/core/prereqs.rs (new), src/core/mod.rs, tests/prereqs_parse.rs (new), fragments
  - `parse(text, &Plan)`: the `## Prerequisites` section, validated against the plan.
  - `blocked_by(prereqs, item, task)`.
- [ ] **Q1b: approvals and the checklist (F3 approvals, F4 rendering)** — depends Q1a
  touch: src/core/prereqs.rs, tests/prereqs_checklist.rs (new), fragments
  - `approvals(&Plan, &Rules, bosun_code)`: per task owner/delegate at merge, batch = level.
  - `render_issue`, `ticks`, `sort`, `message`: the checklist as an issue body and a chat text.
- [ ] **Q2: `bosun prereqs` (F2, F3 setup items, F4)** — depends Q1b
  touch: src/cli/prereqs.rs (new), src/cli/mod.rs, src/lib.rs, src/github/mod.rs (`exists`),
  tests/cli_prereqs.rs (new), tests/common/fake.rs, fragments
- [ ] **Q3a: gating (F5)** — depends Q2
  touch: src/cli/prereqs_gate.rs (new: open items of `bosun:prereqs` issues per task),
  src/cli/plan.rs (`promote`, refresh before promote, `data.held`), src/cli/claim.rs (one call
  before the claim), src/cli/wave.rs (leave held tasks out), tests/cli_prereqs_gate.rs (new),
  fragments
- [ ] **Q3b: `bosun hold` (F6)** — depends Q3a
  touch: src/cli/hold.rs (new), src/cli/mod.rs (command and Args), src/lib.rs (dispatch),
  tests/cli_hold.rs (new), fragments
- [ ] **Q3c: approval batches (F7)** — depends Q2
  touch: src/cli/approve.rs (`data.message`), src/cli/intake.rs (next step text),
  tests/cli_approve_message.rs (new), fragments
- [ ] **Q4: docs (F6, F8)** — depends Q3b and P5, P7 of
  `docs/superpowers/plans/2026-10-27-parallel-conflicts.md` (branch `docs/parallel-conflicts-design`)
  touch: AGENTS.md, channel/pieces/core/files/sections/agents.md, docs/plan-format.md (link),
  docs/spec/<section> (new section), fragments
- [ ] **Q5: org docs** (other repos): D-009 in keelstackhq/decisions; playbook and Start here in
  keelstackhq/keelstack.

## Owner items

- Files shared with the parallel-conflicts plan: `src/cli/claim.rs` (its P2; Q3a adds one call
  line before the claim, outside P2's lease code), `src/github/*` (its P9 goes after Q2),
  `src/cli/mod.rs` and every command's Args (its P10 goes after Q2 and Q3b), `src/cli/plan.rs`,
  `src/cli/wave.rs` (P10 Args only). No function is edited by both in parallel; whichever lands
  second merges main.
- Every Q1-Q3 PR touches `src/**`, so each needs the owner's approval at merge (critical floor).
  They are meant to be approved as one batch.
