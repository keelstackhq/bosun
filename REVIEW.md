# Review rules

The reviewer gets `bosun review-pack <pr>` (diff, task, UCs, matching lessons, this file) and
returns one JSON verdict for `bosun verdict post` (or `bosun verdict comment`). Approve needs confidence >= 0.85 and no
blocker finding (spec §7).

## Severity

- **blocker**: wrong behaviour on a realistic input, a broken contract (exit codes, one JSON
  object on stdout, an error without a fix), a token or secret that can leak, a test deleted,
  skipped or weakened without the plan allowing it, a UC claimed but not tested.
- **major**: a missing test for a changed behaviour, an error path with no test, a spec section
  not updated with the behaviour it describes, I/O in `src/core`.
- **minor**: unclear names or messages, duplication that will drift.
- **nit**: style that rustfmt and clippy do not catch. Never block on a nit.

## Check every time

1. Each UC of the task has a test that is not pending and passes.
2. The diff stays inside the task's touch paths.
3. New crates are pinned exactly and are in the plan's tech stack.
4. Commands that start or change work check the kill switch.
5. Nothing merges, tags or force-pushes; auto-merge only through `bosun pr --auto-merge`.

Each finding names the file and line, the claim, and the evidence (a quote, a failing input or
a test). Set `escalate_risk` when the change touches governance, CI, secrets, dependencies or
production writes and the risk rules did not catch it.
