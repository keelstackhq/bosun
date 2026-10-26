# CI minutes

GitHub Actions bills private repos per job, and rounds each job up to a whole minute. Windows
counts x2 and macOS x10. Standard runners on public repos are free. A job that is skipped
(its job-level `if:` is false) never starts a runner and costs nothing. The figures below are
"billable if private" estimates: ceil(job minutes) x OS multiplier, summed over the jobs that
start.

## Measured: keelstackhq/bosun, 2026-10-19 to 2026-10-26

These figures come from 2,150 runs, read with the GitHub Actions API. Job durations come from
31 sampled runs. Total: about 5,200 min in 7 days.

| workflow | event | runs | jobs that start / run | min / run | min (7 d) |
| --- | --- | ---: | ---: | ---: | ---: |
| ci | pull_request | 453 | 9 | ~6.7 | ~3,050 |
| ci | push | 159 | 9 | ~5.6 | ~900 |
| risk | pull_request_target | 308 | 1 | 1 | ~300 |
| main-open | pull_request(_target) | 332 | 1 | 1 | ~330 |
| verdict | issue_comment | 294 | 0.7 | 0.7 | ~200 |
| release | push | 93 | 2 | ~1.9 | ~175 |
| main-health | workflow_run | 116 | 1 | 1 | ~115 |
| release | workflow_dispatch | 1 | 8 (2 macOS, 1 Windows) | 58 | 58 |
| risk | issue_comment | 298 | 0.1 | 0.1 | ~35 |
| review | pull_request_target | 55 | 0 (skipped) | 0 | 0 |

- `ci` is 76 % of the total. A `ci` run has 9 jobs that together take about 4 minutes of real
  time, but each job is billed as at least 1 minute. gate-test takes 54-80 s, gate-lint 21-43 s,
  and fmt, deps and test-integrity 17-25 s each. bosun-bin, pin, docs and ci-ok take 2-32 s each.
  So most of a run's bill is rounding.
- Comment-triggered workflows already skip at job level. 264 of 298 risk runs and 91 of 294
  verdict runs started no job.
- 81 of 93 release push runs failed in `prepare`. Each failed run still billed about 2 min.

## The other repos (from their workflow files, cost per run if private)

These are public today, so they cost nothing. The orchestrator has run data only for bosun.

| repo | biggest consumer | per PR push if private |
| --- | --- | --- |
| rust-canary (private) | the channel's bosun.yml: 9 jobs, plus risk and main-open | ~12 min |
| berth | ci matrix ubuntu + macOS + Windows on every PR | ~50 min |
| hoard | self-test: 4 OS x 2 binaries x 2 jobs on PRs that touch the action | ~100 min |
| splice | ci on ubuntu + Windows on every PR | ~10 min |
| hull, hoist, keelstack, example-app, cli-canary, decisions | one ci job + ci-ok | ~2-4 min |

## What the bosun channel does about it

- **One job.** bosun.yml runs the docs-only check, the pinned bosun, the pin check and every
  gate in one job, named after the required check (`ci-ok`, or `bosun-ok`). The gates that run
  no PR code go first. Before, this took 9 jobs (~10-12 min billed). Now it takes 1 job (~3-4
  min billed). A docs-only PR takes 1 min instead of 2.
- **Edits run test-integrity only.** A PR body or title edit changes no file. The edit run
  waits for the running run. If the head passed, it keeps the file gates and runs only
  test-integrity, which reads the body, with no toolchain setup (~1 min). Otherwise it runs
  every gate.
- **Drafts fail fast.** A draft PR fails the check in seconds and runs no gate. A skipped
  required check would count as passed. `ready_for_review` runs the gates.
- **Label events skip.** main-open judges again only when the label is `bosun:kind/mainfix`.
- **Weekly, not nightly.** main-health runs the gates weekly. Every push to main already runs
  them on all of main. The trade-off: a new advisory against an unchanged lockfile (cargo deny)
  is found up to 7 days later, or at the next push to main.
- **Local first.** AGENTS.md: run every gate locally before a push, and push once per PR.

Rules for every repo: run the local checks before every push. Skip non-command events at job
level. Use Windows and macOS only for releases.
