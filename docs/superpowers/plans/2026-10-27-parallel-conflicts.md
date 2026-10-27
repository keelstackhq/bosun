# Parallel conflicts: Implementation Plan

> **For agentic workers:** use superpowers:subagent-driven-development and
> superpowers:test-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** PRs opened in parallel never need a merge-conflict resolution: leases tell the truth
about the files a PR edits, plans show overlap before work starts, shared files are split, and a
clean merge of main costs no re-review.

**Design:** `docs/superpowers/specs/2026-10-27-parallel-conflicts-design.md` (C1-C8).
**Spec:** §4 (leases, claim check, edit hook), §8 (risk-check, signed approvals), §9 (main-health).
Seed constraints hold: one task per PR of at most 400 changed lines, `src/core` does no I/O, tests
never reach a real API, one envelope on stdout, `error.fix` never empty, fragments instead of
CHANGELOG/PROGRESS edits. No new CI job or run (`docs/ci-minutes.md`).

Each task lists its touch (whole files). Two tasks without `depends` between them have disjoint
touch lists. #251, #253 and #254 merged on 2026-10-27, and the owner approved P7 and P9, so P7
(the spec split) goes first: after it, a spec edit touches one section file
(`docs/spec/04-lifecycle.md` for P2 -> P3 -> P4, `08-risk.md` for P6 -> P9 (P9 also after P8, `src/github`), `09-main-health.md`
for P8), and only same-section tasks are chained.

## Tasks

- [ ] **P0: design and plan** (this PR)
  touch: docs/superpowers/specs/2026-10-27-parallel-conflicts-design.md,
  docs/superpowers/plans/2026-10-27-parallel-conflicts.md, fragments
- [ ] **P1: `core::touch`, plan overlap and hotspots (C4)**
  touch: src/core/touch.rs (new), src/core/mod.rs, src/core/plan.rs, docs/plan-format.md,
  tests/cli_plan_overlap.rs (new), fragments
  - `touch::companions(globs)`: `Cargo.toml` -> `Cargo.lock`, `package.json` -> `pnpm-lock.yaml`.
  - `touch::parallel_overlaps(tasks, depends)`: pairs that overlap with no depends path (transitive,
    across items) -> plan warnings with the fix `add "depends: X"`.
  - `touch::hotspots(tasks, 3)`: globs/paths matched by 3+ tasks -> plan warnings (so
    `src/cli/plan.rs`, which builds the output field by field, stays untouched).
- [ ] **P2: a lease grows on re-claim (C2)** — depends P7
  touch: src/cli/claim.rs, tests/cli_claim_grow.rs (new), docs/spec/04-lifecycle.md, fragments
  - `bosun claim N --touch G..` on an own active task: overlap check of the new globs against
    other live leases (exit 3), then a new lease record with the union; `data.added`. A lost race
    re-posts the old touch, so the session keeps its original lease.
- [ ] **P3: edit hook keeps edits inside own touch (C1)** — depends P1, P2
  touch: src/core/guard.rs (edit_lock), src/hooks/pre_tool_use.rs, tests/hook_edit_own_touch.rs (new),
  docs/spec/04-lifecycle.md, fragments
  - With an own live lease: deny an edit of an existing repo file no own lease covers, except
    fragments and companions; new files pass; fix names `bosun claim <issue> --touch <path>`.
- [ ] **P4: `bosun pr` refuses files outside the lease (C3)** — depends P1, P3 (spec §4)
  touch: src/cli/pr.rs, tests/cli_pr_touch.rs (new), docs/spec/04-lifecycle.md, fragments
- [ ] **P5: guidance (C5, C8)** — depends P1, P7 (AGENTS.md)
  touch: docs/plan-format.md, AGENTS.md, channel/pieces/core/files/sections/agents.md, fragments
- [ ] **P6: risk-check warns on out-of-touch files (C3)** — depends P4, P7
  touch: src/cli/risk_check.rs, tests/cli_risk_touch.rs (new), docs/spec/08-risk.md, fragments
- [ ] **P7: split docs/spec.md into docs/spec/ (C6)** — first; owner OK (a pure move over 400 lines)
  touch: docs/spec.md, docs/spec/** (new), CLAUDE.md, AGENTS.md, tests/channel.rs, fragments
- [ ] **P8: main-health flags conflicting PRs (C7)** — depends P7
  touch: src/cli/main_health.rs, src/github/types.rs, tests/cli_main_health_dirty.rs (new), docs/spec/09-main-health.md
- [ ] **P9: approval carry-over across a clean merge of main (C7)** — depends P6, P8; owner signed
  off the rule (R-high; an independent adversarial security review before merge, a test per bypass)
  touch: src/cli/risk_check.rs, src/core/approve.rs, src/github/* (trees API), tests/cli_carry_over.rs
  (new), docs/spec/08-risk.md
  - Implements exactly the five rules of design §3 C7. Required tests, one per case, each must
    fail to carry over (and the honest merge must carry over):
    1. honest merge of main into H, main touched only other files: carries;
    2. "take ours" merge that keeps H's blob for a file main also changed: no carry (rule 2);
    3. main renamed or deleted a path of P (X -> Z, X in P): no carry (rule 2, previous_filename);
    4. merge with an old main commit (M an ancestor of B): no carry (rule 1);
    5. a mode-only change (100644 -> 100755), a symlink or a gitlink change in H2: no carry (rule 3);
    6. an extra file or byte outside M and P in H2: no carry (rule 3);
    7. a dir/file clash between P and M: no carry (rule 3);
    8. an octopus merge or a non-merge commit on the path H..H2: no carry (rule 1);
    9. a truncated compare or tree, or a trust file in P: no carry (rule 4);
    10. the owner requested changes on H, or on a commit between H and H2: no carry (rule 5);
    11. a newer verdict on H or a later commit supersedes the carried one: no carry (rule 5);
    12. the PR's base is not the default branch: no carry;
    13. a rule tightened on main applies to H2 (the requirement is recomputed, only the
        verdict or signature is carried).
- [ ] **P10: `src/cli/mod.rs` Args split (C6)** — depends P1-P9, optional
  touch: src/cli/mod.rs, src/cli/*.rs (each command's Args)

## Owner items

- OK for P7 (a generated move over 400 lines) and the carry-over rule of P9 (design §3 C7).
- Nothing here changes `.github/workflows`, `.bosun/*`, governance or CODEOWNERS.
