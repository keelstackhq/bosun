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

## Owner controls

- **Approve a high-risk PR.** Comment on the PR, as a login in `governance/owners.toml`, with
  exactly `/bosun approve <head sha>` (the full 40-character SHA of the current head). The
  `risk` workflow re-runs and `risk-check` turns green. A new push needs a new comment. A GitHub
  review that approves the head SHA counts too, when the owner did not open the PR. Post the
  comment yourself (web UI or `gh` with your own token): a comment that an agent session writes
  through the Claude app does not count, even under your login. To approve from a Claude chat,
  tell the orchestrator: it posts the same line with a `bosun-sig ed25519` signature line from a
  key enrolled in `.bosun/approvers.toml` on the default branch, and that counts whoever posts it
  (spec §8, signed approvals). Enroll or rotate a key with a PR to that file that you approve on
  GitHub.
- **Delegate decisions.** A key with `role = "delegate"` lets one Claude session approve PRs whose
  risk rules are all `approval = "delegate"`, with one clean review verdict on the head, while
  bosun is not halted. Critical changes (`.bosun/`, `governance/`, CODEOWNERS, every workflow,
  bosun's code and manifests, secrets, prod writes, an unseen diff) always need you (spec §8,
  delegated decisions); here, once the pin is 0.2.1, that leaves `deny.toml`, the toolchain files, `.config/` and
  `profiles/` to a delegate. To turn it off at once, delete the delegate key from
  `.bosun/approvers.toml` in a PR you approve.
- **Review verdicts from cloud sessions.** A reviewer runs `bosun verdict comment --from -`;
  the `verdict` workflow checks the comment and posts the `review-verdict` status.
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
