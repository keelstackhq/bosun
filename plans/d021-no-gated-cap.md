# Plan: D-021 no gated cap

project: bosun

D-021 (keelstackhq/decisions#21) supersedes D-016's 400-line, one-task, alone rule for gated
PRs. Chunked review, owner approval of gated paths and the 2x drift note stay.

## Prerequisites
- H1: merge keelstackhq/decisions#21 (D-021)
  on: phone
  steps: GitHub > keelstackhq/decisions > Pull requests > #21 > Merge
  blocks: DC21/T1

## DC21: Gated PRs have no line cap
area: plan
shape: split

Drop the gated cap and the gated bundle exclusion from plan lint, shape, wave, claim,
`bosun pr --bundle` and risk-check (R-bundle), with the docs that teach them. T1 syncs the org
decisions snapshot first, so the repo's copy of D-021 lands before the code that follows it.
T2 is about 620 changed lines outside tests; it has no `size:` because the bosun on main still
caps a gated task at 400 until T2 lands (the drift note is expected). T2 is R-weaken: owner only.

### Tasks

- T1: chore: sync org decisions (D-021), committed alone
  touch: docs/agents/org-decisions.md
  kind: bookkeeping
  size: 4
- T2: drop the 400-line, one-task, alone rule for gated work (D-021; R-weaken, owner-only)
  touch: src/cli/claim.rs, src/cli/metrics.rs, src/cli/plan.rs, src/cli/pr.rs, src/cli/pr_bundle.rs, src/cli/review_pack.rs, src/cli/risk_check.rs, src/cli/wave.rs, src/core/meta.rs, src/core/plan.rs, src/core/risk.rs, src/core/shape.rs, src/core/wave.rs, tests/cli_claim_bundle.rs, tests/cli_claim_subtree.rs, tests/cli_plan_import.rs, tests/cli_plan_shape.rs, tests/cli_plan_subtree.rs, tests/cli_pr_bundle.rs, tests/cli_pr_bundle_closes.rs, tests/cli_review_pack_drift.rs, tests/cli_risk_plan.rs, tests/cli_verdict_chunks.rs, tests/cli_wave_single.rs, tests/cli_wave_subtree.rs, tests/metrics_core.rs, tests/plan_parse.rs, tests/risk_bundle_gated.rs, tests/risk_bundle_title_commits.rs, AGENTS.md, CLAUDE.md, docs/defaults.md, docs/plan-format.md, docs/spec/04-lifecycle.md, docs/spec/05-spawn.md, docs/spec/08-risk.md, docs/spec/12-plan.md, plans/zero-touch-onboarding.md
  depends: T1
  risk: high
  allow: delete-test, fewer-asserts

### Done when

- UC-DC21-1: the org decisions snapshot carries D-021.
- UC-DC21-2: a gated task of any size plans, bundles, claims and opens its PR like any task; risk-check still rates its gated paths high and asks the owner.
