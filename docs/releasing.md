# Releasing bosun

bosun releases through [hoist](https://github.com/keelstackhq/hoist) (`hoist release`), the way
berth and hoist do (spec D13, D24). Agents never tag or publish. The tag and all uploads come
from `.github/workflows/release.yml`. One release publishes these items from one tag
`v<version>`:

- five static binaries and `SHA256SUMS` on a GitHub Release (`bosun-<target>[.exe]`),
- the npm launcher `@keelstackhq/bosun` and the platform packages
  `@keelstackhq/bosun-{linux-x64,linux-arm64,darwin-x64,darwin-arm64,win32-x64}`,
- the crate `keel-bosun` on crates.io (`cargo install keel-bosun`).

These files define the release: `hoist.toml`, `npm/bosun/**`, `npm/bosun-*/package.json`,
`pnpm-workspace.yaml`, `pnpm-lock.yaml`, `CHANGELOG.md` and `.github/workflows/release.yml`.
`tests/release_config.rs` keeps the versions and names in step.

## The gate

`release.yml` writes nothing unless the repository variable `BOSUN_RELEASE` is `enabled`.

| Event | Variable unset | Variable `enabled` |
| ----- | -------------- | ------------------ |
| push to `main` | `detect` only | `detect`, then `build`, `publish` and `prepare` when a release is pending |
| `workflow_dispatch` | `detect` and `build` (dry run: 5 targets, `bosun --version` smoke test, static check of the musl binaries; nothing is published) | on `main` as push, plus the dry-run checks; on another branch the same as unset (`publish` runs only on `main`) |

So a merge to `main` never tags, releases or publishes until the owner sets the variable.

## Normal flow (after the one-time setup)

1. Change the version in `Cargo.toml` and in all six `npm/*/package.json` files (including the
   launcher's `optionalDependencies`). Add a `## v<version>` section to `CHANGELOG.md`, then run
   `pnpm install --lockfile-only`. `hoist release prepare` does this in a release PR (branch
   `hoist/release`). `tests/release_config.rs` fails when the files disagree.
2. Merge to `main`. In `release.yml`, `detect` finds a version without a tag and `build` makes
   the five binaries. Then `publish` runs in environment `release` and waits for the owner's
   approval. It creates the tag `v<version>` through the API, the GitHub Release, the npm
   packages and the crate.
3. `prepare` opens, updates or closes the next release PR.
4. You can re-run a failed `publish` from the Actions page. hoist checks each registry before it
   uploads.

Dry run at any time: `gh workflow run release.yml --repo keelstackhq/bosun --ref <branch>`.

## One-time setup for v0.1.0 (owner, in this order)

Do steps 1 and 2 before the variable is set. Do step 6 within 2 days of step 4: the npm trusted
publishers that step 4 registers expire when they are not used for 2 days (D24).

1. **Environment `release`**, limited to `main`, with you as the required reviewer. If the
   environment does not exist, the first run creates it without protection.

   ```bash
   gh api -X PUT repos/keelstackhq/bosun/environments/release --input - <<EOF
   {"reviewers":[{"type":"User","id":$(gh api users/urbanisierung --jq .id)}],
    "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
   EOF
   gh api -X POST repos/keelstackhq/bosun/environments/release/deployment-branch-policies \
     -f name=main -f type=branch
   ```

2. **Optional secret `HOIST_RELEASE_TOKEN`**: a fine-grained token with Contents and Pull
   requests: write on keelstackhq/bosun. With it, the release PR runs CI. Without it, `prepare`
   uses `GITHUB_TOKEN`, and that PR runs no checks.
   `gh secret set HOIST_RELEASE_TOKEN --repo keelstackhq/bosun`
3. **Turn on the gate.** `gh variable set BOSUN_RELEASE --body enabled --repo keelstackhq/bosun`,
   then `gh workflow run release.yml --repo keelstackhq/bosun --ref main` (or wait for the next
   push to `main`). Approve the `release` deployment. `publish` tags `v0.1.0` and creates the
   GitHub Release. Then it stops at the first registry step, because the packages do not exist
   yet (a `first-publish:<pkg>` pending item). This failure is expected.
4. **`hoist release bootstrap`** in a local checkout of the tag. You must be logged in to npm
   (`npm login`) and crates.io (`cargo login`):

   ```bash
   npm install -g @keelstackhq/hoist@0.2.0
   git clone https://github.com/keelstackhq/bosun && cd bosun && git checkout v0.1.0
   hoist release bootstrap --dry-run
   hoist release bootstrap
   ```

   It publishes npm placeholders for the six packages and registers `npm trust github` for each
   (repo `keelstackhq/bosun`, workflow `release.yml`, environment `release`). It also publishes
   `keel-bosun` 0.1.0 to crates.io.
5. **crates.io trusted publisher for `keel-bosun`**: crates.io → keel-bosun → Settings → Trusted
   Publishing → Add: owner `keelstackhq`, repository `bosun`, workflow `release.yml`,
   environment `release`.
6. **Re-run the failed `publish` job** of the v0.1.0 run, within 2 days of step 4:

   ```bash
   gh run list --repo keelstackhq/bosun --workflow release.yml --limit 3
   gh run rerun <run-id> --failed --repo keelstackhq/bosun
   ```

   Approve the deployment again. The job finds the tag, uploads what is missing and publishes
   the npm packages through OIDC. Check the result with `npm view @keelstackhq/bosun version`
   and `cargo search keel-bosun`. From here on, every release uses the normal flow.

Until then, `npm i -g @keelstackhq/bosun`, `cargo install keel-bosun` and the GitHub Release
download do not work. Build from source with `cargo install --path .`.

## After v0.1.0: pin the release (R-gov PR)

An agent opens one PR, and the owner approves it on its head SHA (R-gov, R-ci). The PR:

- sets `.bosun/version` to `0.1.0`;
- changes job `bosun-bin` in `ci.yml` to install the pinned release
  (`npm install -g @keelstackhq/bosun@$(cat .bosun/version)`) instead of building the base
  commit (spec §11);
- changes `docs/setup-script.sh` to install `@keelstackhq/bosun@0.1.0` with the other tools,
  instead of `cargo install --git … --branch main`;
- updates the Status in `docs/PROGRESS.md`.
