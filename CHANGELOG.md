# Changelog

All notable changes to bosun. The format follows [Keep a Changelog](https://keepachangelog.com/).

## Unreleased

- `bosun approve --list` adds `data.message`, `data.message_id` (8 hex characters) and `data.message_pairs`: one short chat text for the owner with inert titles, at most 10 PRs, and the `Reply: approve ...` line; the id hashes only the shown `pr@head` pairs (all null when nothing is ready).
- `core::text::inert`: text an agent or a plan wrote, shown to a person as one plain line (control, format and separator characters as spaces, as an inline code span, cut to a length).
- `core::prereqs`: `approvals` estimates which plan tasks will need the owner or a delegate at merge (as risk-check would, from touch paths; `risk: high` needs the owner), in dependency batches; `message` is the short checklist text for the owner's chat.
- `core::prereqs`: the owner checklist as an issue body. Plan and agent text is shown inert. Each line carries a bosun marker with a content digest; `read_issue` and `ticked` count a tick only on a single line whose digest matches the plan. Ids, blocks and verify kinds come only from the plan. The parser takes `replaces:`.
- `bosun prereqs <plan>... [--no-setup]`: the human steps of a plan, checked through the API (secrets, variables, environments, files) and the ticks of its checklist issue, plus the `bosun doctor` steps, the approvals at merge and a delegate key, with a short message for the owner's chat.
- `core::prereqs`: parse the `## Prerequisites` section of a plan (who, phone or computer, steps, verify, blocks) and find the prerequisites that block a task.
- Docs: spec §14 (front-loaded human prerequisites), the plan-format pointer, and AGENTS.md rules: front-load human steps, never wait mid-run (`bosun hold`), relay approvals in batches.
- docs: design and plan for front-loaded human prerequisites (D-009), and the `## Prerequisites` format in `docs/prerequisites.md`.
- `bosun prereqs` builds each plan's checklist in one reusable function (`Env::checklist`); no behaviour change.
- `bosun claim` refuses (exit 5) a task that an open owner-checklist item blocks and `bosun spawn` refuses an epic blocked as a whole; ids, blocks and verify kinds come from the plan on the default branch, the checklist issue only gives ticks, and anything unreadable or ambiguous fails closed.
- `bosun hold <task> "<what the owner must do>" [--phone]`: an agent that meets a human step nobody listed records it on the task, adds a box to the plan's owner checklist and puts the task back to ready, instead of stopping to ask.
- `bosun hold` ids stay unique for the life of a plan (they skip the ids the plan `replaces`), `replaces:` releases only a hold of a task its item blocks, a hold re-adds its box once if a parallel write dropped it, and `bosun prereqs` lists a plan named twice once.
- The markers-lock hook treats a `gh api` or `curl` call with several method flags (`-X GET --method PATCH`, `-XGET -XPATCH`) as a write when any of them is a write method; the `gh api graphql -F query=@file` gap is listed with the known bypasses.
- feat(hooks): the PreToolUse hook refuses agents' GitHub writes that name the owner checklist marker `bosun:prereq` (best effort, design F4).
- `bosun prereqs --post` keeps one checklist issue per plan (label `bosun:prereqs`): created when there is something to do, rewritten from the API and the owner's ticks, closed when nothing is open and reopened when something is.
- `bosun plan import` and `plan amend` refresh the owner checklist of a plan with a `## Prerequisites` section (`--dry-run` only reads) and keep a task `planned` while an open item blocks it or its item; they are listed under `data.held`.
- `bosun wave plan` leaves out ready tasks that an open owner-checklist item blocks (`data.held`; when the checklist cannot be read, every ready task of the plan, with a warning that names the fix), and `bosun spawn` refuses an epic blocked as a whole item.
- Design and plan: gates as check runs of a dedicated GitHub App pinned in the ruleset (B2).
- Design and plan: owner teams in governance/owners.toml, expanded per run and failing closed (B4).
- docs: design (proposed, not built): spec §17 clean template updates, a lower approval class for clean splice bot updates; plan `docs/superpowers/plans/2026-10-28-clean-template-update.md`.
- `docs/plan-format.md` explains how to cut tasks by file ownership so parallel PRs never conflict; AGENTS.md and the channel's agents section point to it.
- Design and plan for parallel PRs without merge conflicts (truthful leases, plan overlap, spec split, approval carry-over).
- `docs/spec.md` is split into `docs/spec/NN-<section>.md`, one file per section (a pure move), so PRs that change different sections never conflict.
- docs: spec 08 notes that the default workflow token should be read-only, and links #298 (bind verdict records to the verdict run).
- `bosun risk-check` carries a high-risk approval (owner review, comment, signature or the class's verdicts) of an older head over a clean merge of main (`data.approval_carried_from`); only the approval, everything else is recomputed on the new head.
- `core::carry`: the pure rule that decides whether an approval carries over to a head that is a clean merge of main into the approved head (not wired yet).
- `bosun risk-check` also carries a delegate approval (v1 signature plus one clean verdict) over a clean merge of main, and refuses any carry when an owner withdrew on any commit, even off the path.
- `risk-check`: the carry-over reader gathers the facts by sha from GitHub (commits, trees, compares) and hands them to the pure rule; any doubt refuses.
- `github`: reads for git commits, recursive trees and the compare merge base, with a truncation flag on compare.
- `bosun risk-check` carries the `review-verdict` status across a clean merge of the default branch (`data.verdict_carried_from`), and restores a real verdict that raced the carry.
- Channel piece `circleci-only` (no GitHub bosun.yml; main-health on the CircleCI status of main's head only, both checks `success` with own_ci) and, without hull (`own_ci` empty), hull's CircleCI setup config and status mirror, byte-identical to copies vendored from hull 58c488b. The channel needs splice 0.3.0 (`min_splice`).
- Channel piece `circleci`: a CircleCI job `bosun-ok` (or `ci-ok` without `own_ci`) that fetches the default branch first and runs bosun.yml's gates with the pinned, sha256-checked bosun.
- Risk: R-ci and the critical floor cover GitLab (`.gitlab-ci.yml`, `.gitlab/ci/**`), Buildkite, Jenkins (`**/jenkinsfile`, any case), Azure Pipelines, Bitbucket Pipelines, Travis and Drone config.
- `bosun claim <issue> --touch <glob>` on a task the session already holds adds the globs to its lease after the same overlap check (exit 3 on a clash) and posts a newer lease record with the union (`data.added`); a grow that loses a race restores its previous record.
- Owner approvals by chat form (D-012): docs, agent rules, `risk-check` and `bosun approve` fixes and hints, and `bosun init` owner steps say the session asks the owner with a form in the Claude Code chat and the orchestrator posts the signed `bosun approve` comment; an owner-written `/bosun approve <sha>` comment is the fallback.
- Bootstrap mode (D-012): `[approvals] bootstrap_until` in `.bosun/bosun.toml` and `bosun approve --sign-key <pem> --bootstrap`, which signs `bosun-bootstrap v1`; risk-check counts it as the owner until that day (UTC, inclusive) on owner-class PRs that touch none of `BOOTSTRAP_EXCLUDED` (the setting, the approval code, governance, risk rules, `hoist.toml`, environment workflows, R-secrets).
- The hook refuses decrypting secrets for agents (`sops` other than version/help/encrypt, `age -d`, `hoist secrets edit|sync|grant|revoke|rotate*`), reading the sops age key and printing `SOPS_AGE_KEY`.
- **Added:** `bosun doctor --remote` checks the environment of every workflow job (each `environment:` in `.github/workflows/*.yml`): step 5 warns, never fails, unless only the branch `main` may deploy to it (custom deployment branches, exactly `main`); a missing environment, no policy, protected branches or any other rule names the Settings → Environments steps, and a 403 says it cannot verify. `data.environments` lists each one.
- The edit hook keeps a session with a lease inside its own touch: an existing file outside it (and its lockfile companions) is denied with the `bosun claim --touch` fix; fragments and new files stay free.
- `bosun gate` and `test-integrity` find their base on CircleCI (`CIRCLECI=true`): merge-base with the default branch for a PR, `--all` for a push to the default branch or a merge-queue branch (queue base sha); the output carries `data.ci`. GitHub and local runs are unchanged.
- A lease race is decided per contested glob pair (each side counts from its earliest record that overlaps the other glob), so a grow keeps its priority on its old globs, races at grow time for the added ones, and the two sides never both win.
- `bosun main-health --result success` flags a queued task whose `task-pr` PR became conflicting (`mergeable_state` dirty): one `conflict` record on the task and one comment on the PR, the task stays queued; `data.conflicts`, `skipped_unknown` and `conflict_errors` report it.
- `bosun plan check` warns when two tasks touch overlapping files with no `depends` path between them, and when 3 or more parallel tasks touch one file; `Cargo.toml` and `package.json` count their lockfiles.
- `bosun plan check` now fails on a touch glob with braces, classes, escapes or `..`, and its overlap warnings stop counting at 1000 pairs ("at least N more").
- `bosun pr` exits 3 when the PR modifies, removes or renames files outside this session's lease touch (lockfile companions, added files and the fragments always pass); a human PR without a lease or a diff of 3000+ files skips the check.
- risk-check posts `pending` on the head while it judges, so auto-merge waits; a run that cannot finish posts `error` instead of leaving `pending`.
- Risk: R-secrets also covers sops files (`secrets.toml`, `access.toml`, `.sops.yaml`, `*.sops.yaml|json`) and age keys in added lines; `.circleci/**` is R-ci and in the critical floor.
- `bosun risk-check` warns (data.out_of_touch, a line in the Risk note of a high-risk PR body, one stderr line) about PR files outside the linked task's live lease; it never changes the risk level or the status state. `bosun pr` skips the touch check for a lease with an empty touch (a spawn's epic lease).
- `risk-check`, `approve`: a v2 owner signature (`bosun-approve v2 ... after-review <review_id>`) bound to the current dismissed owner review clears a dismissal; `approve --sign-key` signs it on a dismissed PR; v1 signatures, delegate keys and older review ids never clear.
- splice 0.3.0 everywhere (both setup scripts; both `splice.yml`, whose `keelstackhq/splice/action` is now pinned by commit). The setup scripts install splice, hoist and berth (and, in bosun's own script, bosun) only from `npm pack` tarballs whose sha512 matches the pinned npm integrity (launcher and linux binary package), and skip a tool with a warning on a mismatch or a failed install. After it merges, paste `docs/setup-script.sh` into the cloud environments again.
- `.bosun/bosun.toml` drops `[approvals]` until a release that reads it is pinned: the pinned 0.2.3 rejected the key and failed every gate on main; `tests/pinned_config.rs` keeps the file to the keys the pinned release knows.
- `bosun claim` re-reads the comments after setting a lost claim back to `ready` and restores `claimed` when a third session won meanwhile; the edit hook resolves the edited path with `canonicalize`, so symlinks and `link/../x` are judged by their real target.
- `bosun claim` re-lists the active issues after setting the task `claimed`, so two ready tasks claimed at the same time with overlapping touch no longer both win; the loser is set back to `ready` (exit 3).
- Hooks: the decrypt-lock lets ack and ugrep (also `ack-grep`, `ugrep+`, `ug+`) run only a strict allowlist of pagers (`less`, `more`, `most`, `cat` with plain flags, no less `-k`/`-o`/`-O`) and `--view` editors (name alone), refusing shell syntax, other paths, `LESSOPEN`/`LESSKEY`/`LESSSECURE`/`BAT_PAGER` and `PAGER`/`LESS` values that fail the allowlist; it reads `-nd recurse` clusters, treats `grep.fallbackToNoIndex` (`-c`, `--config-env`, `GIT_CONFIG_KEY_<n>`) as `--no-index`, and takes unique prefixes of `git grep --no-index`, GNU grep `--recursive`/`--dereference-recursive`/`--directories=recurse` and ag `--hidden`/`--unrestricted`.
- decrypt-lock: a `git grep` whose `-C`/`--work-tree` base is a key root (`git -C ~ grep x`, `git -C ../../root grep x`) is judged like `--no-index`, whatever config sets `grep.fallbackToNoIndex`; vim and nvim init variables, and ugrep's `EDITOR`/`VISUAL` view fallbacks, are judged when ugrep can open a viewer (`-Q`, `--query`, `--view`); a bare ugrep `--pager` or `--view` no longer takes the next word as its command.
- Hooks: the decrypt-lock refuses a recursive grep, rg or ag whose root holds the age key (.config and ancestors, any pattern), any search under .config/sops, and bun reading the environment.
- decrypt-lock: a `git grep` pathspec-magic root (`:(exclude)x`, `:!x`, `:/`) is judged as its base, and a `-C`/`--work-tree` base of one segment after two or more `..` (`../../user`) counts as a possible home.
- Hooks: the decrypt-lock judges egrep, fgrep, rgrep, zgrep, ugrep, ug, pcregrep, ack and git grep --no-index (with -C) by root too, judges the option values of ack and ugrep as commands, refuses .. chains and ~user homes, and resolves dir/.. before matching.
- Hooks: the decrypt-lock judges the command after doas, chrt, taskset, unbuffer, watch (also as its sh -c string), flock, busybox, strace, ltrace, unshare, chroot and caffeinate; unshare -w moves the base and a new root counts as /.
- `risk-check`: an owner request for changes that someone dismissed (GitHub then reports DISMISSED) still withdraws comment, signed and delegate approvals; only a newer owner APPROVED review or a newer plain-token owner comment clears it, never a signature.
- `bosun reap` closes epics whose tasks are all merged or closed and quiet for 10 minutes (`data.epics_closed`, best-effort with `data.epics_error`), and `plan import`/`amend` reopen such an epic when it has an open task again (`data.reopened`, `data.would_reopen`).
- `bosun gate` and `test-integrity` also see staged, unstaged and untracked changes: they select the gate, and `data.uncommitted` lists them.
- The edit hook judges a repo-root file named `..x` as inside the repo (only a real `..` path component means outside).
- Hooks: a session may edit paths leased under its own `<session>/<task>` ids; the label-lock no longer refuses read-only GitHub MCP tools or heredoc prose that names a state label; AGENTS.md says to run `git checkout -b` as its own command.
- `bosun init` sets `[github] merge_queue = false` in an existing hoist.toml when the key is absent (bosun's statuses are not posted on `merge_group` refs), keeps an explicit bool, warns in `data.warnings` and the PR body on `true` or when it cannot place the key, refuses a non-bool, and accepts `[ github ]` and `["github"]` headers; `data.hoist_merge_queue` says `added`, `kept_true`, `kept_false` or `unset`.
- `bosun plan amend` and `plan import` make the plan task of an item already merged when a merged intake PR added the item to the plan file, and promote its epic and tasks at once, but only while the local item equals the one in the plan file on the default branch.
- `plan import`/`amend` skip an intake whose merge commit GitHub answers with 422 instead of failing; `reap` keeps an epic open while a hand-made open task carries its item.
- The label-lock skips only an explicit allowlist of read-only GitHub MCP tools (`READ_ONLY_GITHUB`), pinned by a test, so an unknown `get_*`/`list_*` tool is judged like a write.
- `bosun pr` names the fix when auto-merge is off or the branch is not pushed, titles the PR after a conventional head commit, the review pack tells reviewers when not to `escalate_risk`, and the channel pins splice 0.2.1.
- `risk.yml` (bosun's and the channel's): `/bosun recheck` runs only for the repo's owners, members and collaborators (`author_association`); anyone else's runs no job and never cancels the judge. It limits CI minutes only (an outsider's `/bosun approve` still re-judges, harmlessly); a recheck from an owner with private org membership or an app (`NONE`) is dropped, so risk-check's fix and the docs now also say to re-run the risk workflow. `/bosun approve` is unchanged.
- risk-check orders an owner's decisive reviews by `submitted_at` (a missing time fails closed), not by list order.
- risk: every `hoist.toml` is R-gov and the root one (the ruleset's required checks) is on the critical floor (owner only, never a delegate), so a PR cannot drop a required check at normal risk; R-gov lists the agent-instruction files in lowercase (`docs/agents/**`, `**/agents.md`, `**/claude.md`, `**/claude.local.md`) so any case matches, and `docs/setup-script.sh` (its pasted copy runs as root in cloud environments), in the built-ins, bosun's own rules and the channel.
- **Security:** `risk.yml` (bosun's and the channel's) no longer runs on `pull_request_review`, which ran the PR's own copy of the file with `statuses: write`, so a same-repo author could rewrite the job and forge `risk-check` with a COMMENT review; an owner's GitHub review (or its withdrawal) now counts on the next run, and a `/bosun recheck` PR comment (anyone) starts one without a push. The `on: pull_request` statuses residual stays open until B2.
- risk-check quotes file names in the Risk note and replaces backticks, angle brackets and control characters, so a file name cannot end the note.
- **Security:** workflow secrets sit behind main-only environments, since a same-repo PR's own `on: pull_request` workflow can read repository secrets: `SPLICE_BOT_TOKEN` in `splice-bot` (splice.yml job `update`, bosun's and the channel's), `HOIST_RELEASE_TOKEN` in `release-pr` (release.yml jobs `prepare` and the new `release-token`, which takes the token check out of `detect` so a dry run still works on any branch). A guard test requires a literal environment on every job that reads a secret other than `GITHUB_TOKEN`. `bosun init` tells the owner to create `splice-bot` first.
- Setup scripts: a `rust-toolchain.toml` channel that is not a toolchain name (`^[A-Za-z0-9][A-Za-z0-9._-]*$`) never reaches `rustup`; it is skipped like a missing file, with an `invalid channel` warning.
- The setup scripts (the channel's `docs/setup-script.sh` and bosun's own) install the `channel` of the repo's `rust-toolchain.toml` instead of a hard-coded Rust 1.97; without the file they warn, skip the Rust toolchain and the cargo tools, install the rest and exit 0; `--rust-version` prints the version.

## 0.2.3 (2026-10-26)

- The channel's bosun.yml runs a ts-pnpm or mixed stack in two jobs (#234): `bosun-trusted` runs test-integrity and deps with no PR code (no install, no cache; `pnpm audit` reads the PR's lockfile with the base's pnpm config and an exact pnpm release), and `ci-ok` (or `bosun-ok`) installs and runs fmt, lint and test only after it passed. The pnpm allow-list, install fingerprint, `--ignore-scripts` install and `pnpm rebuild` of #233 are gone; rust stacks stay one job.
- bosun's own `.bosun/risk.toml` delegates R-ci, R-gate and R-deps again now that `.bosun/version` pins 0.2.1; after the critical floor a delegate decides `deny.toml`, the toolchain files, `.config/**` and `profiles/**`.
- The channel's docs call `.bosun/fold-fragments.sh` through bash (`bash .bosun/fold-fragments.sh`): splice ships files without their mode (template format 9.13), so the script lands as 0644; a test keeps every shipped script call behind `bash`.

## 0.2.2 (2026-10-26)

- CI minutes (docs/ci-minutes.md): the channel's bosun.yml runs every gate in one job named after the required check (`ci-ok`/`bosun-ok`) instead of nine (the gates that run no PR code first); a draft fails it at once; after a PR body or title edit only test-integrity runs again on a head that passed; main-health runs the gates weekly, not nightly. AGENTS.md: run every gate locally before a push, push once per PR.
- release.yml: `prepare` (the release PR) runs only when the secret `HOIST_RELEASE_TOKEN` is set; without it the job failed on every push to main (GITHUB_TOKEN may not open PRs here) and is now skipped with a warning.
- CI minutes: bosun's own ci.yml runs everything in one job `ci` (9 jobs before), `ci-ok` aggregates it; drafts fail at once, edits rerun only test-integrity on a passed head, the pin check runs before the gates; the channel's main-open and main-health comments name bosun.yml's pinned-release steps.
- CHANGELOG and PROGRESS entries are one-line fragments (`changelog.d/`, `docs/progress.d/`) that `scripts/fold-fragments.sh` folds in a docs PR to main before a release, so merging main into a PR no longer conflicts on them; the channel ships the script as `.bosun/fold-fragments.sh` and the rule in AGENTS.md.

## 0.2.1 (2026-10-26)

- bosun's own `.bosun/risk.toml` delegates nothing until `.bosun/version` pins 0.2.1: the pinned 0.2.0 risk-check rejected `approval = "delegate"` and failed every PR. A test keeps the rules parseable by the pin.
- No signed approval (owner or delegate key) counts on a PR that changes `.bosun/approvers.toml`, `governance/owners.toml` or a CODEOWNERS file: key and owner changes need the owner's own GitHub approval, as does a diff bosun could not see whole. `.github/actions/**` is R-ci.
- Delegated decisions: `approval = "delegate"` rules and `role = "delegate"` keys; risk-check counts a signed `bosun-delegate v1` approval with one clean independent verdict, never on a critical change (hard-coded floor: every workflow, governance, bosun's code), while halted, or for a PR whose linked issues are not open leased tasks; verdict records carry `major`; `bosun approve --as delegate`. The kill switch and notify read owners at `refs/heads/<default>`.
- `bosun pr` refuses an open PR on the branch that does not link the task (exit 3).
- A human merge moves a building, review or fixing task to merged: `bosun pr`, `bosun reap` (also for closed issues) and `bosun result --outcome merged --pr N` see it; leases end with reason `merged`.
- `bosun init --branch <name>`; in a Claude cloud session the default is `claude/bosun-init`; `--no-pr` returns the PR title and body. The owner steps say each weekly splice update PR needs `/bosun approve <sha>`.
- `bosun doctor` sees the Claude GitHub App through `performed_via_github_app`, and step 3 reads the checks the default branch enforces (403: "cannot verify").
- The label-lock hook lets read-only commands that mention a state label pass (grep, git show, heredoc prose, `gh issue list --label`); any other command naming one is refused. The hook denies a call when its guard panics.
- `verdict apply`: a newer verdict comment replaces only one of the same session, so two reviewers posting close together both count.
- A PR body edit no longer cancels the running CI of the same head (bosun's ci.yml and the channel's bosun.yml).
- Channel: the gates install the toolchain of `rust-toolchain.toml` (else the 1.97 pin); with own_ci a PR runs only test-integrity unless `BOSUN_ALL_GATES=1`; a docs-only PR (Markdown, not README.md, not with ts-pnpm) skips the gates; `docs/agents/**`, `**/AGENTS.md` and `**/CLAUDE.md` are R-gov.
- Signed owner approvals: risk-check counts `/bosun approve <head>` with a `bosun-sig ed25519` line from a key in `.bosun/approvers.toml` on the default branch, bound to repo, PR and head; `bosun approve <pr>@<head> --sign-key <pem> [--print]` signs and posts it, also from a Claude cloud session.
- Risk rules: the owner approves all workflows, `deny.toml`, the toolchain files, `Cargo.toml`, `Cargo.lock`, build inputs, `REVIEW.md`, `docs/lessons.toml` and the gate code (R-gate); `channel/**` passes with two independent verdicts instead.
- `bosun approve --list` returns a hint: approvals posted through the Claude GitHub tool never count.
- `bosun approve` refuses to post from a Claude cloud session (its proxy rewrites GitHub writes); a PR whose owner review requests changes is not ready; a counting approval is re-posted when risk-check is still red.
- `bosun reap` reads the PR that `bosun pr --auto-merge` recorded on the task (a `task-pr` record) instead of listing every PR; tasks queued before this keep the old sweep.
- `bosun result --outcome blocked|needs-human` sets that state on the epic; `merged` stays a report.
- `bosun init` detects a repo's own `ci-ok` check by the job's name (or key without a name), at any indentation.
- `bosun approve`: the missing-token fix says to run it on your own machine.
- The channel's main-open and main-health comments name bosun.yml; a test renders every channel variant and parses the output.

## 0.2.0 (2026-10-24)

- `bosun fail`: stop rules (3 equal failures or 0 changed lines block the task) and tier escalation, with `failure` and `escalate` records.
- `bosun claim` refuses new work with exit 6 at 80 % of the daily budget, like `spawn` check 2; mainfix tasks and a re-claim of your own lease are exempt (UC-M4-1).
- `bosun budget check`: the level (normal, guard, stop) of today's spend; at 100 % one `interrupt_session` per live remote session in `next`, for the janitor (UC-M4-2).
- `bosun budget record`: the record that blocks a task over `per_task_usd` opens a `budget` lesson, or adds a hit to it (UC-M4-3).
- `bosun progress [--days N]`: cost per merged task over the last N days, read-only (UC-M4-4).
- `wave plan` and `status` read `.bosun/limits.toml` from the default branch, like `spawn`, `claim` and `budget record`; `status` reports `daily_usd`.
- `bosun pr` moves a claimed task through building to review; `bosun pr` and `bosun reap` move a queued task to merged once GitHub merged its PR (agents never merge).
- `bosun fail`: a stop opens or hits a `stop-rule` lesson.
- `bosun resume`: after a container restart, renew the session's join record and leases and list the open PR and next action of each held task.
- `bosun result <epic> --outcome .. [--pr N]`: a child posts its result, ends its epic lease and leaves the registry; the spawn prompt uses it.
- `bosun pr --auto-merge` works in Claude Code cloud sessions: it falls back to the CCR auto-merge route when GraphQL is refused; the merge-lock also blocks `gh api` auto-merge calls.
- The janitor Routine prompt (`bosun doctor`) runs `bosun budget check` and makes its `interrupt_session` calls.
- `bosun gate main-open --pr N`: posts the required `main-open` status; while main is red only PRs labelled `bosun:kind/mainfix` (or linked to an open issue with that label) pass (UC-M3-2).
- `bosun main-triage --good <rev>`: bisects the merged PRs on main and names the first bad one (UC-M3-3).
- `bosun revert <pr>`: opens a `bosun:kind/mainfix` PR that reverts a merged PR, without touching your checkout (UC-M3-4).
- `bosun main-health --result <conclusion>`: opens or closes the `bosun:main-red` issue, opens a `main-red` lesson, and re-posts `main-open` on every open PR (UC-M3-1).
- CI: `main-open` and `main-health` (bosun's and the channel's) run the pinned bosun release, with a shell fallback until it has the commands; main-health ignores fork runs from a branch named main; splice is pinned to 0.2.0.
- `bosun approve --list` and `bosun approve <pr>@<sha>...`: the owner approves ready high-risk PRs from their own Claude Code session, posting the same `/bosun approve <sha>` comment risk-check already counts.
- risk-check: a two-verdict approval class (mechanism only; the rules are unchanged).
- `bosun plan check <file>` and `bosun plan import <file>`: epics, tasks and one plan task per item from a plan file; a second import creates nothing (UC-M2-1).
- `bosun plan import` moves an item's tasks to ready once its plan task merged (UC-M2-2); it writes no UC test stubs, the UC tests land with their code (#199).
- `bosun plan amend <file>`: items and tasks gone from the plan close as not planned; changed ones update in place (UC-M2-4).
- `bosun intake "<text>" --source ..`: opens a kind/plan issue that asks for a plan amendment.
- `bosun lesson capture`: a failure opens a lesson issue, or adds a hit to the open lesson with the same signature (UC-M5-1).
- Process-red: an open `bosun:process-red` issue pauses `claim`, `spawn` and `wave plan` for all but mainfix and process-fix work (UC-M5-2).
- `bosun lesson triage <issue> --as blocking|mechanizable|advice`, called by the curator Routine prompt.
- A hit on a mechanized lesson reopens it as blocking (UC-M5-5); `lesson add --upstream`; `bosun progress --lessons`.
- The merge-lock follows `cd`, `git -C`, `--git-dir` and `--work-tree`; the label-lock reads label fields only.
- Paging works through the Claude Code cloud proxy.
- `risk.yml` and `verdict.yml` run the pinned bosun release instead of building main.
- `bosun review-ci <pr>`: review mode (b). The review verdict comes from one Anthropic API call in CI, with no session (UC-M8-1); it fails closed on a diff it cannot see or a request over the cap, and does nothing unless `review.mode = "ci"`. `--dry-run` records a verdict and writes nothing.
- A later review covers only the new commits, and only from an approved ancestor that is one of the PR's commits (UC-M8-2).
- `core::agreement` and the recorded-review fixtures for comparing mode (a) with mode (b) (UC-M8-3 waits for 20 recordings).
- risk-check flags a file with changed lines and no patch whatever its name.
- `bosun notify`: owner questions, halt, main-red, budget-halt and thread results as `slack_post` items in `next`, posted by Claude Tag or the notify Routine and acknowledged with `--ack`, so each post is made once; it refuses a channel that is shared or has guests (UC-M6-1, UC-M6-4). bosun holds no Slack token.
- `bosun answer`: an owner answers an ask from Slack; only owner Slack IDs count and the answer never changes the task state (UC-M6-2).
- `bosun spawn --slack-thread`: a Slack thread spawns a child (a new request goes through intake), with the same checks and budget as any spawn; the result is posted back into the thread (UC-M6-3, UC-M6-4).
- The splice channel `gh:keelstackhq/bosun//channel` (D16): pieces `core` (governance seeds, the `bosun`, `risk`, `verdict`, `main-open`, `main-health` and `splice` workflows, the AGENTS.md and REVIEW.md sections), `rust`, `ts-pnpm` and the optional `claude-md`. The gates run in `.github/workflows/bosun.yml` (aggregate `ci-ok`, or `bosun-ok` beside a repo's own ci.yml); consumer workflows run the release pinned in `.bosun/version`, never PR code.
- `bosun init --stack <rust|ts-pnpm>... [--plan FILE] [--owner LOGIN] [--source SRC] [--ref REF] [--bosun-version X.Y.Z] [--no-pr]`: adds the channel on branch `bosun/init` and opens one R-gov setup PR with the owner steps. Beside a hull template it sets `own_ci`, skips `claude-md` when the template owns CLAUDE.md, excludes existing seeds and `.claude/settings.json`, and merges the required checks into an existing hoist.toml.
- Gate profiles: `manifest` (a stack whose manifest is missing at the head and the base is skipped, so an empty repo's setup PR is green), `requires`, the `{packages}` placeholder and a closed set of packages adapters (`cargo-metadata`, `pnpm-workspace`, `none`, `generic`).
- The `ts-pnpm` profile (root `package.json` scripts, `pnpm --filter`), the `ts-skip` and `ts` test-integrity adapters, and generic `regex` adapters for any stack, reported as reduced integrity; mixed stacks get one kit per stack.
- `bosun pr --auto-merge` and `bosun doctor` read the required checks as the defaults plus hoist.toml `[github] required_checks` from the default branch; doctor finds `ci-ok` in any workflow.
- `bosun doctor --remote` runs `hoist check` and folds its drift into steps 3, 5 and 10.
- `.bosun/risk.toml`: `channel/**` is R-ci.

### Features

- plan import writes no UC stubs; UC tests land with their code (#179) (#199) (b0e0d48)
- channel workflows run the pinned bosun (2/2) (#109) (cebf97b)
- one thread reply per child result (M6/T4) (#196) (c23a253)
- thread spawn with intake first, no exemptions (M6/T3) (#195) (8b69b49)
- bosun answer counts only owner Slack IDs (M6/T2) (#194) (63b60b3)
- bosun notify with the channel check (M6/T1) (#193) (36ec948)
- agreement harness and review-ci --dry-run (M8/T3) (#184) (d2d7565)
- bosun review-ci, review mode (b) with no session (M8/T1a) (#182) (28f12e0)
- delta since the newest approved head of this PR (M8/T2) (#180) (cdf576b)
- bosun main-health opens and closes main-red, opens a lesson (M3 T4) (#155) (0b49fcf)
- bosun intake (M2/T4) (#165) (377f31b)
- splice channel with pieces core, rust and ts-pnpm (1/2) (#107) (d3e50a2)
- ts-pnpm profile with the pnpm-workspace packages adapter (M7 T3 1b) (#122) (ee753f9)
- generic regex adapters, reduced integrity in status, mixed stacks (UC-M7-3, UC-M7-5) (#168) (d90fa30)
- --remote runs hoist check for steps 3 and 5 (#118) (cb277bc)
- work beside a hull template (own_ci, claude-md, excludes, hoist checks) (#157) (a69bb22)
- reopen mechanized lessons as blocking, --upstream, progress --lessons (M5/T4) (#166) (747a579)
- ts-skip and ts assertion adapters for test-integrity (M7 T3 2/2) (#123) (9e931d2)
- runner keys {packages} and requires; packages adapter dispatch (M7 T3 1a) (#167) (79ab011)
- required checks follow hoist.toml; doctor finds ci-ok in any workflow (#158) (8285d9f)
- bosun plan amend (M2/T3) (#163) (0627b7d)
- bosun revert opens a mainfix PR (M3 T3) (#111) (f7a126d)
- lesson triage and the curator prompt (M5/T3) (#154) (80a4c75)
- bosun init adds the splice channel and opens the setup PR (#112) (701bf2b)
- ready after the plan task merged, UC stubs, list-lag guard (M2/T2b) (#153) (b98a709)
- critical path and dependency cycles (M2/T1b) (#151) (b8bae91)
- janitor prompt runs bosun budget check (M1 janitor, UC-M4-2) (#174) (0bd9015)
- bosun main-triage bisects merged PRs (M3 T2) (#108) (5c50abd)
- bosun result for child results (M1 T4) (#116) (b2748d5)
- a task over per_task_usd opens a budget lesson (UC-M4-3) (#171) (adb980d)
- bosun budget check stops the live sessions at 100 % (UC-M4-2) (#169) (d50dc05)
- fall back to the CCR auto-merge route when GraphQL is refused (#164) (33f2a1f)
- a stop rule opens or hits a lesson (M1 T2b) (#149) (35082e9)
- bosun pr sets building, pr and reap set merged (M1 T1) (#115) (6747ffc)
- cost per merged task over the last N days (UC-M4-4) (#127) (1b3fe20)
- spend guard at 80 % of the daily budget (UC-M4-1) (#126) (569c846)
- bosun plan check and import (M2/T2a) (#124) (7817c6c)
- bosun gate main-open (M3 T1) (#117) (81c8824)
- plan-format parser, rules 1-5, 7, 8 (M2/T1a) (#113) (274b418)
- skip a stack whose manifest does not exist yet (#110) (f3f61ba)
- bosun resume after a container restart (M1 T3) (#105) (c0462f6)
- process-red pauses claim, spawn and waves (M5/T2) (#102) (012629d)
- capture lessons by failure signature (M5/T1) (#103) (6b7bf7e)
- bosun fail — stop rules and tier escalation (M1 T2) (#104) (7b3a42a)

### Fixes

- a file with changed lines and no patch is unseen, whatever its name (#181) (3d3d87e)
- the label-lock reads label fields, not prose (#162) (7e5efbb)
- wave plan and status read the limits from the default branch (#170) (8dbcc5d)
- COMMANDS holds 26 entries (plan and progress both merged) (#152) (ca13968)
- follow next-page links by owner/repo, not repository id (#120) (6a6b556)
- merge-lock reads the branch where git acts (#101) (f182f14)

## v0.1.0

The first release: the seed (S1-S10, `docs/superpowers/plans/2026-10-22-bosun-seed.md`).

- One JSON envelope on stdout for every command, fixed exit codes 0-6, `error.fix` always set.
- Identity, config and time: `BOSUN_REPO`, `BOSUN_SESSION`, `.bosun/*.toml`, `BOSUN_NOW`.
- Coordination on GitHub issues: `join`, `claim`, `heartbeat`, `release`, `reap`, `status`, with
  leases in the registry issue and the kill switch (an owner's `bosun:halt` issue).
- Spawning and limits: `spawn`, `wave plan`, `wait`, `msg`, `budget record`; the
  claude-code-remote calls come back in `next`.
- Gates: `gate` (rust profile), `test-integrity`, `ci-log`.
- Review and risk: `review-pack`, `verdict post|comment|apply`, `risk-check`, `pr --auto-merge`,
  `ask`.
- Learning loop: `lesson add`, `lesson hit`.
- Hooks: `hook pre-tool-use` (edit-, label- and merge-lock) and `hook session-start`.
- `doctor`, which also prints the janitor, curator and notify Routine prompts.

Install: `npm i -g @keelstackhq/bosun`, `cargo install keel-bosun`, or a binary from the
GitHub Release.

### Added after the seed

- `risk-check` accepts an owner's PR comment `/bosun approve <head sha>` (full 40-character SHA,
  unedited) as the approval of a high-risk PR, beside a GitHub review. The `risk` workflow also
  runs on such comments, from the default branch.
- Kill switch: an open issue labelled `bosun:halt` and opened by an owner halts bosun (exit 4).
  Create the label with `gh label create bosun:halt --repo <owner>/<repo> --color B60205`.
- `bosun verdict comment` posts a verdict as a PR comment, and the `verdict` workflow runs
  `bosun verdict apply <comment-id>` to check it and post the `review-verdict` status, so cloud
  sessions (which cannot write statuses) can deliver verdicts.

### Fixed

- The `verdict` workflow has `pull-requests: write`: with `issues: write` alone, its verdict record
  comment on the PR got 403 after the status was posted (first full run, #73).
- `bosun verdict apply` reads only the ```json block of a verdict comment: the agent proxy
  appends a footer to every comment, which made the second verdict comment on #73 invalid.
- `bosun verdict apply` checks the author's repository permission (write or admin) instead of
  `author_association`: the workflow token sees an org member as `CONTRIBUTOR`, so the first
  verdict comment (#73) was refused.

### Changed

- risk-check counts a `/bosun approve` comment only when GitHub says no app wrote it
  (`performed_via_github_app` present and `null`). Agent sessions comment as the owner's login
  through the Claude app, so before this an agent could approve its own PR.
- The `risk` workflow runs on `pull_request_target` instead of `pull_request`: the judging job
  comes from the base branch, never checks out PR-head code, passes event data through `env`
  only, and has least permissions. `tests/workflows.rs` asserts it.
- The repo variable `BOSUN_HALT` is now a second, optional kill-switch source: 404 (unset) and 403
  (not readable from this token) are no signal; any other read error still halts. Before, a 403
  halted every session behind the agent proxy.
- `bosun doctor` step 5 ("kill switch") checks the `bosun:halt` label instead of the variable.
