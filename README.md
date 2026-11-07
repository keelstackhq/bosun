# bosun

Orchestration helper for repos where agents write all the code.

bosun is one static Rust binary in the keelstackhq family (splice, hull, hoard, hoist, berth). It
owns the process logic of an agent-built repo: claims and leases, spawn limits, waves, gates, risk
checks, review verdicts, lessons and cost. A Claude Code session calls `bosun <command>` and acts
on the one JSON object it prints. GitHub issues, labels, comments and PRs are the only shared
state.

Status: bosun 0.7.0 is released and pinned in `.bosun/version` (see `CHANGELOG.md`,
`docs/PROGRESS.md`). To set it up on a repo you already have, read
[docs/adopt-existing-repo.md](docs/adopt-existing-repo.md).

## Install

Releases: see `docs/releasing.md`.

```bash
npm i -g @keelstackhq/bosun     # launcher; the binary comes from a platform package
cargo install keel-bosun        # installs the binary `bosun`
# or download bosun-<target> from GitHub Releases (SHA256SUMS next to it)
```

From source (Rust 1.97): `cargo install --path .` (package `keel-bosun`, binary `bosun`).

## Cloud sessions: paste once

Paste this as the setup script of the Claude Code cloud environment (claude.ai/code >
Environments > Setup script), once. It never changes: each checkout runs its own committed
`docs/setup-script.sh` (bosun's, or the one `bosun adopt` writes), which pins the tools and their
checksums and moves them by reviewed PRs. A failure only warns. `bosun setup-snippet` prints the
same text and a line to send a client.

```bash
#!/usr/bin/env bash
# bosun bootstrap: paste it once as a Claude Code cloud environment's setup script; it never
# needs a change. It runs as root before each session, in the directory that holds the checkouts.
# Each checkout (or this directory) runs its own committed docs/setup-script.sh, which pins the
# tool versions and checksums and moves them only by reviewed PRs. It never fails the session.
w() { echo "bosun-bootstrap: $*" >&2; }
case "$(uname -m)" in x86_64 | amd64 | aarch64 | arm64) ;; *) w "untested arch $(uname -m); continuing" ;; esac
found=0
for d in . ./*/; do
  d="${d%/}"
  [ -e "$d/.git" ] && [ -f "$d/docs/setup-script.sh" ] || continue
  found=1
  w "running $d/docs/setup-script.sh"
  (cd "$d" && timeout 1800 bash docs/setup-script.sh </dev/null) ||
    w "$d/docs/setup-script.sh failed (exit $?); continuing"
done
# No repo script and no bosun: the latest bosun, kept only when npm verifies its registry
# signatures and any provenance (that needs the host tuf-repo-cdn.sigstore.dev).
p="${BOSUN_BOOTSTRAP_PREFIX:-/usr/local/lib/bosun-latest}"
if [ "$found" = 0 ] && ! command -v bosun >/dev/null 2>&1; then
  if command -v npm >/dev/null 2>&1 && rm -rf "$p" &&
    npm install --prefix "$p" --ignore-scripts --no-audit --no-fund @keelstackhq/bosun@latest >/dev/null 2>&1 &&
    (cd "$p" && npm audit signatures >/dev/null 2>&1) &&
    ln -sf "$p/node_modules/.bin/bosun" "${BOSUN_BOOTSTRAP_BIN:-/usr/local/bin}/bosun"; then
    w "no repo setup script; installed the latest bosun (npm registry signatures verified)"
  else
    rm -rf "$p"
    w "no repo setup script and no verified bosun (npm or its signature check failed; allow tuf-repo-cdn.sigstore.dev or run npm i -g @keelstackhq/bosun); continuing"
  fi
fi
exit 0
```

## Use

```text
bosun claim 42                         # lease the task's paths; exit 3 if a live lease overlaps
bosun wave plan 7                      # tasks that can run in parallel now
bosun pr 42                            # open the PR as a draft: no CI runs on it yet
for g in fmt lint test deps test-integrity; do bosun gate $g || break; done   # local, stamped
bosun pr 42 --ready                    # ready + auto-merge; GitHub merges on green (bosun never merges)
bosun gate-missed --pr 41              # "Which gate missed this?" issue for an escaped defect (main-red and revert open it too)
```

The loop is draft first: `bosun pr`, local gates, then `bosun pr --ready`. A draft costs no CI
minutes; see [docs/ci-minutes.md](docs/ci-minutes.md). `--auto-merge` is an old alias of `--ready`.

Exit codes: 0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit.

## Owner controls

- **Approve a high-risk PR.** The session asks you with a form in the Claude Code chat (repo, PR,
  head SHA; approve or hold); on approve the orchestrator posts the signed `bosun approve`
  comment for that exact head (`bosun approve <pr>@<head> --sign-key <pem>`), the `risk`
  workflow re-runs and `risk-check` turns green (D-012, spec §8, signed approvals). Fallback:
  comment on the PR yourself, as a login in `governance/owners.toml`, with exactly
  `/bosun approve <head sha>` (the full 40-character SHA of the current head). A new push needs
  a new approval. A GitHub
  review that approves the head SHA counts too, when the owner did not open the PR; no review
  event runs the workflow, so comment `/bosun recheck` (or re-run the risk workflow) after it:
  it only re-judges, as the next push does. A recheck runs only for a repo owner, member or
  collaborator, which limits CI minutes only; one from an owner with private org membership or
  an app (`NONE`) is dropped, so re-run the workflow then. To withdraw, request changes or
  dismiss the approval, then comment `/bosun recheck` (or re-run the risk workflow; it re-judges
  and fails); turning off auto-merge also stops a merge. A fallback
  comment must be your own (web UI or `gh` with your own token): an unsigned comment that an
  agent session writes through the Claude app does not count, even under your login. The signed
  comment carries a `bosun-sig ed25519` line from a key enrolled in `.bosun/approvers.toml` on
  the default branch, and counts whoever posts it. Enroll or rotate a key with a PR to that file
  that you approve on GitHub. Until `[approvals] bootstrap_until` (2026-11-30) the orchestrator
  may also sign gate approvals itself in bootstrap mode, never for the excluded paths (D-012).
- **Delegate decisions.** A key with `role = "delegate"` lets one Claude session approve PRs whose
  risk rules are all `approval = "delegate"`, with one clean review verdict on the head, while
  bosun is not halted. Critical changes (`.bosun/`, `governance/`, CODEOWNERS, every workflow,
  bosun's code and manifests, secrets, prod writes, an unseen diff) always need you (spec §8,
  delegated decisions); here that leaves `deny.toml`, the toolchain files, `.config/` and
  `profiles/` to a delegate. To turn it off at once, delete the delegate key from
  `.bosun/approvers.toml` in a PR you approve.
- **Review verdicts from cloud sessions.** A reviewer runs `bosun verdict comment --from -`;
  the `verdict` workflow checks the comment and posts the `review-verdict` status.
- **Trace and roll back one session.** Every agent commit carries `Bosun-Session`, `Bosun-Task`
  and `Bosun-Model` trailers, stamped by a commit-msg hook (`bosun trailers install`; the
  SessionStart hook runs it). `bosun pr` notes commits without them; `[trailers] require = true`
  in `.bosun/bosun.toml` makes that an error. To undo one session without the agent:
  `bosun trailers list <session>` prints its commits and a `git revert` recipe (spec §4).
- **Stop all work.** Create the label once:
  `gh label create bosun:halt --repo keelstackhq/bosun --color B60205`. Then open an issue with
  that label: `gh issue create --repo keelstackhq/bosun --label bosun:halt --title halt --body "<why>"`.
  Every command that starts or changes work exits 4 until you close the issue. The repo variable
  `BOSUN_HALT=1` also halts, where the token can read it.

- Adopt bosun on an existing repo (also cloud sessions and Claude Tag): [docs/adopt-existing-repo.md](docs/adopt-existing-repo.md)
- CI minutes: [docs/ci-minutes.md](docs/ci-minutes.md)
- Design: [docs/spec.md](docs/spec.md)
- Default values and their evidence: [docs/defaults.md](docs/defaults.md)
- Plan format: [docs/plan-format.md](docs/plan-format.md)

## License

MIT
