# Plan: D-021 no gated cap

project: bosun

D-021 (keelstackhq/decisions#21) supersedes D-016's 400-line, one-task, alone rule for gated
PRs. Chunked review, owner approval of gated paths and the 2x drift note stay.

## D21: Gated PRs have no line cap
area: plan

Drop the gated cap and the gated bundle exclusion from plan lint, shape, wave, claim,
`bosun pr --bundle` and risk-check (R-bundle), with the docs that teach them.

### Tasks

- T1: drop the 400-line, one-task, alone rule for gated work (D-021)
  touch: src/cli/claim.rs, src/cli/metrics.rs, src/cli/plan.rs, src/cli/pr.rs, src/cli/pr_bundle.rs, src/cli/review_pack.rs, src/cli/risk_check.rs, src/cli/wave.rs, src/core/meta.rs, src/core/plan.rs, src/core/risk.rs, src/core/shape.rs, src/core/wave.rs, tests/**, AGENTS.md, CLAUDE.md, docs/defaults.md, docs/plan-format.md, docs/spec/**, plans/**
  risk: high
  size: 400
  allow: delete-test, fewer-asserts

### Done when

- UC-D21-1: a gated task of any size plans, bundles, claims and opens its PR like any task; risk-check still rates its gated paths high.
