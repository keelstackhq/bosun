## 18. Modes

**Status: parse, policy and weaken check (plan PRs 1a, 1b, 5); the boosters land later.** Design:
`docs/superpowers/specs/2026-11-11-modes-booster-design.md`, plan `…/plans/2026-11-11-modes-booster.md`.

### `[mode]`

```toml
[mode]
name = "gatekeeper"   # gatekeeper | steady | booster | autonomous
# optional overrides, the knobs below only
push_gates = "push"
```

- No `[mode]` table = `gatekeeper` = the behaviour before modes, byte for byte.
- A mode is a preset (never `.bosun/limits.toml`): it fills knobs the repo leaves unset.
  Explicit override > preset of `name` > default (gatekeeper value); `bosun mode` names the
  source of each knob (`explicit`, `preset`, `default`).
- Unknown mode names, unknown knobs and unknown values are parse errors. `policy` is never a
  `[mode]` key (parse error): only `[approvals] policy` or the mode name sets it.
- The preset fills `[approvals] policy` when the table leaves it unset (booster and
  autonomous: `autonomous`, D-017); risk-check and `approve` print `data.policy_source`.
- Weaken (§8, R-weaken): a move down the ladder, and an effective `push_gates` push → ready,
  `brake` on → off or `budget_feature` lowered (ask > warn > off: it removes a spend stop).
  Helper knobs (`heal_*`, `spawn_auto`, `approvals_batch`, `curator_loosen`) change who
  acts, not what passes: changing them alone is
  no finding; the file is R-gov, so the owner still approves. A move up is no finding.
- Code that acts without the owner (`core/mode`, `digest`, `heal`, `metrics`, `spawn`, `wave`,
  core and cli) is R-gate; R-gov is in the D-017 deny set.
- Release ordering as for every new key: set `[mode]` only once `.bosun/version` pins a release
  that reads it.

### Knobs

| Knob | gatekeeper | steady | booster | autonomous |
| ---- | ---------- | ------ | ------- | ---------- |
| `push_gates` | `ready` | `push` | `push` | `push` |
| `heal_pr` | `off` | `notify` | `spawn` | `spawn` |
| `heal_main` | `off` | `revert-pr` | `revert-pr` | `revert-auto` |
| `approvals_batch` | `per-pr` | `daily` | `daily` | `daily` |
| `spawn_auto` | `off` | `off` | `on` | `on` |
| `budget_feature` | `off` | `warn` | `ask` | `ask` |
| `brake` | `off` | `on` | `on` | `on` |
| `curator_loosen` | `off` | `off` | `propose` | `propose` |

### Brake

Pure (`core::mode::brake`): over a 7-day window it is on when main went red at least twice, at
least two PRs were reverted, or the red-CI rate (failing `ci-ok` heads / pushed PR heads) is
at least 10 points over the mode's target (steady 30 %, booster 20 %, autonomous 15 %;
gatekeeper has no target). Nothing is stored. `bosun metrics` computes it on every run
(`data.brake`, in every mode; `knob` says whether the mode applies it); `bosun mode` stays
offline and prints `brake: null`; heal, digest and wave tick read it through
`cli::metrics::window`. Sources: check runs of the GitHub Actions app, on merge commits of
main (what main-health judges) and on the commits of own-branch PRs. A commit with a green or
red `ci-ok` is a pushed head. A `ci-ok` that failed while no other job of that app failed is
excused only on a PR commit a newer push superseded (its run was cancelled) and on the head of
a draft: elsewhere it is red, also when a hand-cancelled run made it fail, and always on main.
Every run of the commit counts, so a green rerun cannot erase a failure; commit statuses and
other apps never count. A PR that changes `.github/**` never counts for the rate: its own
`on: pull_request` workflows post its `ci-ok`. A revert is a merged PR by a
writer from a `bosun/revert-N` branch (the name cannot change) or with the `bosun:revert N`
marker. Main-red issues are not read: closing one cannot lift the brake. Main red and reverts
are the authoritative trips. The rate trip is advisory: PR heads are PR-controlled, and known
gaps lower it (never main red or reverts): a rebase of a PR branch drops its old commits
from the list, a hand-cancel followed by a new push is excused, and a workflow on a PR branch
can post same-app `ci-ok` runs that dilute the rate.

### Revert (plan PR 3b)

`revert-auto` and the `R-revert` rule act only on a PR with the `bosun:revert N` marker. A
revert without it gets the normal rules and is never `revert-auto`; a wrong N fails the
exact-inverse check.

### Commands

- `bosun mode`: the effective mode and knobs with their sources; no GitHub call.
- `bosun metrics [--days N]` (PR 5, read-only): over `--days`: red-CI rate, cost per merged PR
  (a task's spend once), lead time p50 per epic, owner touches (an owner's own approve or
  answer, or a plain or after-review signature of an owner-role key) and owner-rule blocks;
  `brake` over 7 days. `--propose` lands in PR 8.
- `bosun heal [--pr N]` (PR 3a/3b), `bosun digest [--answer --from F --sign-key PEM]` (PR 4),
  `bosun wave tick <epic>` (PR 6): registered with their final arguments; until their PR lands
  each exits 1 with a `fix` naming it.
