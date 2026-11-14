## 1. Summary and decisions

bosun lets one or more autonomous Claude Code sessions build a repo with no human edits to code.
Humans set up the repo, own the risk decisions, and approve high-risk PRs. Everything else goes
through bosun commands, GitHub issues and PRs, and CI.

| #   | Decision | Choice |
| --- | -------- | ------ |
| D1  | Product shape | bosun is its own product in its own repo: one static Rust binary, stack-agnostic. It builds itself first. |
| D2  | Coordination substrate | GitHub issues, labels, comments and PRs. No other database. |
| D3  | Process logic | Only in the bosun CLI. JSON output, fixed exit codes. Hooks block the bypasses. |
| D4  | Human authority | Tiered. A human acts only on high-risk changes, questions and budget raises. |
| D5  | Execution | Hybrid: one orchestrator session per area (epic), and wave agents inside it. |
| D6  | Limits | Hard caps on depth, live sessions, children, parallel agents, turns and money. |
| D7  | Verification | CI gates through a generic gate contract, machine-checkable UCs, mutants, and an agent review verdict. |
| D8  | Review | Mode (a) a fresh subagent first. Mode (b) a CI check later (M8). |
| D9  | Learning | Lessons are captured, triaged and pruned. The preferred result is a mechanism, not advice. |
| D10 | Seed | Seed A: a human checklist plus agent seed PRs (see D22). |
| D11 | Slack | Through Claude Tag only. Threads may start sessions with the same checks as any spawn. |
| D12 | Generated files | No reference to the source playbook in generated files. Each comment states its own reason. The evidence for the defaults is in `docs/defaults.md`. |
| D13 | Merging and tags (A1) | Agents never merge, tag or force-push. Merges go through the GitHub merge queue or auto-merge, which `bosun pr --auto-merge` turns on as soon as the PR exists and the task is in review; GitHub merges on green. A PR that is already clean cannot get auto-merge, so the orchestrator merges it (D-008). Releases run through `hoist release` in a workflow; tags come from workflows only. |
| D14 | Waiting (A2) | `bosun wait` polls GitHub state with backoff inside one call. It never depends on send_later or send_message. Child-to-parent results go through issue comments. |
| D15 | Required checks (A3) | `ci-ok` passes only when every gate passed (the org convention; the channel's bosun.yml runs them in one job of that name, or, with a ts-pnpm stack, in that job after `bosun-trusted`, §3). `risk-check`, `review-verdict` and `main-open` stay separate required checks. In a repo whose own ci.yml already has `ci-ok` (a hull template), bosun's gates aggregate as `bosun-ok`, a fifth required check. |
| D16 | Init and upgrade (A4) | `bosun init` runs `splice channel add gh:keelstackhq/bosun//channel` (pieces `core`, `rust`, `ts-pnpm`). There is no `bosun upgrade`: the splice update bot (`keelstackhq/splice/action@v1`) keeps the channel fresh. |
| D17 | Repo settings (A5) | Checklist steps 3 and 5 (branch protection, environments, secrets, variables) are done with `hoist apply`. `bosun doctor --remote` calls `hoist check`. |
| D18 | Workspaces (A6) | Wave agents get isolated workspaces from `berth up` (worktree, env files, port block, database per agent). `berth gc` cleans up after merge. |
| D19 | CI cache (A7) | Generated gate workflows cache through `keelstackhq/hoard@v0`. hoard is a Turborepo remote cache, so it applies to ts-pnpm stacks; Rust gate jobs (bosun's own CI included) cache with `Swatinem/rust-cache`, as berth and hoist do. |
| D20 | Usage (A8) | The usage provider `claude-sessions` reads `usage.cost_usd` from the claude-code-remote `get_session` result. It ships in the seed with the `generic` estimate. |
| D21 | Exit codes (A9) | Exit codes 0-6 as in §1.1; 2 is usage, as in splice and berth. Every error JSON has a `fix` field that says how to fix it. |
| D22 | Seed shape (A10) | The seed is a sequence of PRs of at most 400 changed lines each, not one seed PR. |
| D23 | Environment (A11) | The environment's setup script is the bootstrap that `bosun setup-snippet` prints, pasted once: it never pins anything, so a release needs no new paste. It runs as root in the directory that holds the checkouts and, for that directory and each direct subdirectory with a `.git`, a `.bosun/version` and a `docs/setup-script.sh`, runs that script from inside the checkout (stdin closed, `timeout -k 10 1800`); a failure warns and the next checkout runs. The trust moves to the checked-out branch: the script that runs as root is the one on whatever branch the session checked out, not only main's reviewed copy. With no repo script and no `bosun` on PATH it installs `@keelstackhq/bosun@latest` into a prefix and keeps it only when `npm audit signatures` passes (registry signatures and attestations; it needs `tuf-repo-cdn.sigstore.dev`) and every installed `@keelstackhq` package has a SLSA provenance statement for its sha512 whose DSSE signature checks against a certificate with the identity (SAN) `https://github.com/keelstackhq/bosun/.github/workflows/release.yml@refs/heads/main` (taken from the certificate, not from the payload the signer writes), else it warns; the certificate chain to Fulcio and the transparency log are checked only by the `npm audit signatures` run before it, which fetches the attestations separately. Attestations come only from `registry.npmjs.org` (`curl -m 30`); another registry, a timeout or any mismatch fails closed. It always exits 0. The repo's `docs/setup-script.sh` (run from inside each checkout, possibly on a warm container) installs Node 24 and makes it the `node` that resolves first (a profile.d PATH entry plus a replacement of an older `node` earlier in PATH, such as `/opt/node22/bin/node`; it warns naming both paths if `node` is still not Node 24), pnpm 12 (native binary) only when the checkout uses pnpm (a `pnpm-lock.yaml`, a `pnpm-workspace.yaml` or a `package.json` whose `packageManager` is `pnpm@...`), rust only when it has a `Cargo.toml` or `rust-toolchain.toml` (the repo's own script checks the checkout root only, the channel's copy also `rust_path`; the `channel` of the repo's `rust-toolchain.toml`; without it the script warns, installs no pinned toolchain, and still installs cargo-deny with rustup's stable default or the image's cargo, rustup installs the pinned toolchain on first cargo use there; a failed step warns, never fails; a checkout with neither file gets no rustup or cargo call), cargo-deny (cargo-nextest is still built for a pin before 0.12.0 or when `.bosun/profiles/rust.toml` names nextest, and not otherwise: building it takes minutes; from the pinned `@keelstackhq/cargo-deny` launcher and `cargo-deny-linux-<arch>` packages like the tools below once `CARGO_DENY_VERSION` and its sha512 pins are set, else, or when that verified install fails, `cargo install --locked cargo-deny`), and the pinned bosun, splice, hoist and berth, each only from `npm pack` tarballs of its launcher and linux binary package whose sha512 equals the pinned npm integrity, installed with `--ignore-scripts` (a mismatch or a failed install warns and skips that tool; a tool is not downloaded again only when the sha256 fingerprint of its install, taken over the resolved command's real path and every file (by content) and symlink (by target) of its launcher and linux packages, sorted by bytes, equals the one recorded at its verified install in `/var/lib/bosun-setup/<tool>@<pin>.sha256`; a printed version, a changed file or another program of that name earlier in PATH installs it again; a pnpm already at its pin is not downloaded again; the fingerprint stops drift and stray binaries between runs, not an attacker with root in the container: a fresh container is the boundary). Exception: the channel's script installs bosun by version (its pin is the channel's release or `bosun_version`, whose integrity is unknown when the channel is written; npm checks the registry's) and records and checks its fingerprint the same way. pnpm, the Node 24 setup, the `cargo install` fallback of cargo-deny and the `cargo install --locked cargo-nextest` build are not integrity-pinned. The splice update bot (`keelstackhq/splice/action`, pinned by commit, `version:` pinned) installs splice by version: the action checks no integrity, so it trusts the npm registry. |
| D24 | First publish (A12) | The first publish follows the hoist bootstrap flow (`hoist release bootstrap`). npm trusted publishers expire if unused for 2 days, so the first workflow release follows within 2 days. `release.yml` publishes nothing until the owner sets the repo variable `BOSUN_RELEASE=enabled`; the steps are in `docs/releasing.md`. |
| D25 | Repo and scope (A13) | The repo is `keelstackhq/bosun` (Q1 answered). SP2, the runtime, is out of scope. |
| D26 | Org rule (A14) | "Agents never merge or tag; merges happen via auto-merge after the gates" is org decision D-006 in keelstackhq/decisions (settled 2026-10-22). It reaches this repo through the decisions channel (D-004): `docs/agents/org-decisions.md`, imported by CLAUDE.md. |
| D27 | Crate shape | One package `keel-bosun` with modules `cli`, `core`, `github`, `sessions`, `gates`, `plan`, `hooks`, not a workspace of 7 crates. This matches berth and hoist. `core` does no I/O; a test enforces it. |
| D28 | HTTP | `ureq` 3 with rustls (as hoist), not octocrab. bosun is synchronous; no tokio. |
| D29 | Session tools | bosun never calls the claude-code-remote tools itself; a binary cannot. A command that needs a tool call returns it in `next` (tool name and arguments). The session makes the call and passes the result back with `--from -` on stdin. |
| D30 | Clock and tests | Time comes from `BOSUN_NOW` (RFC 3339) when set; the GitHub base URL from `BOSUN_GITHUB_API` (unset, empty or blank: `https://api.github.com`). Tests use a fake GitHub server and never reach a real API. |
| D31 | Identity | The repo comes from `BOSUN_REPO` (`owner/name`), else the `origin` remote. The session id comes from `BOSUN_SESSION`, else `CLAUDE_CODE_REMOTE_SESSION_ID` with its `cse_` prefix changed to `session_`, else `local-<host>`. The GitHub token comes from `GITHUB_TOKEN` or `GH_TOKEN`, never argv. |
| D32 | Status posts | `risk-check` and `review-verdict` are commit statuses, because only GitHub Apps can create check runs. Any workflow's `GITHUB_TOKEN` can post them, so D33 replaces this once the ruleset is switched (§15). |
| D33 | Gate source (B2, designed) | `risk-check`, `review-verdict` and `main-open` become check runs of a dedicated GitHub App whose key lives in a `main`-only environment, and the ruleset pins each to that App's `integration_id` (§15). |
| D34 | Owner teams (B4, designed) | `governance/owners.toml` may list `team = "org/slug"`, expanded per run on the App token; an unreadable team counts no members (§16). |
| D35 | Tool pins | Pins move by `bosun pin --latest` in the bosun repo (approach a): it reads npm latest for splice, hoist, berth and bosun, verifies each launcher and linux-x64/arm64 tarball against `dist.integrity`, and only moves up (downgrade exit 3, mismatch exit 1, nothing written). It edits `docs/setup-script.sh`, `.bosun/version` and the channel template's pins; `--check` only lists what is behind; `bosun doctor` shows it as step 17 `update` (warn). When files changed, one pin PR per repo: branch `bosun/pin` (`bosun/pin-YYYYMMDD` when a stale `bosun/pin` is left; both taken is exit 3, nothing written); if a PR from it is open `data.pr` names it and the session refreshes it with a new commit (merge the default branch first, never force-push), else `next` is one draft `mcp__github__create_pull_request` call (a GitHub error sets `data.pr_lookup` and the branch is still picked as above). Consumers get pins through the bosun channel (the splice update bot), so the command refuses there. Rejected: (b) a scheduled workflow (`GITHUB_TOKEN` PRs run no CI, needs a bot token, Actions minutes D-007), (c) a `release.yml` dispatch (cross-repo token). |

### 1.1 Exit codes

Fixed for all commands:

| Code | Name | Meaning |
| ---- | ---- | ------- |
| 0 | ok | The command did its work. |
| 1 | error | An unexpected failure (API, I/O, bad config file). The JSON has the error and its fix. |
| 2 | usage | Bad arguments. |
| 3 | conflict | A claim or a path overlaps a live lease, or another orchestrator owns the area. |
| 4 | halted | The kill switch (an owner's open `bosun:halt` issue, or `BOSUN_HALT=1`), main-red or process-red stops the command. |
| 5 | needs-human | The command needs an owner decision. |
| 6 | limit | A cap or a budget refuses the command. |

### 1.2 JSON output

stdout is always exactly one JSON object, also on errors and usage errors; bosun has no table
output, because its reader is a session (D3). `--json` is accepted and changes nothing, so calls
read the same as for splice, hoist and berth. Logs and child-process output go to stderr. Hook
commands are the one exception: their stdout follows the Claude Code hook protocol (§4, S8).

```json
{"version":1,"command":"claim","ok":false,"exit_code":3,
 "data":{},
 "error":{"kind":"conflict","message":"crates/x.rs is leased by session_02 until 10:30Z",
          "fix":"pick another task with `bosun wave plan`, or wait for the lease to end"},
 "next":[]}
```

- `data` is the command's result. Its shape is per command: `bosun schema [<command>]` prints
  it as JSON Schema (`data.commands`, or `data.schema` for one command: the envelope with its
  `data`), and `--format ts` as TypeScript declarations in the string `data.ts`
  (`declare namespace bosun`, one `function verdictPost(...args: string[]):
  Promise<Envelope<"verdict post", VerdictPostData>>` per command, for the harness to implement)
  for Code Mode harnesses; both come from the notation in `src/schema/data.rs`. The schema is
  open (new keys are not a breaking change); the test helper checks every envelope the tests see
  against it strictly, so it cannot drift from the code.
- `error` is `null` when `ok` is true. `kind` is the exit-code name of §1.1, except `upstream` (exit 1) for a GitHub 5xx that outlived the 3 attempts: retry the same command. `fix` is never empty.
- `next` lists the tool calls the session must make (D29): `{"tool":"create_session","args":{..}}`.
- The npm launcher (`@keelstackhq/bosun`, `npm/bosun`) keeps this contract when it cannot start the binary: it prints one envelope with `exit_code` 1 and a `fix`, and the message on stderr. That holds for `hook` commands too; Claude Code treats a hook's exit 1 as a non-blocking error.
- Time budget: all GitHub calls of one command may take 120 s (`BOSUN_HTTP_BUDGET_SECS`, 0 = no limit; time in requests and retry sleeps counts, local work does not; risk-check lifts it for its `error` cleanup post; `wait` with a timeout is exempt). Past it the command exits 1 with `GitHub calls ran past the 120s budget` and a `fix` (narrow it, for example `bosun metrics --days 1`), so `heal`, `wave tick` and `metrics` never hang behind a slow proxy.
- Summary output (agents read it): `doctor` lists only steps that are not `ok` (`items_ok` counts the rest; `routines` only when a step points at them), `plan import`/`plan amend` list only epics and tasks that changed (`unchanged: {epics, tasks}` counts the rest), `progress` leaves out `tasks`; each adds `summary: true`. `--full` prints everything (the shape of the schema, `summary` absent). Errors keep their full `data`.
