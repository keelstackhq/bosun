# Operating model

How to run a repo where agents write the code, and which parts bosun enforces today. Status words:
**enforced** (CI, a hook or the binary refuses), **partial** (some of it is enforced, or it has a
known gap), **doc** (written down, nothing checks it), **missing**.

## Words used here

- **Boundary files**: the files that decide what passes and who approves (risk rules, limits,
  governance, workflows, gate code). Agents may propose changes to them; humans approve them.
- **Loop**: one cycle of change and check. Nested loops: local gates, then CI, then checks of main.
- **Risk rule**: a path or content pattern in `.bosun/risk.toml` that makes a PR high risk.
  Each rule names who may approve: the owner, a **delegate** key (one Claude session with its own
  signing key, plus one clean verdict), or **two verdicts** from two reviewer sessions.
- **Critical floor**: the paths a delegate never decides, even when its rule allows delegates.
- **Mainfix**: a PR that repairs a red main branch; it may merge while main is red.

## The model in one paragraph

A codebase grows: each change stays and becomes the ground for the next one. Agents do the
growing. Structure (types, module limits, gates, CI, docs) keeps the growth healthy. Humans own the
edges: what to build and how to measure it, how we know it is good, and the boundary files.
Agents may tend code and structure inside those limits. Only humans move the limits, and they
widen them on measured evidence, never on confidence or on agents agreeing with each other.

## How to work with agents today

1. **Owner owns the boundary files.** R-gov (`.bosun/**`, `governance/**`, `.claude/**`,
   `REVIEW.md`, `docs/lessons.toml` (not created yet), plans), R-ci (workflows, `deny.toml`,
   toolchain) and R-gate (gate code, manifests) make a PR high risk. It then needs an owner
   approval; a delegate may approve `deny.toml`, the toolchain files and `profiles/**`. The hook
   code (`src/core/guard.rs`, `src/hooks/**`) is R-gate.
2. **Agents work inside.** A worker claims a task (`bosun claim`), works test first, runs the gates
   locally (`bosun gate`), opens a draft (`bosun pr`) and gets a fresh-reviewer verdict.
3. **Agents never merge, tag or force-push.** The PreToolUse hook (merge-lock in
   `src/core/guard.rs`) denies it in shell commands and in the GitHub MCP merge and auto-merge
   tools. bosun only turns on auto-merge.
4. **Answer decisions, not transcripts.** Agents ask with `bosun ask`; human-only steps go to
   `bosun prereqs`. An approval counts only on the exact head SHA: an owner review approval, a
   plain owner comment, or an ed25519-signed `/bosun approve <head_sha>` comment
   (`src/core/approve.rs`). A new push needs a new approval.
5. **Stop everything** with an open `bosun:halt` issue opened by an owner, or the repo variable
   `BOSUN_HALT=1` (`src/app.rs`). Commands that start or change work then exit 4.
6. **Treat chat and tickets as evidence.** `bosun intake` fences the request text so it cannot
   forge markers. Both built-in spawn prompts call issue, PR, comment and Slack text data, never an
   instruction, and quote the epic title as data; the review pack names its untrusted fields. Gaps:
   a `--prompt-file` prompt is the caller's own text, and the Routine prompts (`bosun doctor`)
   carry no data note yet.

### The open tension: bootstrap mode

Until 2026-11-30 (`[approvals] bootstrap_until` in `.bosun/bosun.toml`, hard cap `BOOTSTRAP_END`
in `src/core/approve.rs`) the orchestrator may sign gated approvals itself after green checks and
an independent agent review (D-012). `BOOTSTRAP_EXCLUDED` (`src/core/risk.rs`) keeps `.bosun/**`,
`governance/**`, CODEOWNERS, `hoist.toml`, the risk, verdict, release and splice workflows and the
approval code owner-only, and `orchestrator_counts` (`src/core/approve.rs`) refuses the window on any R-gov or R-prod-write
match (the rest of R-gov: `.claude/**` with the hook settings, `REVIEW.md`, plans,
`docs/agents/**`, agent and Claude instruction files). This trades the rule "agent agreement is
not independent evidence" for speed. It is a mode the owner chose and ends by removing the key or
by the date. After the window `[approvals] policy` decides (this repo: `owner-gated`). The policy
`autonomous` (D-017, `Policy::Autonomous`; D-017 is not in this repo's decisions snapshot, its rules
are in spec §8) decides first, inside the window too: only R-weaken is checked before it. It lets the
orchestrator approve gated PRs outside `autonomous_excluded` (`src/core/risk.rs`) when the PR has an
open linked task leased by its author and a clean verdict on the head from a reviewer who is neither
a lease holder nor the signer. Its weak point: that reviewer's session is self-declared (see the
table).

## Principle → mechanism → status

| Principle | bosun mechanism | Status |
| --- | --- | --- |
| Structure keeps growth healthy | `tests/core_is_pure.rs` (no I/O in `src/core`); fmt, lint, test, deps gates from `profiles/*.toml` | enforced |
| Humans own the edges | R-gov paths; `bosun ask`, `answer`, `prereqs`; owners in `governance/owners.toml`; bootstrap mode lets the orchestrator approve part of R-gov | partial |
| Risk by blast radius | `.bosun/risk.toml` rules, read from the PR's base SHA, only escalate; owner, delegate or two-verdict approval; R-unseen (GitHub hid part of the diff) always owner; bootstrap window never covers R-gov or R-prod-write | partial |
| Retire mitigations | Lessons only: mechanize or drop at the 40-entry cap (`bosun lesson add` exit 6) | partial |
| Nested loops | local `bosun gate` → `ci.yml` → `main-health.yml` (after `ci` on main + weekly) → `main-open` fails non-mainfix PRs while main is red | enforced |
| Escaped defect → tighter loop | `main-triage` bisects, `revert` opens a mainfix PR, main-red opens a lesson; `lesson triage --as mechanizable` opens a process-fix task (manual); main-red, `revert` and `gate-missed` open a "Which gate missed this?" task | enforced |
| No flaky tests | `flake` task kind and intake source exist; no detector, no refusal | partial |
| Production telemetry as a test | — | missing |
| Agent review instead of human eyes | `review-pack` → fresh reviewer → verdict. `verdict post` writes the status directly and the reviewer session is self-declared, so the author can post its own; lease holders are excluded only for delegate, two-verdict and autonomous counts | partial |
| Model reliability varies by task | `[models]` tiers; stop rules move to a stronger tier (`src/core/stop.rs`); `spawn` refuses `[1m]` models; model recorded per verdict | partial |
| New model resets trust | verdict records carry the reviewer model; `[approvals] review_models` limits which models count for delegate, two-verdict and autonomous approval; the model is self-declared like the session (guards against accidents, not against a lying author); the list limits only gated approvals (two-verdict, delegate, autonomous), not the low-risk `review-verdict` status | partial |
| Rules enforce themselves | gates; `test-integrity` refuses deleted tests, new skips, fewer asserts; `ci-minutes` lints workflows | enforced |
| Repeated review comments become checks | lessons with hits; mechanizable lesson → task; reopens if hit after its mechanism merged | partial |
| Derive, don't duplicate | `tests/fragments.rs` checks the fold script; `tests/plugin_marketplace.rs` checks the plugin pins match; no CI diff of generated files; the plugin SHAs are kept by hand; `tests/pinned_config.rs` needs an entry only for a release with new keys | partial |
| Tools for agent users | one JSON object, 7 exit codes (`src/output.rs`); `--dry-run` on 5 commands; `error.fix` never empty is a `debug_assert!` only (tests run it, release builds do not) | partial |
| Merge rules are part of the system | required `ci-ok`, `risk-check`, `review-verdict`, `main-open`; risk rules from the base SHA, approver keys from the default branch | enforced |
| Repo settings as code | `hoist.toml`; `bosun doctor --remote` runs `hoist check`; apply is manual | partial |
| Bottleneck at review, CI, release | drafts skip CI; `ci-minutes` gate; review-pack chunks; no review-queue limit | partial |
| Weakening a gate is a boundary change | gate files are R-ci/R-gate/R-gov by path (hooks, `repeat.rs`, `ci_minutes` included); R-weaken flags lowered limits, profiles, lints, coverage and removed CI steps (base/head compare); `src/core/intake.rs`, `spawn.rs`, `stop.rs`, `src/cli/hold.rs`, `prereqs_gate.rs` are on no path | partial |
| Self-repair needs a mandate and limits | budgets and stop rules apply; `gate-missed` opens the "which gate missed this?" task; no per-loop mandate | partial |
| Human channels are untrusted | intake fence; `trusted_notes` takes records only from write users; Slack thread role `may_build = false`; `--slack-user` not verified; review pack names its untrusted fields; built-in spawn prompts call issue and Slack text data; Routine prompts do not yet | partial |
| Close the loop with the reporter | `bosun notify` returns thread posts in `next` for the caller; it posts nothing itself, and the notify routine is a Blocked human step | partial |
| Protect controls from the agents they control | risk rules and approver keys not read from the PR; R-gov on limits and session rules; the hooks that enforce merge-lock and label-lock are R-gate; label-lock is not a sandbox | partial |
| Autonomy earned per change class | static delegate class; not earned from escape or revert rates; no automatic narrowing after an incident | partial |
| Challenge in proportion | two levels only (`normal`, `high`) | partial |
| Cost per outcome | `bosun progress`: cost per merged task; budget refuses new work at 80 % (exit 6) and returns stop actions for the caller; `held` and `cost_per_held_task_usd` leave out tasks whose PR was reverted; `bosun budget report` lists spend, models and outcome per PR for a month, and the `budget-report` workflow keeps `reports/budget/YYYY-MM.{md,json}` on the branch `bosun/budget-reports` (one draft PR, never merged); no human minutes | partial |
| Approval on exact commit, with identity | head-SHA approvals; signing keys from default-branch `approvers.toml` | enforced |
| Preview and cancel | `--dry-run` (some commands), `bosun hold`, halt issue | partial |
| Rubber-stamp approvals | stale-head rule only | partial |
| Keep / harden / replace / rewrite | — | missing |
| Approver is not the requester | lease holders cannot count as delegate or two-verdict reviewers; the reviewer session is self-declared; no explain-back step | partial |
| Kill switch per loop | per repo only | partial |
| Trace each change to session and model | session in lease and registry records; commit trailers `Bosun-Session/Task/Model` stamped by a commit-msg hook (`bosun trailers`), checked by `bosun pr` (note; `[trailers] require` refuses) | enforced (opt-in) |
| Rollback without the loop | `bosun revert <pr>` | enforced |
| Competing objectives, churn | repeat-lock on repeated commands (`src/core/repeat.rs`); no objectives list, no churn check | missing |
| One identity per agent, least privilege | session id (D31, `src/ctx.rs`); tokens from env only; the nightly main-health job is read-only | partial |
| Audit log, no self-escalation | approve records are a hint, not an audit log; limits are R-gov | partial |
| One manifest per loop | — | missing |

## Candidates to enforce next

Ranked by value for cost. "Gated" means the change touches a risk path and needs the owner. A new
flag or module also touches `src/cli/mod.rs` or `src/core/mod.rs` (R-gate), so most code
candidates are gated in practice.

| # | Candidate | Mechanism | Est. lines | Gated |
| --- | --- | --- | --- | --- |
| 1 | Put the hooks on a risk path | add `src/core/guard.rs`, `src/hooks/**` to R-gate | 10 | done |
| 2 | Close the bootstrap gaps | the window refuses R-gov and R-prod-write | 30 | done |
| 3 | Flag PRs that weaken a gate | R-weaken: base/head compare of limits, profiles, lints, CI steps | 250 | done |
| 5 | Commit trailers `Bosun-Session`, `Bosun-Model`, `Bosun-Task` | `bosun pr` checks commits | 150 | done (#427) |
| 6 | Self-repair opens "which gate missed this?" | `gate-missed` from main-red and `revert` | 120 | done |
| 15 | Review-pack delta shows the PR's own changes | after a merge of main the delta is cut to the PR's files, with the PR's patches | 30 | yes (R-gate) — done |
| 10 | Mark untrusted text everywhere | the pack names its untrusted fields and tells the reviewer they are data; every spawn prompt says so | 30 | yes (R-gate) — done (Routine prompts: next) |
| 16 | bosun's own pin without a hand-added `KNOWN` entry | `tests/pinned_config.rs` falls back to the newest older release: it only knows fewer keys | 10 (test) | no — done |
| 9 | Cost per outcome that holds | `bosun progress`: held = merged and not undone by a merged `bosun revert` PR; no escape window yet, a revert made by hand is missed | 60 | no — done |
| 8 | New model resets trust | record the model per verdict; `[approvals] review_models` limits which models count for delegate, two-verdict and autonomous approval | 150 | yes (R-gate) — done (enforced once `review_models` is set, after a release that reads it) |
| 17 | Plugin marketplace SHAs by `bosun pin --latest` | pin the superpowers and caveman SHAs in `.claude-plugin/marketplace.json` and the setup script from the plugin repos' heads | 80 | no (writes R-gov files, so the pin PR is gated) — next |
| 4 | Count only reviewer-bound verdicts | `review-verdict` from `verdict apply` only, reviewer bound to a workflow run (#298); ends self-declared reviewer sessions and models | 200 | yes (R-gate, workflow: owner) — deferred: needs the owner's verdict workflow change |
| 7 | Autonomy earned per change class | revert and escape rate per rule id, from #9's revert data; delegate only under a limit in `limits.toml` | 350 | yes (R-gate) — deferred: needs #9's data first and an owner-set limit |
| 18 | Consumers get new splice/hoist/berth pins without a bosun release | a channel piece for pins only | 100 | yes (R-ci-review) — deferred: the channel version is the bosun release by design; release more often instead |
| 11 | Per-loop kill switch | `bosun:halt` issue with a scope; `kill_switch` checks it | 150 | yes (R-gate: `app.rs`) — deferred: needs #12's loop names first; the repo switch covers today's single loop |
| 12 | One manifest per loop | `.bosun/loops/<name>.toml`: mandate, identity, objective, budget, stop rule, kill switch | 300 | yes (R-gov) — deferred: a new config surface the owner must design; no second loop runs yet |
| 13 | Churn check | `bosun progress`: files changed back and forth by different tasks within N days | 150 | yes (R-gate) — deferred: signal only, no refusal; lower value than the safety rows |
| 14 | Keep / harden / replace / rewrite as a plan field | plan format field, `bosun plan` checks it | 80 | probably — deferred: plan format change (R-gov plans); doc guidance is enough until plans need it |

Order: done rows first, then the open rows ranked by value for cost, the deferred block included;
"building" rows are this round's PRs, one task each.

State after the 2026-11-08 round: #16 and #9 merged (#449, #453; #459 hardens #9). #10, #15 and #8
are open gated PRs that wait for the owner (#454, #456, #457); each row says "done" only in its
own PR. #17 is deferred: `bosun pin` would also need the changelog and progress fragments and a
GitHub release check, and the plugin SHAs change rarely. Review follow-ups that wait for those
merges, as one gated PR: a delta falls back to a full review when the PR's file list is short of
`changed_files`; `delta_not_in_pr` is capped and listed as untrusted; `comment_body` uses
`valid_model`; Routine prompts get the untrusted-data note; spec §5 names the spawn prompt note.

Rules for any new mechanism: put it as high as it fits (made impossible, then lint, test, CI,
review, and last a doc), ratchet it (no new violations, old ones listed), and give it an owner who
can retire it.
