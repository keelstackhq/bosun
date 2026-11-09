# Update bosun

Short version: **`bosun pin --latest --pr`**, approve the PR, then `npm i -g @keelstackhq/bosun@<new>` on your
own machine. A cloud environment needs nothing: you paste its setup snippet once and never again.

What "a bosun version" means here:

| Place | What holds the version | Who changes it |
| --- | --- | --- |
| A repo | `.bosun/version` and the `BOSUN_*` lines of `docs/setup-script.sh` (version and sha512 of each package) | a pin PR (reviewed) |
| A cloud environment | nothing; each session runs the checkout's `docs/setup-script.sh` | nobody |
| Your machine | the globally installed `bosun` | you, with `npm i -g` |

A repo that uses the bosun channel (`.splice/channels/bosun.toml`) does not run `bosun pin`: the splice update
bot opens that PR, or run `splice update --channel bosun`.

## 1. Know that an update exists

- Human: `bosun doctor` always shows step 17 `update`: `ok` (everything matches npm latest), `warn` (something is behind; exit stays 0; its `fix` is the command to run) or `skipped` (not checked: the `detail` says why, for example npm is unreachable). A bosun channel repo still compares the binary.
- Agent: the SessionStart text says `tool pins behind: …` and names the command. Do nothing if it is absent.
- Either: `bosun pin --latest --check` lists what is behind and writes nothing.

`BOSUN_PIN_CHECK=0` turns the check off. A broken network only logs a warning.

## 2. Move the repo to the new version (human or agent)

On the default branch, up to date, with a clean tree:

```bash
export GITHUB_TOKEN=$(gh auth token)   # not needed in a cloud session
bosun pin --latest --pr
```

It downloads each tarball, checks it against npm's `dist.integrity`, rewrites the pins, creates the branch
`bosun/pin`, commits, pushes (never forced) and opens a draft PR. It prints `data.pr` and `data.left`, the steps
that remain. If a pin PR is already open it only names it. If `bosun/pin` is left from an old pin, it stops
(exit 3) and says what to delete.

Then:

1. Mark the PR ready and get the owner approval. It edits `docs/setup-script.sh` (and `.bosun/version` when bosun moves), which are gated paths (R-gov, spec §8). `data.left` lists only the steps a human must do.
2. Turn on auto-merge. GitHub merges when the checks are green. **Agents never merge.**
3. Post a review verdict if your repo asks for one (`bosun review-pack`, `bosun verdict comment`).

## 3. Update the CLI on your machine

After the PR merges:

```bash
npm i -g @keelstackhq/bosun@<new>    # or: cargo install keel-bosun --locked
bosun --version
```

`bosun doctor` clears its `update` warning when the binary and the pins match npm latest.

## 4. Update a cloud environment

Nothing to do. Before each session the pasted bootstrap runs the checkout's `docs/setup-script.sh`. After the pin
PR merges, the next session on the default branch installs the new pinned version (each version has its own
install record, so a warm container installs it again). A test keeps this true: `tests/cli_setup_snippet.rs`.

Paste the snippet **once**, when you create the environment: `bosun setup-snippet`, then claude.ai/code >
Environments > Setup script. Never paste it again; it does not change between releases.

## If verification fails

| You see | What it means | What to do |
| --- | --- | --- |
| `tarball sha512 does not match dist.integrity` or `outside the registry` | npm served other bytes than it signed | Do not pin. Tell the owner. Rerun later. |
| `npm latest X is older than the pin` (exit 3) | the `latest` tag moved back | Pins only move up. Check the npm dist-tag, or pin by hand. |
| `npm …: HTTP …` or `timeout` | network or registry | Check the network and `BOSUN_NPM_REGISTRY` (https only), rerun. |
| `the working tree has changes`, `not the default branch` (exit 3) | the pin PR needs a clean base | Commit or stash, `git switch <default>`, rerun. |
| `bosun/pin and bosun/pin-<date> already exist` (exit 3) | old pin branches are left | If only `bosun/pin` is left, the command uses `bosun/pin-<date>` by itself. When both are left, delete them locally (`git branch -D`) and on origin (the owner). |
| exit 4 | the repo is halted | Close the `bosun:halt` issue (owner). |
| setup script: `does not match its pinned sha512; skipping it` | a pin in a merged script is wrong | The session goes on without that tool. Run `bosun pin --latest --pr` again. |
| bootstrap: `no verified bosun` | no repo script, and the npm signature or provenance check failed | Allow `tuf-repo-cdn.sigstore.dev`, or `npm i -g @keelstackhq/bosun`. |

Never install a package that failed a check, and never edit a sha512 by hand to make a check pass.
