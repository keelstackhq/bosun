# bosun: design spec

Date: 2026-10-22 · Author: Adam Urban (owner) · Status: accepted for the seed

Names: repo `keelstackhq/bosun`, binary `bosun`, Cargo package `keel-bosun` (lib `bosun`), npm
launcher `@keelstackhq/bosun`. Part of the keelstackhq family (splice, hull, hoard, hoist, berth).

bosun is a stack-agnostic orchestration helper for repos where agents write all the code. It is
one static Rust binary. It owns the process logic, so a session only calls bosun and acts on its
JSON output.

This spec is the "forge" spec of 2026-10-22 with the forge name changed to bosun and the owner's
adjustments applied (D13-D26). Where an adjustment and the original text disagreed, the text below
already follows the adjustment.

## Sections

One file per section, so PRs that change different sections never touch the same file. A
task that changes a section lists that file in its `touch`. `§N` in the text means section N.

- [1. Summary and decisions](spec/01-summary.md)
- [2. Scope and sub-projects](spec/02-scope.md)
- [3. Repo layout](spec/03-layout.md)
- [4. Work lifecycle and conflict avoidance](spec/04-lifecycle.md)
- [5. Spawning, waves and limits](spec/05-spawn.md)
- [6. Learning loop](spec/06-learning.md)
- [7. Verification and the gate contract](spec/07-gates.md)
- [8. Risk gating](spec/08-risk.md)
- [9. Main-health, cost and spend guard](spec/09-main-health.md)
- [10. Slack stream (Claude Tag)](spec/10-slack.md)
- [11. Seed and bootstrap](spec/11-seed.md)
- [12. Implementation plan](spec/12-plan.md)
- [13. Open questions](spec/13-open-questions.md)
- [14. Front-loaded human prerequisites](spec/14-prereqs.md)
- [15. Clean template updates (proposed, not built)](spec/15-template-updates.md)
