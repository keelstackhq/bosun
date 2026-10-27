# Prerequisites: human steps, asked for up front

Some steps only a human can do (add a token, create an environment). bosun finds them when work
enters, so the owner does them once, from a phone if possible. After that, agents run to the end.

Status: being built (`docs/superpowers/plans/2026-10-27-front-loaded-prereqs.md`); the parser
lands first, then `bosun prereqs`, then the gating.

A plan lists them in a `## Prerequisites` section. The plan parser ignores this section.

```markdown
## Prerequisites
- H1: npm token for the release environment
  on: phone
  steps: npmjs.com > Access Tokens > Generate; GitHub > repo > Settings > Environments > release > Add secret NPM_TOKEN
  verify: secret release/NPM_TOKEN
  blocks: M4/T2, M5
```

Titles, steps and `who` cannot contain `<!--`, `-->` or control or bidi characters. A field
line (`on:`, `steps:` ...) that is not indented under a bullet is an error, and so is a heading
like `### Prerequisites` or `## Pre-requisites`: only `## Prerequisites` counts.

## Fields

- `H1: title`: the id (`H1` to `H999`, each used once) and a short title.
- `on:` (required): `phone` or `computer`. Where the owner can do it.
- `steps:` (required): one line the owner can follow.
- `blocks:` (required): what waits for it. A task `M4/T2` or a whole item `M5`.
- `who:` (default `owner`): who does it, a plain name (letters, digits, `.`, `_`, `-`, `@`).
- `replaces:` (optional): ids of `X` items (added by `bosun hold`) that this item takes over.
- `verify:` (default `owner` when the line is absent; an empty `verify:` is an error): how bosun
  checks it is done.

## verify

| Kind | What bosun reads |
| --- | --- |
| `owner` | Nothing. The owner ticks the box. |
| `secret NAME` | The Actions secret list of the repo. |
| `secret ENV/NAME`, `variable ENV/NAME` | The secrets or variables of that environment. |
| `variable NAME` | The Actions variables of the repo. |
| `environment NAME` | The environments of the repo. |
| `file PATH` | The file on the default branch. |

bosun writes what it found into the issue (see below). If the API says the item is missing, a
tick alone does not finish it. If GitHub answers 403 (the token cannot read it), bosun treats the
item like `owner`: its marker says `verify=owner` and the tick decides. A `file` path is relative to the repo (no `/`, `~` or
`..`). Secret and variable names cannot start with `GITHUB_`.

## blocks

Tasks that a prerequisite blocks stay `planned` until it is done; when it blocks a whole item,
the item's epic stays `planned` too. Other tasks go on. `plan import` and `amend` refresh the
checklist issue first, and the next import after the item is done makes the tasks ready.
`bosun claim`
refuses a blocked task that is already ready, `bosun wave plan` leaves it out, and `bosun spawn`
refuses an epic whose whole item is blocked. When the plan has a `## Prerequisites` section but
its checklist issue cannot be read, they refuse too (fix: `bosun prereqs <plan> --post`).

An agent that finds a human step nobody listed runs `bosun hold <task issue> "<step>"`. bosun
adds it to the checklist (id `X1`, `X2`, ..., verified by the tick), releases the task, and the
task waits for it. The agent takes other work. Rewrites of the checklist keep X items until a
plan item lists them in `replaces:`.

## Automatic items

bosun adds these without being asked:

- Approvals at merge, judged as risk-check would. A task needs the owner when it touches an
  owner rule, a delegate rule on the critical paths, or both a delegate and a two-verdicts rule,
  or when it says `risk: high`. A task that hits only delegate rules needs a delegate. Tasks that
  hit only two-verdicts rules, or no rule, are not listed. They are grouped
  in batches by dependency level, so the owner answers once per batch. This is an estimate from
  paths; risk-check still decides at merge.
- Setup steps (ids `D1`, `D2`, ... after the `bosun doctor` step). Each `missing` step of
  `bosun doctor`.
- A delegate key (id `K1`). If some task needs a delegate and none is enrolled.

## The command and the issue

`bosun prereqs <plan>... [--post] [--no-setup]` prints the checklist (`--no-setup` leaves out
the doctor steps): the items (open computer items first, then open
phone items, then done), the approvals, and a short message for the owner's chat. With `--post`
it opens or updates one GitHub issue per plan, labelled `bosun:prereqs`. The owner ticks boxes in
the GitHub app. bosun reads the ticks back, marks what it verified itself, and closes the issue
when nothing is open.

Each line ends with a hidden marker that bosun writes, for example
`<!-- bosun:prereq H1 blocks=M4/T2,M5 verify=api ok -->`. An item is done when its marker says
`ok` (bosun read it through the API), or when it is `verify=owner` and its box is ticked. The
gates read only this issue, open or closed. A tick is not authenticated: anyone with write access
can tick a box or edit a marker. Agents cannot: bosun's hook refuses an agent's GitHub write that
contains `bosun:prereq`.
