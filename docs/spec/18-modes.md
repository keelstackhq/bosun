## 18. Modes

**Status: parse and registrations only (plan PR 1a); the boosters land later.** Design:
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
- **Not active yet:** until the weaken check lands (plan PR 1b), any `name` but `gatekeeper` is a
  parse error, so no release carries a lighter mode without it.
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
then `bosun mode` prints `brake: null`.

### Commands

- `bosun mode`: the effective mode and knobs with their sources; no GitHub call.
- `bosun heal [--pr N]` (PR 3a/3b), `bosun digest [--answer --from F --sign-key PEM]` (PR 4),
  `bosun metrics [--days N] [--propose]` (PR 5/8), `bosun wave tick <epic>` (PR 6): registered
  with their final arguments; until their PR lands each exits 1 with a `fix` naming it.
