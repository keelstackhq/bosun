## 18. Modes

**Status: parse, policy and weaken check (plan PRs 1a, 1b); the boosters land later.** Design:
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
gatekeeper has no target). Nothing is stored. The counts come from GitHub in plan PR 5; until
then `bosun mode` prints `brake: null`. Its sources are writer-only or immutable: main-health
commit statuses, merged revert PRs by a writer with the `bosun:revert N` marker, and check
conclusions. Never an issue an agent can close: closing a main-red issue cannot lift it.

### Revert (plan PR 3b)

`revert-auto` and the `R-revert` rule act only on a PR with the `bosun:revert N` marker. A
revert without it gets the normal rules and is never `revert-auto`; a wrong N fails the
exact-inverse check.

### Commands

- `bosun mode`: the effective mode and knobs with their sources; no GitHub call.
- `bosun heal [--pr N]` (PR 3a/3b), `bosun digest [--answer --from F --sign-key PEM]` (PR 4),
  `bosun metrics [--days N] [--propose]` (PR 5/8), `bosun wave tick <epic>` (PR 6): registered
  with their final arguments; until their PR lands each exits 1 with a `fix` naming it.
