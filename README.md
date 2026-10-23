# bosun

Orchestration helper for repos where agents write all the code.

bosun is one static Rust binary in the keelstackhq family (splice, hull, hoard, hoist, berth). It
owns the process logic of an agent-built repo: claims and leases, spawn limits, waves, gates, risk
checks, review verdicts, lessons and cost. A Claude Code session calls `bosun <command>` and acts
on the one JSON object it prints. GitHub issues, labels, comments and PRs are the only shared
state.

Status: seed in progress. Nothing is released yet. See `docs/PROGRESS.md`.

```text
bosun claim 42 --touch 'src/core/**'   # exit 3 if a live lease overlaps
bosun wave plan 7                      # tasks that can run in parallel now
bosun gate test                        # runs the profile's test command on affected packages
bosun pr --auto-merge                  # turns on auto-merge after all gates are green
```

Exit codes: 0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit.

## Owner controls

- **Approve a high-risk PR.** Comment on the PR, as a login in `governance/owners.toml`, with
  exactly `/bosun approve <head sha>` (the full 40-character SHA of the current head). The
  `risk` workflow re-runs and `risk-check` turns green. A new push needs a new comment. A GitHub
  review that approves the head SHA counts too, when the owner did not open the PR.
- **Stop all work.** Create the label once:
  `gh label create bosun:halt --repo keelstackhq/bosun --color B60205`. Then open an issue with
  that label: `gh issue create --repo keelstackhq/bosun --label bosun:halt --title halt --body "<why>"`.
  Every command that starts or changes work exits 4 until you close the issue. The repo variable
  `BOSUN_HALT=1` also halts, where the token can read it.

- Design: [docs/spec.md](docs/spec.md)
- Default values and their evidence: [docs/defaults.md](docs/defaults.md)
- Plan format: [docs/plan-format.md](docs/plan-format.md)

## License

MIT
