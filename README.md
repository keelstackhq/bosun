# bosun

Orchestration helper for repos where agents write all the code.

bosun is one static Rust binary in the keelstackhq family (splice, hull, hoard, hoist, berth). It
owns the process logic of an agent-built repo: claims and leases, spawn limits, waves, gates, risk
checks, review verdicts, lessons and cost. A Claude Code session calls `bosun <command>` and acts
on the one JSON object it prints. GitHub issues, labels, comments and PRs are the only shared
state.

Status: seed done; v0.1.0 is prepared (see `CHANGELOG.md`, `docs/PROGRESS.md`).

## Install

After the first release (see `docs/releasing.md`; until then build from source):

```bash
npm i -g @keelstackhq/bosun     # launcher; the binary comes from a platform package
cargo install keel-bosun        # installs the binary `bosun`
# or download bosun-<target> from GitHub Releases (SHA256SUMS next to it)
```

From source (Rust 1.97): `cargo install --path .` (package `keel-bosun`, binary `bosun`).

## Use

```text
bosun claim 42 --touch 'src/core/**'   # exit 3 if a live lease overlaps
bosun wave plan 7                      # tasks that can run in parallel now
bosun gate test                        # runs the profile's test command on affected packages
bosun pr --auto-merge                  # turns on auto-merge after all gates are green
```

Exit codes: 0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit.

- Design: [docs/spec.md](docs/spec.md)
- Default values and their evidence: [docs/defaults.md](docs/defaults.md)
- Plan format: [docs/plan-format.md](docs/plan-format.md)

## License

MIT
