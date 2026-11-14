# Plan format

A plan is one Markdown file in `plans/` (for example `plans/bosun.md`). `bosun plan import` turns
it into GitHub issues (spec §4): each item becomes one epic, each task one issue, each "Done when"
line one UC. The format is plain Markdown that a person can read; bosun parses only the parts
below and ignores other text.

## Example

```markdown
# Plan: bosun

project: bosun

## M1: Lifecycle

area: lifecycle
depends: seed

The state machine, stop rules and resume.

### Tasks

- T1: State machine with the label-lock hook
  touch: src/core/state.rs, src/hooks/label_lock.rs
  size: 300
- T2: Stop rules and tier escalation
  touch: src/core/stop.rs
  depends: T1
  size: 250
- T3: Raise the janitor interval
  touch: .bosun/limits.toml
  kind: bookkeeping
  risk: high
  size: 5

### Done when

- UC-M1-1: a task moves planned -> merged through bosun commands and auto-merge only, and a direct label edit is refused.
- UC-M1-2: the same failure 3 times sets blocked and escalates one tier.
```

## Rules

1. The first `# ` heading is the plan title. A `project:` line names the bosun project; it must
   equal `project` in `.bosun/bosun.toml`.
2. Each `## <ID>: <title>` heading is one item. The ID matches `^[A-Z][A-Z0-9]{0,7}$` (for example
   `M1`, `S10`) and is unique in the plan.
   A `## ` heading whose first word is prose (no colon, no digit, a lowercase letter after the
   first character, like `## Notes`) is ignored and ends the current item; any other `## `
   heading that is not `## <ID>: <title>` is an error.
3. Item fields are `key: value` lines directly after the heading:
   - `area:` (required) the area label, `^[a-z0-9-]+$`. One area has one orchestrator.
   - `depends:` (optional) comma-separated item IDs, or `seed`. Every ID must exist in the plan,
     and the graph has no cycles.
   - `budget_usd:` (optional) a positive number: the feature budget of the item in US dollars. The
     budget is read from this plan on the default branch (the `budget_usd` in the epic marker is
     display only, written when the epic is created). Without it, a mode whose `budget_feature` knob
     is not off uses tasks times `per_task_usd` (a cost-per-changed-line estimate comes later).
     Once only. To raise it, change the item in a PR to the default branch.
   - `shape:` (optional) `single`, `split` or `auto` (default). `single` makes the whole item
     one PR; `split` makes one small PR per task. See "Choosing the PR shape".
4. Text between the fields and `### Tasks` is the item's description; it goes into the epic body.
5. `### Tasks` lists the tasks, one `- <ID>: <title>` bullet each. The task ID matches
   `^T[0-9]{1,3}$` and is unique in its item. Indented `key: value` lines under the bullet:
   - `touch:` (required) comma-separated repo-relative globs (`*` inside a segment, `**` as a whole
     segment, across segments). They are the claim's touch paths. Only `*`, `?` and `**` are allowed: a glob with
     braces, character classes, escapes or `..` is a plan error.
     For the overlap and hotspot warnings below, a `Cargo.toml` also counts the `Cargo.lock` beside
     it and at the root, and a `package.json` counts `pnpm-lock.yaml` the same way. Claim, the
     edit hook and `bosun pr` will adopt this in later tasks.
   - `depends:` (optional) task IDs of the same item, or `<item>/<task>` for another item.
   - `size:` (optional) the planned changed lines. There is no cap (D-016); `size:` is only the
     plan the review compares the PR with: a PR over twice the planned lines of the tasks it
     closes gets a drift note in `bosun review-pack`. A `risk: high` task over 100 is a warning.
   - `kind:` (optional) `feature` (default), `bookkeeping`, `mainfix`, `process-fix`, `flake`.
   - `risk:` (optional) `high` marks a task that bosun expects to need an owner review.
   - `done-by:` (optional) `#N` (or `N`): PR N already did this task. `bosun plan import` and
     `plan amend` make a new task, or an open planned or ready one, merged (comment `Done by PR #N`,
     issue closed as completed) only when PR N is merged into the default branch and links the
     task: `Closes #<task issue>` in its body (an existing task), or the plan id (`M1/T2`) in its
     title or body. A refused field (not a PR, not merged, another base, not linked) leaves the
     task as it was, goes to `data.warnings` and makes the command exit 1 after the rest of the
     import is done; the fix is to merge the PR and name the task in it, or to remove `done-by:`.
     Any other value is a plan error.
   - `allow:` (optional) comma-separated test-integrity finding kinds the task may raise:
     `delete-test` (also allows `fewer-tests`), `skip`, `only`, `fewer-asserts`. Any other value
     is a plan error. Use it for a task that removes code with its tests, for example a seed
     crate. Import stores it in the task metadata only when the same task in the plan file on
     the default branch has that list with the same `touch:`, `depends:`, `kind:` and `risk:`
     (the owner approved the whole grant by merging): a local edit, a
     plan missing there or an unreadable one stores none, with a warning, and `plan amend` sets it
     once the plan merged. `bosun test-integrity --pr <n>` reads it from
     the PR's linked task, when that task's `touch:` covers every file the PR changes. A snapshot change is not an `allow:` kind: it needs a
     `snapshot-change: <reason>` line in the PR body.
   `bosun plan check` gives two warnings from the touch lists (not errors). Two tasks that touch
   overlapping files with no `depends` path between them (across items too; an item `depends:`
   counts for all its tasks) are not parallel: claim and wave keep apart tasks whose raw touch globs overlap
   (the companions apply only to this check for now), and the warning only tells the planner. Add a `depends:` or give the file to one task. A file that 3 or
   more tasks touch, that overlap each other and are not already in one order, is a hotspot: give
   it to one task and chain the others, or split the file. A pair of two merged tasks is not warned about (nor a hotspot counted for merged tasks). At most 20 overlap warnings are shown;
   the count stops at 1000 ("at least N more overlapping pairs").
6. The critical path of an item (the longest chain of `depends` inside it) has at most 4 tasks still
   open. When only this rule fails, `bosun plan check`, `import` and `amend` read the issues on GitHub
   and do not count tasks that are merged or closed as completed; a good plan is checked without GitHub.
   Other commands that read a plan (`intake`, `prereqs`, `hold`) still count every task, and a copy
   that is only compared (the plan on the default branch, at an intake's merge) skips the rule.
7. `### Done when` lists the UCs, one `- UC-<item>-<n>: <text>` bullet each, numbered from 1 with
   no gaps. Each line must be checkable by a test. Every item has at least one UC.
8. Any rule broken is a parse error with the file, the line and the fix; `bosun plan import` then
   changes nothing (exit 1).

## Choosing the PR shape

The shape follows the goal, not a line count (D-016): there is no cap on task or PR size.

A task is gated when it is `risk: high`, or when one of its `touch` globs (with the lockfile
companions) reaches a risk path (`.bosun/risk.toml`, else the built-in rules): a rule glob matches
the touch entry itself (any case), or the entry overlaps a rule glob that does not start with `**`
(`.github/**` reaches `.github/workflows/**`). A wide touch such as `src/**` is not gated by a
match-anywhere rule like `**/agents.md`; risk-check still judges each real file of the PR.
A gated task always gets its own single-task PR and is never bundled, and it is at most 400 changed lines (D-016): a gated task with `size:` over 400 is a plan error at its `size:` line (`plan check`, `import` and `amend` all refuse it; `import` and `amend` judge the risk paths by `.bosun/risk.toml` on the default branch, `plan check` by the working tree). `bosun plan check` lists
them as `gated`.

A wildcard touch (`*` or `?`, e.g. `packages/**`) is also resolved against the default branch's file tree: it is gated when a file it matches is matched by a risk path (`packages/a/hoist.toml` ~ `**/hoist.toml`). `import` and `amend` read the tree once; when it is unreadable or truncated, a wildcard touch that no risk path settles is a plan error at the task line (fix: narrow the touch to the paths the task edits, or mark it `risk: high`). `plan check` has no GitHub and keeps the advisory working-tree view of anchored paths only.

`shape: auto` (the default) decides on the other tasks, in this order:

1. At most one task is not gated: `single` ("one task", or "N gated tasks run alone").
2. `split` only when the tasks are independent: at least 2 run side by side (tasks divided by
   the longest `depends` chain), no two touch overlapping paths, and the item has more than one
   UC.
3. Otherwise `single`, with every reason that applies: shared files, a chain with no parallel
   gain, one UC (the goal is testable only whole).

`bosun plan check` prints `shape`, `shape_reasons` and `gated` for each item. An explicit
`shape: single` or `shape: split` is honoured; gated tasks are split out of a `single` item.

A single item is one wave entry: `bosun wave plan` returns the lowest ready issue with a
`bundle` of the others, and one PR closes every bundled task (`Closes #N` per issue).

## Cutting tasks for parallel work

These rules matter for split items; a single item is one PR.

PRs that run in parallel must not edit the same file, or the second one to merge has a merge
conflict to resolve. Cut the plan by file ownership:

- One task owns a file. A second task that must edit it gets a `depends:` on the first, so it
  starts from the merged result. `bosun plan check` warns when two tasks overlap without one.
- List every file a task edits in its `touch:`, including the spec section it updates
  (`docs/spec/NN-<section>.md`), module registries (`mod.rs`, `lib.rs`) and the test files it
  changes. The claim, wave and edit-hook checks only see what `touch:` names.
  `bosun plan check` warns when a task touches a crate's `src/` but none of that crate's `tests/`
  (`crates/x/src/..` needs `crates/x/tests/..`; at the repo root, `tests/..`).
- Put new tests in a new file (`tests/<command>_<topic>.rs`), not at the end of a shared one:
  two PRs that append to the same file conflict.
- Append-only lists live as one file per PR: CHANGELOG and PROGRESS entries are fragments in
  `changelog.d/` and `docs/progress.d/`. Never edit the folded files in a task.
- A dependency change is its own task (touch the manifest and its lockfile). On a lockfile
  conflict, take main's copy and regenerate it: `git checkout origin/main -- Cargo.lock` then
  `cargo update -w` (for pnpm: `git checkout origin/main -- pnpm-lock.yaml` then `pnpm install`).
  Never merge a lockfile by hand; the tools cannot read conflict markers.
- Versions change only in release PRs.
- Registries (`mod.rs`, `lib.rs`, a command list) are typical hotspots: let one early task add
  stub modules for the whole item and own the registry, so later tasks only touch their own
  files. Shared test helpers (`tests/common/**`) are another: extend them in that early task.
- A file that many tasks need is a hotspot: give it to one early task, or split it first (as
  `docs/spec.md` was split into one file per section).

## What import makes

| Plan part | GitHub | Labels |
| --------- | ------ | ------ |
| item | one epic issue, title `<ID>: <title>` | `bosun:kind/feature`, `bosun:area/<area>`, `bosun:source/plan`, `bosun:state/planned` |
| task | one issue linked to its epic, title `<item>/<task>: <title>`; the metadata carries `touch`, `depends`, `kind`, `risk` and `allow` | as the item, plus its `kind` |
| plan task | one issue per item, `<item>/P: plan`; `ready` at once, `kind: plan`, `allow: []`, touches the plan file | `bosun:kind/plan`, area, `bosun:source/plan`, `bosun:state/ready` |
| UC line | listed in the epic; the UC test lands passing with the task that implements it, no pending tests (spec §7); `bosun plan import` writes no test files; tasks become ready on the import run after the plan task merged | — |

When the `budget_feature` knob is not off, import (and `plan amend`, which backfills existing
epics) also comments one `{"bosun":"bind",...}` record on each task issue and on the epic, once:
they bind the task to its epic, item and task id, and the epic to its plan and item (see spec 05), and one `{"bosun":"epic-index",...}` on the registry issue naming the epic of the item.

Each issue body starts with a hidden marker `<!-- bosun:plan <item>/<task> -->`. Import finds
existing issues by that marker, so a second import of the same plan creates nothing new (UC-M2-1).
Import also adopts hand-made issues: a task issue whose `bosun:task` metadata names the same
item and task (or an epic whose `bosun:epic` marker names the item) counts as existing and is
never changed. `--dry-run` writes nothing and lists what would be created; `--item <ID>`
(repeatable) imports only those items.
An item removed by an amendment closes its epic as "not planned" (UC-M2-4).

## Amendments

New work enters only as an amendment (spec §4, intake): a kind/plan PR that edits the plan file.
When it merges, run `bosun plan amend <file>` (`--dry-run` shows the changes first): it imports like
`bosun plan import`, closes as "not planned" the epics, tasks and plan tasks the file no longer
lists (with the comment "Removed from <file> by an amendment."), and updates the title and
metadata (touch, depends, kind, risk) of changed tasks in place. It never touches the issues of
another plan file or hand-made issues. A dropped task that is active or holds a live lease is not
closed (it is listed under `skipped`, with a warning; its epic stays open); amend again after it
ends. An item that comes back reopens its epic and tasks. Plan paths are stored relative to the
repo root. Only after the plan task merged do new tasks become ready.

## Human prerequisites

Steps only a human can do (a secret, a repo setting, an account, a key) go in a
`## Prerequisites` section of the plan, with the tasks they block; `docs/prerequisites.md` has the
format, and spec §14 how bosun asks for them up front and holds the tasks they block.
