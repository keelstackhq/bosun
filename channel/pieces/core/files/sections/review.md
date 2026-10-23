## bosun: review rules

The reviewer gets `bosun review-pack <pr>` (diff, task, UCs, matching lessons, this file) and
returns one JSON verdict for `bosun verdict post` (or `bosun verdict comment`). Approve needs
confidence >= 0.85 and no blocker finding (bosun spec §7).

### Severity

- **blocker**: wrong behaviour on a realistic input, a broken contract (an API, a CLI, a file
  format), a token or secret that can leak, a test deleted, skipped or weakened without the
  plan allowing it, a UC claimed but not tested.
- **major**: a missing test for a changed behaviour, an error path with no test, docs not
  updated with the behaviour they describe.
- **minor**: unclear names or messages, duplication that will drift.
- **nit**: style that the formatter and the linter do not catch. Never block on a nit.

### Check every time

1. Each UC of the task has a test that is not pending and passes.
2. The diff stays inside the task's touch paths.
3. New dependencies are pinned exactly and are in the plan's tech stack.
4. Nothing merges, tags or force-pushes; auto-merge only through `bosun pr --auto-merge`.

Each finding names the file and line, the claim, and the evidence (a quote, a failing input or
a test). Set `escalate_risk` when the change touches governance, CI, secrets, dependencies or
production writes and the risk rules did not catch it.
