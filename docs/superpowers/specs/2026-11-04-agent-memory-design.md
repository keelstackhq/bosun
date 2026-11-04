# Agent memory (progress + learnings): design

Date: 2026-11-04. Owner ask: a mechanism for agents only to track progress and learnings,
minimal, agent-readable, fewest tokens. Decisions are the orchestrator child's; hull not read
(access denied), hoist and berth read at HEAD.

## 1. What exists and where it falls short

| Piece | Where | Gap |
| --- | --- | --- |
| Lessons store | spec §6, `docs/lessons.toml`, `bosun lesson add/hit/capture/triage` | TOML ~300 B/entry; file never created in any repo; only reader is `review-pack`; nothing loads it at claim or session start |
| Lesson issues | `bosun:lesson`, FNV signature dedup, curator daily | good dedup and prune (caps 40/8, 30-day retire); only the advice tail reaches agents |
| Gotchas | CLAUDE.md in bosun (2), hoist (3), berth (6) | always loaded, no scope, no evidence, never pruned; TMPDIR line copied in all 3 repos |
| PROGRESS.md | bosun 85 KB (~21k tokens, 285 Done lines), hoist 32 KB, berth 18 KB | Done is human history; Status is prose; fragments fix conflicts, not tokens |
| Handoff | `bosun handoff`, `resume --handoff` | terse `<kind>: <text>` lines, cap 40: right shape, but roots only |
| SessionStart hook | `src/hooks/session_start.rs` | prints leases, kill switch, main-red; no lessons, no next steps |
| Cross-repo | `lesson add --upstream` | only lessons about bosun; no org-shared gotchas |

## 2. Design (extend; no new store, no new command)

Tokens are spent on what is loaded, not on what is stored, so the store stays TOML and the
outputs get terse.
1. Compact render: every output that hands lessons to an agent (`claim`, the hook,
   `review-pack`) prints one line each, `L-012 src/gates/** Run cargo metadata once per gate run.`
   (id, scope or `*`, rule; `why`/evidence stay in the file). Rule <= 160 chars, `add` refuses
   longer (exit 2). Path stays `docs/lessons.toml`: no risk, channel or review-pack change.
2. Proven only: manual `lesson add` needs `--evidence #N` (the PR or issue that proved it);
   triage advice already passes its issue. A rule whose normalised text equals an entry's
   records a hit instead (exit 0, `data.dup: "L-NNN"`).
3. Relevant only: `claim` adds `data.lessons` = lines whose scope matches the task's touch, plus
   globals, read from the local file (advice, not trust input).
4. Session start: the hook reads `docs/lessons.toml` locally before any GitHub call and appends
   the global lines (<= 8, ~200 tokens) to both the normal and the error text. Handoff stays
   with `resume --handoff` (roots only; the lease cache has no handoffs).
5. Gotchas: mechanize first (TMPDIR in test setup); facts stay in CLAUDE.md; unfixed failures
   go to the store as lessons. Gotchas then hold facts only.
6. Progress for agents is computed, not read: `bosun status`/`resume` (leases, PRs, states) +
   handoff `next:`/`blocker:` lines. PROGRESS.md keeps Status (<= 15 lines) + Blocked; the fold
   script writes Done to `docs/progress-done.md`; CLAUDE.md: agents never read it.
7. Curator compacts as §6 today (caps 40/8, merge, mechanize, retire after 30 days no hit).
8. Not now: org-wide lessons (a splice piece like D-004) wait for evidence of a repeat across
   repos; `lesson add --upstream` covers lessons about bosun.

Cost: session start + claim load <= ~400 tokens of memory; reading PROGRESS.md cost ~21k (bosun).

## 3. Tasks (one PR each; sizes exclude tests)

- T1: compact render, 160-char cap, `--evidence` for manual add, dup -> hit
  touch: docs/spec/06-learning.md, src/core/lessons.rs, src/cli/lesson.rs, src/cli/mod.rs, src/cli/review_pack.rs, docs/defaults.md
  size: 150
- T2: `claim` returns `data.lessons` for the touch
  touch: src/cli/claim.rs, docs/spec/04-lifecycle.md
  depends: T1
  size: 80
- T3: SessionStart prints global lessons, read before any GitHub call
  touch: src/hooks/session_start.rs, docs/spec/05-spawn.md
  depends: T1
  size: 80
- T4 (gated: channel): fold Done into `docs/progress-done.md`; split bosun PROGRESS.md once;
  mechanize TMPDIR; CLAUDE.md "never read progress-done.md"
  touch: scripts/fold-fragments.sh, channel/pieces/core/files/.bosun/fold-fragments.sh, tests/fragments.rs, tests/channel*.rs, docs/spec/17-template-updates.md, docs/PROGRESS.md, docs/progress-done.md, docs/progress.d/README.md, CLAUDE.md, .cargo/config.toml
  size: 120 (+ moved lines)
- Follow-up outside bosun: hull, hoist, berth take T4 through the channel after a bosun release
  and move their own unfixed Gotchas into `docs/lessons.toml`.
