# Modes: from gatekeeper to booster (design)

Date: 2026-11-11 · Status: proposed (owner reviews) · Plan: `docs/superpowers/plans/2026-11-11-modes-booster.md`

Owner goal: bosun is an opinionated booster for Claude Code agencies, not only a gatekeeper.
Speed, quality, no conflicts, budget and learning all count. The stance is a mode ladder set in
`.bosun/bosun.toml`, from `gatekeeper` (today) to `autonomous` (almost no human touches).

Owner pain on two repos: PRs too small, CI red too often, too many owner approvals.

## Why it hurts today (findings)

1. **Small PRs.** D-016 already drops the size cap for non-gated tasks and prefers one PR per
   item (`shape: auto`). But every new module registers in `src/core/mod.rs`,
   `src/cli/mod.rs` (arg structs) and `src/lib.rs` (dispatch), all R-gate paths. So almost every feature touches a gated file:
   it is split out, capped at 400 lines, and needs a delegate or owner. The cap is not the cause;
   the registration files are.
2. **Red CI.** Gates run locally only before `bosun pr --ready` (stamps). Agents push earlier,
   CI runs on a red tree, and nobody fixes a red PR until a human looks.
3. **Approvals.** After D-012 bootstrap ends (2026-11-30), `owner-gated` sends every gated PR
   and every plan PR (`**/plans/**` is R-gov) to the owner, one chat form per PR.

## Decisions

- **Four modes:** `gatekeeper`, `steady`, `booster`, `autonomous`. Five gave no knob a distinct
  value; three merged "same rules, more help" with "fewer owner gates".
- **A mode is a preset.** `[mode] name = "<mode>"` fills knobs the repo leaves unset. Explicit
  keys win (`[approvals] policy`, `[mode]` overrides). Resolution: explicit > preset > built-in
  default. Limits and money (`.bosun/limits.toml`) are never part of a preset: the owner sets
  them on purpose.
- **Default stays `gatekeeper`.** No `[mode]` table = `gatekeeper` = today, byte for byte.
- **A lighter mode is R-weaken.** Any move down the ladder, and any override that loosens a knob,
  is a weaken finding (`.bosun/bosun.toml: mode gatekeeper → booster`), owner-only in every mode.
  A move up (tighter) is no finding (still R-gov by path, so still the owner's file).
- **Never in any mode:** R-weaken, R-gov, R-secrets, R-prod-write, workflow files
  (`WORKFLOW_PATHS`), release environments, tags and secrets stay with the owner. Agents never
  merge, tag or force-push (D-008, D13). bosun never calls remote tools (D29): every spawn,
  archive, message or notification below is a call in `next`.
- **No new approval mechanism.** The fast lane is the D-017 `autonomous` policy (allowlist +
  one clean independent verdict + orchestrator signature). Modes never use `orchestrator`
  policy (it needs no verdict). `policy` is never a `[mode]` override (parse error): only the
  `[approvals]` key or the mode name sets it, and both are R-weaken findings when they loosen.
- **R-gov joins the D-017 deny set.** Today `AUTONOMOUS_DENIED_RULES` is R-secrets,
  R-prod-write, R-gate, R-weaken; R-gov files inside the allowlist (`src/core/session_rules.rs`,
  a `docs/**/CLAUDE.md`) could pass an orchestrator signature. Plan PR 1b adds R-gov, so the
  "owner in every mode" line below holds.
- **Code that acts without the owner is gate code.** `src/core/mode.rs` (effective policy),
  `src/{core,cli}/digest.rs` (owner answers), `src/{core,cli}/heal.rs` (fixers, revert
  auto-merge), `src/{core,cli}/metrics.rs` (brake, curator targets) and the spawn decisions
  (`src/core/spawn.rs`, `src/core/wave.rs`, `src/cli/wave.rs`) join R-gate in both rule files
  (`.bosun/risk.toml`, `src/core/risk_default.toml`).
- **No half release.** Until the weaken check lands (plan PR 1b), `[mode] name` other than
  `gatekeeper` is a parse error.
- **In bosun itself** all of `src/**` is critical (`APPROVAL_CODE_PATHS`): every gated src PR
  needs the owner whatever the mode, and D-017 adds little. The fast lane pays off in consumer
  repos (the two repos with the pain); in bosun the gain is helpers and fewer gated PRs.

## The ladder

| Mode | One line |
| ---- | -------- |
| `gatekeeper` | Today. Every knob as configured; helpers off. |
| `steady` | Same approval rules; bosun helps: gates before push, red-PR notices, revert PRs, one daily digest, feature budget warnings. |
| `booster` | Fast lane: D-017 autonomous approval inside the allowlist; bosun spawns fixers and children; budget asks at 100 %; curator proposes loosenings. |
| `autonomous` | Booster plus automatic revert merge on red main and digest-only owner contact; target one owner touch a day. |

## Mode x knob

| Knob | gatekeeper | steady | booster | autonomous |
| ---- | ---------- | ------ | ------- | ---------- |
| `[approvals] policy` (exists) | repo value, default `owner` | repo value | `autonomous` (D-017) | `autonomous` |
| `[review] mode` (exists) | repo value | repo value | repo value | repo value |
| PR shape (D-016, exists) | `auto` | `auto` | `auto` | `auto` |
| `push_gates` (new) | `ready` | `push` | `push` | `push` |
| `heal_pr` (new) | `off` | `notify` | `spawn` | `spawn` |
| `heal_main` (new) | `off` | `revert-pr` | `revert-pr` | `revert-auto` |
| `approvals_batch` (new) | `per-pr` | `daily` | `daily` | `daily` |
| `spawn_auto` (new) | `off` | `off` | `on` | `on` |
| `budget_feature` (new) | `off` | `warn` | `ask` | `ask` |
| `brake` (new) | `off` | `on` | `on` | `on` |
| `curator_loosen` (new) | `off` | `off` | `propose` | `propose` |
| `.bosun/limits.toml` | file | file | file | file |

Weaken check (`src/core/weaken.rs`): the mode name moving down the ladder is a finding; so is
an override that loosens what passes: `push_gates` push → ready, `brake` on → off, or a policy
looser than the one before (as today, any `[approvals]` change already is one).
`Config::parse` resolves presets and fills `approvals.policy`, so risk-check and `approve`
(which call `Approvals::policy()`) see the effective policy with no change of their own. The helper
knobs (`heal_*`, `spawn_auto`, `approvals_batch`, `budget_feature`, `curator_loosen`) change
who acts, not what passes; changing them alone is no finding (the file is R-gov anyway).

Syntax:

```toml
[mode]
name = "booster"
# optional overrides: the new knobs of the table only (never policy)
heal_main = "off"
```

Release ordering as for every new key: set `[mode]` only after `.bosun/version` pins a release
that reads it (the pinned release refuses unknown keys). `bosun mode` prints the effective knobs,
which source set each (explicit, preset, default) and the brake state.

## The boosters (ideas weighed)

1. **Two lanes — kept, mapped.** Fast lane = low-risk PRs (one verdict, auto-merge, as today)
   plus gated `src/**`, `tests/**`, `docs/**` PRs under D-017 `autonomous`. Heavy lane = the
   "never" list above, plus gate code (R-gate rule, module roots) as D-017 already denies.
   No new lane machinery.
2. **Feature-sized tasks — mostly exists, change the cause.** D-016 already sizes PRs by goal
   and caps only gated tasks at 400. The real blocker is that module registration is R-gate
   (finding 1). Plan PR 1a registers every new module of this plan up front, with its final
   arg structs, so later PRs touch only new, non-gate files. Moving registration out of R-gate for good is an open point
   (it is itself R-weaken). Split only when leases collide stays the `shape: auto` rule.
3. **CI self-heal — kept.**
   - `push_gates = "push"`: the PreToolUse hook refuses `git push` when the tree being pushed
     lacks a passing stamp for every CI gate of the profile (`fix`: the gate loop of AGENTS.md
     step 4). Same stamps `pr --ready` reads today.
   - `heal_pr`: `bosun heal` (janitor, hourly) finds open bosun PRs whose head has a failing
     `ci-ok`. `notify` returns a `send_message` to the lease holder with the failing gate and
     log signature (`bosun ci-log`). `spawn` also returns a `create_session` fixer when the
     holder is dead (lease expired), within spawn checks. At most 2 heal rounds per PR; the
     existing stop rules (same signature 3x, empty fix round) still block; then it goes to the
     digest as needs-human.
   - `heal_main`: on an open main-red issue `bosun heal` names the first bad PR
     (`main-triage`, or the only PR merged since green) and opens its revert (the `bosun revert`
     code). `revert-pr` leaves it a draft mainfix; `revert-auto` marks it ready with auto-merge
     only when the revert diff is checked to be the exact inverse of the merged PR, a clean
     verdict is in, and the reverted PR needed no owner. Rule (risk-check, built-in `R-revert`,
     owner): reverting a PR that had an owner-class match (R-weaken, R-gate, R-gov, ...) needs
     the owner again, and so does a revert that is not the exact inverse. Weaken heuristics
     are only a second net.
   - Verdict carry over merge-only heads: exists (`carry_verdict`, D-017 C7). No change.
4. **Approvals only where they matter, batched — kept.** `approvals_batch = "daily"`:
   `bosun digest` (orchestrator, once a day at a fixed UTC hour) lists every owner-waiting PR
   (repo, PR, head SHA, rule, one-line why), open `bosun ask` questions and budget asks, as one
   D-012 chat form ("approve 12 15", "raise 7 +20"). The orchestrator posts the signed approvals
   for the exact heads; a head that moved since the digest drops out and is listed the next
   day. Mainfix PRs and release-environment taps never wait for the digest. This is D-009's
   "batched chat approvals" made mechanical. The digest signs nothing: approvals come back as
   `next` calls `bosun approve <pr>@<head>` for the orchestrator (the key holder).
5. **Spawn and archive — kept, via `next`.** `spawn_auto = "on"`: `bosun wave tick <epic>`
   returns `create_session` calls for ready wave entries while every spawn check passes
   (limits, budget < 80 %, no lease conflict, brake off) and `archive_session` for children whose
   result is recorded and whose PR merged or closed. Environment from `[cloud]`.
6. **Budget per feature — kept.** A plan item may carry `budget_usd:`; `bosun plan import`
   fills an estimate when missing: planned `size:` × cost per changed line over the last 30
   days (budget report), else `per_task_usd` per task (`plan check` has no GitHub access and
   uses only the fallback). Spend per epic = its tasks' spend
   records. `warn`: at 80 % a warning in `status`, `claim` and the digest. `ask`: also at 100 %
   `claim` and `spawn` exit 5 until the owner answers "raise <epic> +N": counted only with a
   verified enrolled-owner signature, as `answer --chat` records are (not a file change). The daily guard (80 %/100 %) is unchanged in every mode.
7. **Metrics and curator — kept, narrowed.** `bosun metrics [--days N]` (read-only): feature
   lead time (first claim → last merge of an epic), red-CI rate (failing `ci-ok` heads / pushed
   PR heads), cost per merged PR, owner touches per day (owner approvals and answers), and
   blocks per rule: each owner-class risk-check requirement with its outcome (`caught` = a new
   commit or a close before approval, `passed` = approved unchanged). `--propose` lists rules
   with at least 10 blocks and no catch in 30 days. `curator_loosen = "propose"`: the curator
   Routine opens at most one draft PR a week that loosens one such rule, evidence in the body.
   It is R-weaken, so the owner decides. Never proposed: R-weaken, R-secrets, R-prod-write,
   R-gov, workflow paths.

**Brake.** `brake = "on"`: the brake is recomputed on every read (`bosun heal`, `wave tick`,
`digest`, `mode`) from GitHub data; nothing is stored, so no agent can lift it. It is on when,
over 7 days, main went red twice, two PRs were reverted, or red-CI rate is 10 points over the
mode's target. While on, the repo acts one step stricter for helpers only: `spawn_auto` off,
`heal_pr` at most `notify`, and the digest leads with "brake on". Approval policy is not
touched by the brake (risk-check never reads metrics). It ends by itself when the 7-day window
is clean again; that returns to the owner-approved file, so no approval is needed.

## Target numbers per mode

Estimates; `bosun metrics` measures the gatekeeper baseline first (plan PR 5), then the owner
moves a repo one step at a time.

| Metric | gatekeeper | steady | booster | autonomous |
| ------ | ---------- | ------ | ------- | ---------- |
| Typical feature lead time (p50) | baseline | ≤ 3 days | ≤ 1.5 days | ≤ 1 day |
| Red-CI rate | baseline | < 30 % | < 20 % | < 15 % |
| Owner touches per day | as needed | ≤ 5 | ≤ 3 | ≤ 1 (the digest) |
| Cost per merged PR | tracked | tracked | ≤ feature estimate | ≤ feature estimate |

A "touch" is one owner answer: one digest reply counts once however many PRs it approves.

## Safety argument

- Nothing changes for a repo without `[mode]`: every new knob defaults to its gatekeeper value
  and the new commands are no-ops there (`bosun heal` returns empty `next`).
- Only the owner lowers the mode (R-weaken + R-gov) and only the owner loosens a rule (curator
  proposals are R-weaken). No mode widens the D-017 allowlist or touches the owner floor.
- Self-heal acts through PRs with full checks; a revert is judged by risk-check like any PR.
- Helpers that spend money (spawn, heal) obey the daily guard, per-task and per-feature budget,
  and the brake.

## Open points (decided here, owner may override)

1. **Plans are R-gov.** Every feature plan PR needs the owner. Kept; the digest batches them.
   Loosening (e.g. plan files that only add a new plan) would be R-weaken: owner's call.
2. **Module registration is R-gate.** Plan PR 1a works around it once. A lasting fix (a non-gate
   registry file for non-gate commands) changes the gate boundary: owner's call, listed as
   optional PR O1.
3. **Revert verdict.** `revert-auto` needs the exact-inverse check and a reviewer verdict. The
   verdict could be dropped later for exact inverses of PRs that needed no owner.
4. **Thresholds** (brake, propose at 10 blocks / 0 catches, 2 heal rounds, targets) are
   estimates; they go to `docs/defaults.md` and get reviewed after 30 days of metrics.
5. **Org decision.** The ladder should become a keelstackhq decision (all repos) once one repo
   ran `booster` for two weeks; until then it is bosun-local.
6. **booster vs autonomous** differ only in helpers, not approvals: one approval floor for both
   keeps the risk story simple. A wider autonomous allowlist (root files, `.changeset/**`) was
   cut.
7. **Curator Routine prompt** is owner-held (Routines are human-only): enabling
   `curator_loosen` needs an owner edit, listed in Blocked.
