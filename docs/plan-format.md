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
4. Text between the fields and `### Tasks` is the item's description; it goes into the epic body.
5. `### Tasks` lists the tasks, one `- <ID>: <title>` bullet each. The task ID matches
   `^T[0-9]{1,3}$` and is unique in its item. Indented `key: value` lines under the bullet:
   - `touch:` (required) comma-separated repo-relative globs (`*` inside a segment, `**` across
     segments). They are the claim's touch paths.
     For the overlap and hotspot warnings below, a `Cargo.toml` also counts the `Cargo.lock` beside
     it and at the root, and a `package.json` counts `pnpm-lock.yaml` the same way. Claim, the
     edit hook and `bosun pr` will adopt this in later tasks.
   - `depends:` (optional) task IDs of the same item, or `<item>/<task>` for another item.
   - `size:` (optional) the planned changed lines. More than 400 is an error; a `risk: high` task
     over 100 is a warning.
   - `kind:` (optional) `feature` (default), `bookkeeping`, `mainfix`, `process-fix`, `flake`.
   - `risk:` (optional) `high` marks a task that bosun expects to need an owner review.
   `bosun plan check` gives two warnings from the touch lists (not errors). Two tasks that touch
   overlapping files with no `depends` path between them (across items too; an item `depends:`
   counts for all its tasks) are not parallel: claim and wave keep apart tasks whose raw touch globs overlap
   (the companions apply only to this check for now), and the warning only tells the planner. Add a `depends:` or give the file to one task. A file that 3 or
   more tasks touch, that overlap each other and are not already in one order, is a hotspot: give
   it to one task and chain the others, or split the file. At most 20 overlap warnings are shown.
6. The critical path of an item (the longest chain of `depends` inside it) has at most 4 tasks.
7. `### Done when` lists the UCs, one `- UC-<item>-<n>: <text>` bullet each, numbered from 1 with
   no gaps. Each line must be checkable by a test. Every item has at least one UC.
8. Any rule broken is a parse error with the file, the line and the fix; `bosun plan import` then
   changes nothing (exit 1).

## What import makes

| Plan part | GitHub | Labels |
| --------- | ------ | ------ |
| item | one epic issue, title `<ID>: <title>` | `bosun:kind/feature`, `bosun:area/<area>`, `bosun:source/plan`, `bosun:state/planned` |
| task | one issue linked to its epic, title `<item>/<task>: <title>` | as the item, plus its `kind` |
| plan task | one issue per item, `<item>/P: plan`; `ready` at once, `kind: plan`, `allow: []`, touches the plan file | `bosun:kind/plan`, area, `bosun:source/plan`, `bosun:state/ready` |
| UC line | listed in the epic; the UC test lands passing with the task that implements it, no pending tests (spec §7); `bosun plan import` writes no test files; tasks become ready on the import run after the plan task merged | — |

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
