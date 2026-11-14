# Move a repo to another bosun mode

Audience: Claude (an agent) in a repo that already runs bosun, and the owner. Goal: one command, one owner
approval. Modes run gatekeeper < steady < booster < autonomous (quickstart §0).

## Quick start

Cloud environment setup script (`bosun setup-snippet` prints it as `data.short`; paste it once):

```bash
for d in . ./*/; do
  [ -f "$d/.bosun/version" ] && [ -f "$d/docs/setup-script.sh" ] || continue
  (cd "$d" && timeout -k 10 1800 bash docs/setup-script.sh </dev/null) || echo "bosun: $d/docs/setup-script.sh failed" >&2
done; exit 0
```

Each repo's own `docs/setup-script.sh` installs the pinned bosun (sha512 checked) and the rest. A repo without that
script needs the long form, `data.script`.

The prompt, the same in the CLI, on the web and to Claude Tag in Slack:

```text
Run `bosun migrate --to booster --direct`; do what its `next` and `data.then` say; stop when it asks for the owner.
After the merge run `bosun migrate --finish`.
```

This prompt goes straight to booster: `--direct` is the owner's choice, and the PR body records it. Drop
`--direct` to move one step (gatekeeper → steady). The owner approves the one PR head.

## What `bosun migrate` does

```bash
bosun migrate --to booster --dry-run   # survey and planned edits; writes nothing
bosun migrate --to booster             # one governance PR (draft)
bosun migrate --finish                 # after the merge: join, plan amend (idempotent)
```

- **Survey:** pinned version, current mode, registry issue, epics without a `bosun:epic` marker, hull lock,
  and a 30-day metrics baseline (best effort, 20 s timeout: a missing baseline never blocks).
- **One PR** with every file step: the `.bosun/version` pin (when the release is on npm; else exit 3 that names
  the pin step), the `.bosun/registry` pin, and `[mode] name`. The PR body lists the one owner approval and the
  owner-only steps left (Routines). An open PR for the same step is reused.
- **One step at a time:** without `--direct` it moves one rung (gatekeeper → steady). With `--direct` it goes
  straight to the target; the PR body records that the owner chose it.
- **Auto-merge without a task:** `bosun pr --pr <n> --ready` marks the PR ready and turns on auto-merge after the
  local gates. risk-check still waits for the owner: R-gov always, R-weaken for a lighter mode.
- **After the merge:** `bosun migrate --finish` runs `bosun join`, then `bosun plan amend` for each plan an open
  epic names (add `--plan <file>` for an epic without a marker). Run it twice: the second run changes nothing.

The owner does one approval per PR: the exact head (repo, PR, head SHA) in the way bosun asks. Routines stay
human-only (claude.ai/code → Routines); the PR body lists them.

## CLI (Claude Code on your machine)

1. `npm i -g @keelstackhq/bosun` (a release where `bosun migrate --help` works), `gh auth login` once.
2. Local only: set `GITHUB_TOKEN` to `gh auth token` in your shell (scopes repo, workflow). Never in a file. A cloud session needs no token.
3. Open Claude Code in the repo, on the default branch, clean tree. Paste the prompt from Quick start.
4. **[phone]** Approve the PR head when Claude asks.
5. Do the Routine steps the PR body lists.

## Web / cloud (claude.ai/code)

1. Environment (session title bar → Edit), once per repo: paste the Quick start script as the
   Setup script; Network access: Limited with "Allow package managers"; add the hosts
   your `docs/setup-script.sh` downloads from (see "Cloud hosts" below; a Rust repo needs `github.com` and
   `release-assets.githubusercontent.com` for the prebuilt cargo tools). No secrets, no PAT: GitHub comes from the session's proxy and the Claude GitHub App.
2. Start a session on the repo in that environment. Paste the prompt from Quick start.
3. **[phone]** Approve the PR head when Claude asks.
4. Do the Routine steps the PR body lists.

## Claude Tag in Slack (the only entry)

1. Once: a private channel, your Slack ID in `governance/owners.toml`, `[slack] channel`,
   `may_spawn = true` and `[cloud] environment_id` in `.bosun/bosun.toml` (an R-gov PR; the `[slack]` table is in [quickstart.md](quickstart.md) §3). The environment is
   the one from the web steps.
2. In the channel, start a thread and post the Quick start prompt to Claude.
3. **[phone]** Approve the PR head when bosun asks.
4. Do the Routine steps the PR body lists.

## Optional: the CI signer

Off by default; the orchestrator-held key stays the default. To let cloud and Claude Tag sessions run
`bosun approve <pr>@<head> --via-signer`, the owner sets `[approvals] signer = true` (R-weaken) and
`[checks] mode = "checks"` with the gate App, then per repo: `bosun keygen --role signer --id signer-ci
--out <pem outside the repo> --enroll`, merge that `.bosun/approvers.toml` PR (R-gov), create the GitHub
environment `bosun-signer` with the secret `BOSUN_SIGNER_KEY` (the PEM), delete the PEM. The real barrier:
Settings, Environments, `bosun-signer`, Deployment branches, Selected branches, only the default branch.
Check it with `bosun doctor --remote`. The key signs only the autonomous form (D-017).

## Cloud hosts

Network access: Limited with "Allow package managers" (npm, PyPI, crates). Add only hosts the repo's
`docs/setup-script.sh` downloads from: `github.com` (plugin marketplace, prebuilt cargo tools),
`release-assets.githubusercontent.com` (those tools; without it a Limited network falls back to a slow
`cargo install`), `deb.nodesource.com` (Node 24, only when the container has none),
`tuf-repo-cdn.sigstore.dev` (only for a checkout without a repo script). A denied host shows as a proxy
refusal that names it. Read the script before you add one.

A cloud session cannot write commit statuses (use `bosun verdict comment --from -`), approve with
`BOSUN_OWNER_TOKEN` (approve by chat form), or merge, tag or force-push. If `bosun pr --ready` returns
`data.ready: "pending"`, make the `mcp__github__update_pull_request` call in `next` and run it again.

## Rollback

A tighter mode is one R-gov PR (no R-weaken): `bosun migrate --to steady` (or `gatekeeper`), or set
`[mode] name` by hand; deleting the `[mode]` table restores the old behaviour byte for byte. Keep
`.bosun/registry`, epic markers and bind records. Do not unpin bosun: pins only move up, a downgrade exits 3.
Undo a session's commits: `bosun trailers list <session>`; revert a merged PR with `bosun revert`
(a `kind/mainfix` PR). Never force-push.

## Troubleshooting

Read `error.fix` first. Exit codes: 0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit.

| You see | What to do |
| ------- | ---------- |
| Session start: installed X, repo pins Y; or `unknown field` for `[mode]` or `[approvals]` | The installed bosun is older than the repo's pin. Run `bash docs/setup-script.sh`, then `bosun --version`. |
| Exit 4 | The repo is halted (the owner closes the `bosun:halt` issue), or on `wave` main is red: wait for the fix. |
| Exit 3 on `claim` | Another session holds an overlapping lease: `bosun wave plan <epic>`. |
| Exit 3 on `pin` | Dirty tree, wrong branch, two pin branches, or npm latest is older than the pin: [update.md](update.md). |
| Exit 5 | A task needs the owner or a budget ask is open: `bosun ask <issue> "<question>"`; see [quickstart.md](quickstart.md). |
| `bosun pr --ready`: `missing_gates` | Run the loop in `error.fix`. |
| `git push` refused | A gate has no stamp: run the gate loop, push alone. |
| `wave tick`: `held_by spawn_auto is off` | The mode is below booster: `bosun mode`. |
| `wave tick`: `brake on` | Main red twice, two reverts, or red-CI 10 points over target in 7 days: wait. |
| `bosun join` reaches no GitHub | Local: set `GITHUB_TOKEN` and `BOSUN_REPO=owner/name`. Cloud: check the GitHub App. |

If a step is not covered here, stop, run `bosun doctor`, read its `fix`, and ask the owner.
