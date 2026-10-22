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
3. Item fields are `key: value` lines directly after the heading:
   - `area:` (required) the area label, `^[a-z0-9-]+$`. One area has one orchestrator.
   - `depends:` (optional) comma-separated item IDs, or `seed`. Every ID must exist in the plan,
     and the graph has no cycles.
4. Text between the fields and `### Tasks` is the item's description; it goes into the epic body.
5. `### Tasks` lists the tasks, one `- <ID>: <title>` bullet each. The task ID matches
   `^T[0-9]{1,3}$` and is unique in its item. Indented `key: value` lines under the bullet:
   - `touch:` (required) comma-separated repo-relative globs (`*` inside a segment, `**` across
     segments). They are the claim's touch paths.
   - `depends:` (optional) task IDs of the same item, or `<item>/<task>` for another item.
   - `size:` (optional) the planned changed lines. More than 400 is an error; a `risk: high` task
     over 100 is a warning.
   - `kind:` (optional) `feature` (default), `bookkeeping`, `mainfix`, `process-fix`, `flake`.
   - `risk:` (optional) `high` marks a task that bosun expects to need an owner review.
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
| UC line | one pending test per UC, written by the plan task (spec §7) | — |

Each issue body starts with a hidden marker `<!-- bosun:plan <item>/<task> -->`. Import finds
existing issues by that marker, so a second import of the same plan creates nothing new (UC-M2-1).
An item removed by an amendment closes its epic as "not planned" (UC-M2-4).

## Amendments

New work enters only as an amendment (spec §4, intake): a kind/plan PR that edits the plan file.
When it merges, `bosun plan import` runs again; only then do the new tasks become ready.
