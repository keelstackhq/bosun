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
channel/                  the splice channel that `bosun init` adds (pieces core, rust, ts-pnpm, circleci, circleci-only)
docs/spec.md (index), docs/spec/ (one file per section), docs/defaults.md, docs/plan-format.md
.bosun/                   bosun runs on itself
```

A consumer repo after `bosun init`:

```
.bosun/bosun.toml         project, stacks, review mode, slack, intake (optional bool)
.bosun/limits.toml        caps and budget (§5)
.bosun/intake.toml        the owner's upfront answers (seed; below)
.bosun/risk.toml          risk rules (§8)
.bosun/version            pinned bosun release (R-gov)
governance/owners.toml    GitHub and Slack IDs of the owners
docs/lessons.toml         the lessons store
plans/*.md                plans in the plan format
.github/workflows/        call only `bosun gate <name>`; cache through hoard
.github/CODEOWNERS
.claude/settings.json     hooks: bosun hook pre-tool-use, post-tool-use, session-start
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
(`workflow_call`), so it runs `bosun gate <name> --all` for any stack. No channel workflow runs
on a review event (those run the PR's copy of the file). Consumers inherit the open residual of
spec §8 and PROGRESS Blocked item 5: a same-repo PR can add or edit an `on: pull_request`
workflow (bosun.yml, the repo's ci.yml or a new file) that posts a forged `risk-check` status,
and the R-ci class does not stop it, since the owner gate is that status. B2 (a check run from a
dedicated GitHub App, pinned in the ruleset) closes it.

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
dtolnay step; the old 1.97 pin when there is no toolchain file, never the runner's stable), and the setup script reads its `channel` too (no fallback: without the file it warns, skips the pinned Rust toolchain but still installs cargo-nextest and cargo-deny with rustup's stable default or the image's cargo, installs the rest and exits 0, also when a tool fails; `--rust-version` prints the version and installs nothing, and exits 1 without the file; a `channel` that is not a toolchain name, `^[A-Za-z0-9][A-Za-z0-9._-]*$`, never reaches rustup: it is skipped like a missing file, with an `invalid channel` warning); the CLAUDE.md section is the optional piece `claude-md`; seeds that exist stay the repo's own.
`bosun init` sets `own_ci`, adds `claude-md` unless the main template owns CLAUDE.md, merges the
required checks and `merge_queue = false` (unless set) into an existing hoist.toml, and excludes a .claude/settings.json it does not own;
the owner then adds the two bosun hooks there (an owner step), since only one source may own it.

CircleCI (hull secrets-ci design §6.5, §7.4): piece `circleci` adds `.circleci/src/jobs/bosun-ok.yml`
and `.circleci/src/workflows/bosun.yml`, one unfiltered workflow (no branch, path or `when` filter)
whose job reports `ci/circleci: bosun-ok`, or `ci/circleci: ci-ok` without `own_ci` (workflow `name:`).
The job (cimg/node by digest) runs `git fetch origin main` into `origin/main` and sets `origin/HEAD`
before anything reads the base, since a stale cached `origin/<default>` would hide what main gained (a
deleted test); then, as bosun.yml, the release that origin/main's tip pins (as `base.sha`; never the
merge-base, which the author picks; the setup-PR fallback to this change's pin and `.bosun` only when
origin/main has no `.bosun`), checked by its version and by its sha256 before each gate, the pin
check, origin/main's gate config, the rust toolchain (rustup-init, cargo-nextest and cargo-deny each
checked against a pinned sha256; the toolchain file in rust_path or a parent, checked line by line
like bosun.yml's (only `[toolchain]` with plain-name channel, components, targets and profile), is
installed as it names, else 1.97, and its channel is exported as `RUSTUP_TOOLCHAIN`) or pnpm, and the gates of bosun.yml in order (a test keeps
the lists equal; with `own_ci` a pull request runs only test-integrity unless the project variable
`BOSUN_ALL_GATES=1`; main and `gh-readonly-queue/*` run all five). When HEAD is the merge-base (an
empty diff) every gate runs with `--all`. `BOSUN_INTEGRITY_ALLOW` and `BOSUN_PR_BODY` are empty: no
CircleCI variable can allow a finding, and a `snapshot-change:` line cannot apply (fails closed).
The ts-pnpm stack runs in the same job: test-integrity, then deps with every package.json and
pnpm-workspace.yaml from origin/main (as bosun-trusted; a path whose directory links outside the
checkout stops the job before anything is removed), then the PR's copies back,
`pnpm install --frozen-lockfile`, fmt, lint and test. There are no draft, docs-only or body-edit
fast paths.
Piece `circleci-only` (requires `circleci`; channel `min_splice` 0.3.0, whose contract rule 27 deletes
an owned file whose re-render is empty) drops the GitHub bosun.yml, and main-health then triggers on
`status` (`concurrency: main-health`) with a job-level `if:`: a terminal `ci/circleci: ci-ok` or
`bosun-ok` from the `circleci-app[bot]` Bot for `github.sha`, the default branch's head (a status
of an older main commit never counts); with `own_ci` main is green only when both contexts are
`success` on that commit, read across every page of its statuses (pending or missing is not). Its weekly gates are a CircleCI scheduled
pipeline (an owner step). Without `own_ci` (no hull) the channel also ships hull's
`.circleci/config.yml`, `.circleci/src/@config.yml` and the mirror
`.github/workflows/circleci-status.yml`, byte-identical to the copies vendored in
`tests/fixtures/hull/<ref>/` (`MANIFEST`); beside hull they render empty and hull owns them. risk,
verdict, main-open and splice.yml are the same in every mode.

The intake file `.bosun/intake.toml` (version 1) holds the owner's upfront answers: `repo` {name, owner, visibility private|public}, `stack` (rust|ts-pnpm, non-empty), `pieces` (hull piece names, non-empty), `bosun_pieces` (claude-md|circleci|circleci-only), exactly one of `goal` or `plan` (a `plans/*.md` path), `publish` (npm|crates|cloudflare|none; `none` stands alone), `approvals` (owner|owner-gated|orchestrator|autonomous), `notify` (chat|slack) and `slack_channel` (required and non-empty with slack, empty with chat). An unknown key at any level, a `[signature]` table included, is an error (the signature is a detached file). `bosun intake check [FILE] [--from -]` (default FILE `.bosun/intake.toml`) validates the schema only and handles no signature: a missing answer (absent or empty field) is exit 5, a bad value, unknown key or bad type is exit 2, each naming the field in `data.field` and `error.fix`; `data` has the parsed fields. It sits next to `bosun intake "<text>" --source ...` (spec §4), which is unchanged; a text of exactly `check` is read as the subcommand.

The signature of an intake is a detached TOML file `.bosun/intake.sig` with `key`, `repo`, `mode` (new|adopt), `nonce` (32 random hex chars), `issued` and `expires` (RFC 3339), `intake_sha256` (hex of the exact intake bytes) and `sig` (base64 ed25519 of `bosun-intake v1 <owner/name lowercased> <mode> <nonce> <issued> <expires> sha256 <intake_sha256>`). `bosun intake sign FILE --sign-key PEM --repo <owner>/<name> --mode new|adopt [--key-id ID] [--expires-in SECS] [--approvers PATH]` validates, then prints the file in `data.sig` and writes nothing; `--repo` must equal the file's repo (exit 2), `--expires-in` is 1 to 3600 (default 3600; more is exit 2), and the key id is `--key-id` or the id of the key in the approvers file (default `.bosun/approvers.toml`) with that public key. `bosun intake verify FILE --sig SIGFILE --approvers PATH` is for provision.yml only, which passes its own checked-out main's approvers file: after the schema check it needs the sha256 to match, the repo to match, `expires - issued` to be at most 3600 s (`window_too_long`), `issued` to be at most 60 s ahead of now (`BOSUN_NOW`; `issued_in_future`), `expires` to be after now (`expired`) and at most 3600 s ahead (`expiry_too_far`); these caps on verify are the real control, since sign takes its clock from `BOSUN_NOW`, and `sign` sets `issued` to now; verify also needs the key to be enrolled with the `owner` role (the orchestrator's key is enrolled as an owner key, as for `bosun answer --chat`; a `delegate` key never signs an intake) and a valid signature. Every failure is exit 5 with `data.reason` one of `bad_intake` (unreadable or fails the schema), `bad_approvers` (unreadable or malformed), `bad_sig_file`, `intake_changed`, `repo_mismatch`, `window_too_long`, `issued_in_future`, `expired`, `expiry_too_far`, `unknown_key`, `role_not_allowed`, `bad_signature`; on success `data` has `key`, `repo`, `mode`, `nonce` and `expires`. Recording used nonces is provision's job. The `--sign-key` path never appears in an error. A text of exactly `sign` or `verify` is also read as the subcommand.

The channel seeds `.bosun/intake.toml` (piece `core`, written once, then the repo's own) as a template rendered from `project`, `owner` and the stack pieces: `repo.name`, `repo.owner` and `stack` are filled in, every other answer is empty, so `bosun intake check` on a fresh seed exits 5 (a missing answer, never accepted), naming the first empty field (`repo.owner` when the `owner` var was empty, else `repo.visibility`). `.bosun/bosun.toml` takes an optional top-level `intake` bool (default false; a wrong type or any other unknown key is an error). The channel renders `intake = true` only when the splice var `intake` is non-empty and `bosun_version` is empty (the repo pins the channel's own release); without the var the render is unchanged. Release ordering: pinned releases reject unknown keys, so bosun's own `.bosun/bosun.toml` does not set `intake` until a pin PR moves `.bosun/version` to a release that reads it (0.5.0) and adds it to KNOWN in `tests/pinned_config.rs`.
