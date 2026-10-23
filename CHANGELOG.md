# Changelog

## v0.1.0

The first release: the seed (S1-S10, `docs/superpowers/plans/2026-10-22-bosun-seed.md`).

- One JSON envelope on stdout for every command, fixed exit codes 0-6, `error.fix` always set.
- Identity, config and time: `BOSUN_REPO`, `BOSUN_SESSION`, `.bosun/*.toml`, `BOSUN_NOW`.
- Coordination on GitHub issues: `join`, `claim`, `heartbeat`, `release`, `reap`, `status`, with
  leases in the registry issue and the `BOSUN_HALT` kill switch.
- Spawning and limits: `spawn`, `wave plan`, `wait`, `msg`, `budget record`; the
  claude-code-remote calls come back in `next`.
- Gates: `gate` (rust profile), `test-integrity`, `ci-log`.
- Review and risk: `review-pack`, `verdict post`, `risk-check`, `pr --auto-merge`, `ask`.
- Learning loop: `lesson add`, `lesson hit`.
- Hooks: `hook pre-tool-use` (edit-, label- and merge-lock) and `hook session-start`.
- `doctor`, which also prints the janitor, curator and notify Routine prompts.

Install: `npm i -g @keelstackhq/bosun`, `cargo install keel-bosun`, or a binary from the
GitHub Release.
