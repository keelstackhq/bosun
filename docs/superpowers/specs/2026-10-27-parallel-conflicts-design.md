# Parallel PRs without merge conflicts: design

Date: 2026-10-27. Owner goal: "the whole mechanism must work ideally with no merge conflicts that
need to be resolved when there are PRs opened in parallel. So make sure working on the same file
is reduced to the max". The owner gave the goal and no further input; the decisions below are
the orchestrator child's, each with its reason.

## 1. What exists today

| Layer | Where | What it does | Gap |
| --- | --- | --- | --- |
| Plan | `core::plan::parse`, `bosun plan check` | parses `touch:` globs, size, depends | no overlap or hotspot analysis between tasks |
| Wave | `core::wave::plan` | never picks two overlapping tasks, nor one that overlaps a live lease | only as good as the touch lists |
| Claim | `cli::claim`, `core::lease::conflicts`, `settle_race` | exit 3 on overlap with a live lease; race settled by comment order | a re-claim ignores `--touch`, so a lease can never grow |
| Edit hook | `core::guard::edit_lock` | denies Edit/Write on a path another session leased | edits outside your *own* touch pass silently, so the lease stops telling the truth |
| PR / CI | `bosun pr`, `risk-check`, gates | nothing compares the PR's files with the task's touch | a PR may edit any file |
| Shared files | `changelog.d/`, `docs/progress.d/` (#237) | append-only lists became one file per PR | `docs/spec.md` and registries are still single files |
| After merge | none | no branch update; approvals bind to the head SHA | a merge of main voids the verdict and signed approvals |

So bosun already serializes tasks *whose touch lists overlap*. Conflicts still happen because
the touch lists do not describe what the PRs edit.

## 2. Evidence

- Hotspots, last 120 commits on main: `docs/spec.md` 31, `docs/PROGRESS.md` 22, `CHANGELOG.md` 18
  (both fixed by #237), `tests/channel.rs` 10, `tests/workflows.rs` 8, `tests/governance.rs` 7,
  `Cargo.toml`/`Cargo.lock` 7, `.bosun/risk.toml` 7, `src/cli/mod.rs` 6, `tests/common/fake.rs` 5.
- #228 merged main three times: a PROGRESS.md conflict with #226, then #227, then #231 (a
  CHANGELOG entry moved by hand). Its signed approval on `4ccf321` was voided by the merge
  `71247b5`; a new verdict and a new signature were needed on `9b9cfb1`, about two hours later.
- #227's branch conflicted in 9 files at once (spec, PROGRESS, `src/cli/mod.rs`, approve, risk_check,
  tests) against #226.
- Open now: #251 and #254 both edit the import block of `src/cli/plan.rs`, append tests to the
  end of `tests/cli_plan_amend.rs`, and insert next to the "Intake:" paragraph of `docs/spec.md`;
  #253 edits `docs/spec.md` too. None of these ran as planned tasks with truthful touch lists.

Root causes, in order: (1) touch lists are not enforced, so overlap checks see too little;
(2) a few shared files (spec, registries, end-of-file tests) are edited by almost every PR;
(3) a clean merge of main costs a full re-review.

## 3. Decisions, ranked by impact vs cost

Impact = how many of the observed conflicts it would have prevented. Cost = lines and CI minutes.
None adds a CI job or a CI run (D-007: CI minutes are scarce); every check runs inside commands
that already run (`bosun pr`, `risk-check`, the edit hook, `plan check`).

### C1. Truthful leases: the edit hook keeps edits inside your own touch (high impact, ~150 lines)

When the session holds at least one live lease with touch paths, the edit hook denies Edit/Write
on an existing file in the repo that no own lease covers, with the fix
`bosun claim <issue> --touch <path>` (C2). The check is earliest possible: before the edit, at
no CI cost. Reasons: every edit is then covered by a lease, so the existing claim, wave and hook
checks see the real file set, and two sessions can never edit the same file.

- Creating a **new** file is allowed (a new path cannot conflict textually unless two PRs add the
  same path, and another session's lease still blocks it). This keeps "put new tests in a new
  file" (C6) free.
- Always allowed: the session's own fragments `changelog.d/*.md` and `docs/progress.d/*.md`, and
  lockfile companions (C4).
- A session with no live lease is not restricted (as today), so ad-hoc work keeps working; the
  hook says on stderr that its edits are invisible to other sessions.
- Fail mode stays as today: a hook error allows with a warning; branch protection and C3 are the
  real guards.
- Known risk: A holds x and wants y while B holds y and wants x. Both get exit 3 from the claim;
  the fix says to finish or release one. C5 makes this rare by cutting plans by file.

### C2. A lease can grow: re-claim with `--touch` (high impact, ~80 lines)

`bosun claim <issue> --touch <glob>...` on a task the session already holds adds the globs to its
lease after the same overlap check as a new claim (exit 3 naming the holder), and posts a new
lease record (the newest lease is the live one, so no format change). Without `--touch` a re-claim
stays idempotent. Reason: C1 needs a way to make a lease true without releasing work.

### C3. PR files must match the lease (high impact, ~120 lines, no CI minutes)

`bosun pr <issue>` compares the PR's changed files (compare API, base...head) with the task's
lease touch, plus fragments and lockfile companions; added files always pass. Files outside it
are refused (exit 3) with the list and the fix `bosun claim <issue> --touch <paths>` and a
re-run. `risk-check` reports the same list (`data.out_of_touch`) as a warning in its summary, not
a failure, because a PR without a task (a human's) has no touch. Reason: the PR is the last point
where an agent can still fix the lease before reviewers spend time; CI only warns so humans are
not blocked.

### C4. Plan check computes overlap and hotspots (medium impact, ~200 lines)

`bosun plan check` (and `plan import`, which parses the same way) adds warnings, computed in a new
pure module `core::touch`:

- **Parallel overlap:** two tasks whose touch lists overlap and with no `depends` path between them
  (across items too) would run one after the other anyway (wave and claim serialize them), but the
  planner believes they are parallel. Warning with the fix `add "depends: <other>" to <task>, or
  split the shared file`. Not an error: the runtime is still safe, and existing plans keep
  importing.
- **Hotspot report:** a path or glob matched by 3 or more tasks is listed with its tasks and the
  fix "give one task the file, or split the file (see C6)". `data.hotspots` in `plan check`.
- **Lockfile companions:** `Cargo.toml` implies `Cargo.lock`, `package.json` implies
  `pnpm-lock.yaml` (same directory, or the workspace root); used by C1-C4 alike, so two tasks that
  add dependencies overlap at plan time.

Auto-adding `depends` at import was rejected: it would silently change the plan, and the runtime
already serializes the pair.

### C5. Planning guidance: cut tasks by file ownership (medium impact, ~60 lines of docs)

In `docs/plan-format.md` (a "Cutting tasks" section), AGENTS.md and the channel's `sections/agents.md`:

- One task owns a file; a second task that must edit it `depends` on the first.
- A task lists every file it edits, including the spec section, registries (`mod.rs`) and test
  files; new tests go in a new file `tests/<command>_<topic>.rs` instead of the end of a shared one.
- Dependency changes are their own task (touch `Cargo.toml` and its lockfile); on a lockfile
  conflict, regenerate (`cargo update -w`, `pnpm install`), never merge by hand.
- Versions change only in release PRs (already true through hoist; written down).

### C6. Split shared single files (high impact for the spec, cost varies)

- **`docs/spec.md` -> `docs/spec/NN-<section>.md`** (one file per § 1-13, `docs/spec.md` becomes a
  short index). The top hotspot (31 of 120 commits). With C1-C3 a task names the section file, so
  two tasks on different sections run in parallel and two on the same section serialize. Cost:
  a mechanical move of ~1000 lines, generated by a script and checked by
  `cat docs/spec/*.md == old spec`; it exceeds 400 changed lines, so it needs the owner's OK, and
  it must land after the open PRs that edit the spec (#251, #253, #254), or they all conflict.
- **`src/cli/mod.rs`**: each command's `Args` struct moves into its own module; mod.rs keeps one
  `pub mod`, one enum variant and one dispatch arm per command, sorted. Two new commands then
  conflict only when adjacent in sort order. ~700 lines moved, so later and optional.
- Not split: `tests/common/fake.rs` (5 hits; a new route is a small, distinct hunk), `README.md`,
  `.bosun/risk.toml` (R-gov, owner-edited). `docs/PROGRESS.md` Status/Blocked stay in place (rare
  edits since #237).
- Rejected: `.gitattributes merge=union`. GitHub's merge button and auto-merge do not run custom
  merge drivers, and union silently keeps both sides of real conflicts.

### C7. After a merge: no automatic branch updates; flag conflicts; carry approvals (medium-high)

- **No automatic update of clean PRs.** Branch protection does not require up-to-date branches
  (`hoist.toml`), so a PR behind main still auto-merges when GitHub can merge it. Updating every
  open PR after each merge would cost one CI run each (~4-7 min, D-007) and void approvals, for no
  gain.
- **Flag a PR that became conflicting** (~100 lines): `bosun main-health --result success` already
  re-judges every open PR; it also reads `mergeable_state`, and for a `dirty` PR posts a `conflict`
  record and moves the task to `fixing`, so `bosun status` and `bosun wait --pr` tell the owning
  session at once (while its context is fresh). Needs `src/github` (open in #251/#254), so after
  they merge.
- **Approval carry-over across a clean merge of main** (security-sensitive, R-high, owner signs
  off on the rule first). A verdict (review-verdict status) or a signed approval for head `H`
  also counts for the current head `H2` when all hold:
  1. every commit on the first-parent path from `H2` back to `H` is a merge commit whose second
     parent `M` is an ancestor of (or equal to) the default branch tip (at most 5 hops);
  2. the PR's own change is byte-identical: the set of `(filename, status, blob sha,
     previous_filename)` of `compare(M...H2)` (M = last merged main commit; its merge-base with
     `H2` is `M` itself) equals that of `compare(B...H)` (B = merge-base of `H` and the default
     branch);
  3. neither compare is truncated (R-unseen), no trust file is in either (TRUST_FILES), and the
     newest decisive owner review on `H2` does not request changes.
  Why it is safe: any byte the merge commit adds beyond `M` and `H` shows up in `compare(M...H2)`,
  either as a new file (set differs) or as a different blob (sha differs). A file main changed that
  the PR also changed gives a new blob, so no carry-over: the reviewer never saw that combination.
  CI (`ci-ok`) and `main-open` still run on `H2`, so behavioral breakage from the combination is
  caught by the same gates as any merge. Residual risk: a semantic interaction between main's
  change and the PR's in *different* files, the same risk auto-merge without "require up to date"
  already accepts. Where: `risk-check` (counts the carried signature and posts the carried
  review-verdict status, it already runs on every push with status rights), `core::approve`.

### C8. The channel (no extra cost)

C1-C4 and C7 live in the binary, so consumer repos get them with the next pin (`bosun_version`).
C5's guidance goes into `channel/pieces/core/files/sections/agents.md`. The fragment folder and
`fold-fragments.sh` are already in the channel. C6 is bosun's own layout and is not pushed to
consumers; the spec split is advice in the guidance ("split append-only and per-section docs").

## 4. Order and cost

| # | Change | Impact | Lines | Can start |
| --- | --- | --- | --- | --- |
| 1 | C4 plan overlap, hotspots, companions (`core::touch`) | medium | ~250 | now |
| 2 | C2 lease grows on re-claim | high | ~120 | now |
| 3 | C1 edit hook own-touch rule | high | ~200 | after 2 (both edit spec §4 conflict layers) |
| 4 | C3 `bosun pr` touch check | high | ~200 | after 1 (uses `core::touch`) |
| 5 | C5 guidance in plan-format, AGENTS, channel | medium | ~80 | after 1 (plan-format.md) |
| 6 | C3 risk-check out-of-touch warning | low | ~120 | after 4 |
| 7 | C6 spec split | high | mechanical | after #251/#253/#254 and 2-6; owner OK |
| 8 | C7 conflict flag in main-health | medium | ~150 | after #251/#254 (src/github) |
| 9 | C7 approval carry-over | medium-high | ~300 | after 6; owner signs off the rule |
| 10 | C6 cli/mod.rs Args split | low-medium | mechanical | last, optional |

The PRs of this plan follow the rule they introduce: their touch lists are disjoint unless
chained by `depends`.
