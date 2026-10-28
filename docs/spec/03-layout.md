## 3. Repo layout

bosun repo:

```
Cargo.toml                package keel-bosun, lib bosun, bin bosun
src/cli/                  clap commands, JSON output, exit codes
src/core/                 lifecycle, leases, limits, risk, lessons; no I/O
src/github/               issues, labels, comments, PRs, checks (ureq)
src/sessions/             claude-code-remote requests and results, usage providers
src/gates/                gate contract, profiles, adapters
src/plan/                 plan.md parser, import, amend
src/hooks/                PreToolUse and SessionStart hook entry points
profiles/                 rust.toml, ts-pnpm.toml
channel/                  the splice channel that `bosun init` adds (pieces core, rust, ts-pnpm)
docs/spec.md (index), docs/spec/ (one file per section), docs/defaults.md, docs/plan-format.md
.bosun/                   bosun runs on itself
```

A consumer repo after `bosun init`:

```
.bosun/bosun.toml         project, stacks, review mode, slack
.bosun/limits.toml        caps and budget (§5)
.bosun/risk.toml          risk rules (§8)
.bosun/version            pinned bosun release (R-gov)
governance/owners.toml    GitHub and Slack IDs of the owners
docs/lessons.toml         the lessons store
plans/*.md                plans in the plan format
.github/workflows/        call only `bosun gate <name>`; cache through hoard
.github/CODEOWNERS
.claude/settings.json     hooks: bosun hook pre-tool-use, bosun hook session-start
AGENTS.md                 no lessons; points to bosun commands
REVIEW.md                 review rules for the verdict
.splice/channels/bosun.toml   the channel record, kept fresh by the splice update bot
```

A channel version is a bosun release tag (`v<x.y.z>`), and the channel writes that version into
`.bosun/version`, so the update bot moves the channel files and the pinned binary in one R-gov PR.
A branch or sha ref has no release version; such a consumer sets the template var `bosun_version`.

The channel's workflows run the release pinned in `.bosun/version` from npm, never bosun built
from the change: bosun.yml reads the pin of the PR's base commit (the setup PR, whose base has no
pin yet, uses its own and says so in a warning); risk.yml and verdict.yml read it from the base
or the default branch and never check out PR code. main-health's weekly run calls bosun.yml
(`workflow_call`), so it runs `bosun gate <name> --all` for any stack. Consumers inherit the
residual of PROGRESS Blocked item 5: a same-repo PR can add a workflow that posts a forged
status; such a PR is R-ci and needs the owner.

The channel never needs a file another template owns (a hull template owns ci.yml, CLAUDE.md
and .claude/settings.json): its gates run in `.github/workflows/bosun.yml`, whose aggregate job is
`ci-ok`, or `bosun-ok` (one more required check) when the repo has its own ci.yml (`own_ci`);
with `own_ci`, main-health watches both workflows and closes `bosun:main-red` only when the
latest push run on main of each is green; with `own_ci` a pull request runs only the `test-integrity`
gate and relies on the repo's own `ci-ok` for fmt, lint, test and deps (if that CI lacks one, such as cargo-deny, set the repo
variable `BOSUN_ALL_GATES=1`); push, merge queue and the weekly run still run all five; a pull request whose files are all `*.md` under `docs/` or `plans/` or at the root except README.md (doctests; never with the ts-pnpm stack: prettier checks Markdown)
runs no gate (step `diff`, fail closed); to save CI minutes (each job is billed as at least one minute on a private repo) a rust stack's bosun.yml is one job
named after the required check (timeout 30 min), which runs the docs-only check, the pinned bosun, the pin check, then every gate (the first failure stops it), the gates that
run no PR code first (test-integrity, deps, fmt) and lint and test (build.rs, tests) last, so PR code on the one runner cannot change the others' result;
a ts-pnpm or mixed stack's bosun.yml has two jobs (#234), because installing the PR's dependencies runs PR code (install scripts, and a pnpmfile, patches, dependency names or workspace settings that pnpm reads from many places): `bosun-trusted` (timeout 15 min) plans, runs the pinned bosun and the pin check and then only test-integrity and deps; it never installs the PR's dependencies and restores no cache, and pnpm runs there only for the deps gate's `pnpm audit`, which reads the PR's `pnpm-lock.yaml` as data while every `package.json` and `pnpm-workspace.yaml` is put back to the base's copy right before deps (test-integrity has read the PR's; one the base lacks is deleted, a stack root `package.json` the base lacks becomes `{}`, and no file is written through a directory link that leaves the checkout; also when the PR has no lockfile; so a PR's change to them, an `auditConfig` ignore entry too, applies after it merges, and the run prints a notice naming them), `.npmrc` and `.pnpmfile.*` are the base's (as for every job), `npm_config_ignore_pnpmfile` and `pnpm_config_ignore_pnpmfile` are `true` and `manage_package_manager_versions` is `false` (both env spellings), and pnpm is the exact release `pnpm@x.y.z` (9 or later) that the base's stack `package.json` (`<ts_path>/package.json`) `packageManager` names (the PR's while the base names none; never a path, URL or range); the required check's job (`ci-ok` or `bosun-ok`, timeout 30 min) needs it and runs `if: always()`, so it fails at once unless bosun-trusted succeeded, also after a cancel or timeout (never skipped: a skipped required check counts as passed), then installs the same pinned bosun (checked against bosun-trusted's sha256), puts the base's gate config over the checkout, installs the PR's dependencies normally (`pnpm install --frozen-lockfile`, scripts and the base's pnpmfile run), and runs the gates bosun-trusted left (fmt, lint, test) with the rust toolchain bosun-trusted checked; it has `contents: read` only, neither job gets a secret, and nothing the second job does (its caches included: rust-cache, the pnpm store, hoard) reaches bosun-trusted, which has finished; an edit or own_ci run that leaves no gate passes the second job in seconds (about one extra billed minute per run);
a draft PR fails it in seconds, never skips it (a skipped required check counts as passed; `ready_for_review` runs the gates); a body or title edit that keeps the base never cancels: it waits in the PR's concurrency group,
then, when the newest finished run of that check on the head from GitHub Actions (`checks: read`) succeeded, keeps its file gates and runs only
test-integrity (it reads the body; the earlier run may have read another one) without the stack setup, else every gate (residuals, both R-ci or fail closed: a check of the same name from another workflow of the head counts, as in `bosun approve`;
a late edit event can replace a pending push run, which leaves the new head without the check until its next event; with two jobs the edit step in bosun-trusted looks up the required check's name); the rust gates install the toolchain of the repo's `rust-toolchain.toml` once (the step looks in `rust_path` and its parents up to the repo root; no
dtolnay step; the old 1.97 pin when there is no toolchain file, never the runner's stable), and the setup script reads its `channel` too (no fallback: without the file it stops; `--rust-version` prints the version and installs nothing); the CLAUDE.md section is the optional piece `claude-md`; seeds that exist stay the repo's own.
`bosun init` sets `own_ci`, adds `claude-md` unless the main template owns CLAUDE.md, merges the
required checks into an existing hoist.toml, and excludes a .claude/settings.json it does not own;
the owner then adds the two bosun hooks there (an owner step), since only one source may own it.
