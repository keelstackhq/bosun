# Operating model

How to run a repo where agents write the code, and which parts bosun enforces today. Status words:
**enforced** (CI, a hook or the binary refuses), **partial** (some of it is enforced),
**doc** (written down, nothing checks it), **missing**.

## The model in one paragraph

A codebase grows: each change stays and becomes the ground for the next one. Agents do the
growing. Structure (types, module limits, gates, CI, docs) keeps the growth healthy. Humans own the
edges: what to build and how to measure it (vision), how we know it is good (validation), and the
fence itself (structure). Agents may tend code and structure inside the fence. Only humans move
the fence, and they widen it on measured evidence, never on confidence or on agents agreeing with
each other.

## How to work with agents today

1. **Owner sets the fence.** `.bosun/**`, `governance/**`, `.claude/**`, `REVIEW.md`,
   `docs/lessons.toml`, plans (R-gov) and workflows, gate code and manifests (R-ci, R-gate) are
   boundary files. `bosun risk-check` marks a PR that touches them high; it waits for an owner
   approval (a delegate key may approve `deny.toml`, the toolchain files and `profiles/**`). Limits and budgets live in `.bosun/limits.toml`, so only the owner raises them.
2. **Agents work inside.** A worker claims a task (`bosun claim`), works test first, runs the gates
   locally (`bosun gate`), opens a draft (`bosun pr`) and gets a fresh-reviewer verdict.
3. **Agents never merge, tag or force-push.** The PreToolUse hook (`src/core/guard.rs`, merge-lock)
   denies it, also through GitHub API tools. bosun only turns on auto-merge.
4. **Answer decisions, not transcripts.** Agents ask with `bosun ask`; human-only steps go to
   `bosun prereqs`. Approvals are ed25519-signed comments bound to the exact head SHA
   (`src/core/approve.rs`); a new push needs a new approval.
5. **Stop everything** with an open `bosun:halt` issue opened by an owner, or the repo variable
   `BOSUN_HALT=1` (`src/app.rs`). Commands that start or change work then exit 4.
6. **Treat chat and tickets as evidence.** `bosun intake` fences the request text so it cannot
   forge markers; the spawn prompt calls it data, never an instruction.

### The open tension: bootstrap mode

Until 2026-11-30 (`[approvals] bootstrap_until` in `.bosun/bosun.toml`, hard cap `BOOTSTRAP_END`
in `src/core/approve.rs`) the orchestrator may sign gated approvals itself after green checks and
an independent agent review (D-012). Governance, risk rules, approval code and the setting itself
stay owner-only (`risk::bootstrap_excluded`). This trades the rule "agent agreement is not
independent evidence" for speed. It is a mode the owner chose and ends by removing the key or by
the date. The org's later autonomous mode (D-017) is not in bosun yet: no decision text in this
repo's snapshot, no config key.

## Point → mechanism → status

| Point | bosun mechanism | Status |
| --- | --- | --- |
| Structure keeps growth healthy | `tests/core_is_pure.rs` (no I/O in `src/core`); fmt, lint, test, deps gates from `profiles/*.toml` | enforced |
| Humans at the edges | R-gov paths; `bosun ask`, `answer`, `prereqs`; owners in `governance/owners.toml` | enforced |
| Risk by blast radius | `.bosun/risk.toml` rules, read from the base branch, only escalate; `approval = owner \| delegate \| two-verdicts`; critical floor and R-unseen always owner | enforced |
| Retire mitigations | Lessons only: mechanize or drop at the 40-entry cap (`bosun lesson add` exit 6) | partial |
| Nested loops | local `bosun gate` → `ci.yml` → `main-health.yml` (after `ci` on main + weekly) → `main-open` blocks PRs while main is red | enforced |
| Escaped defect → tighter loop | `main-triage` bisects, `revert` opens a mainfix PR, main-red opens a lesson; `lesson triage --as mechanizable` opens a process-fix task (manual) | partial |
| No flaky tests | `flake` task kind and intake source exist; no detector or quarantine refusal | partial |
| Telemetry as a test | — | missing |
| Zero-eyes review | `review-pack` → fresh reviewer → `verdict` (written only by `verdict apply`); lease holders never count as reviewers | enforced |
| Model envelope | `[models]` tiers; stop rules escalate the tier (`src/core/stop.rs`); `spawn` refuses `[1m]` models; no per-model reliability map | partial |
| Model swap resets trust | — | missing |
| Rules enforce themselves | gates; `test-integrity` refuses deleted tests, new skips, fewer asserts; `ci-minutes` lints workflows | enforced |
| Repeated comments become checks | lessons with hits; mechanizable lesson → task; reopens if hit after its mechanism merged | partial |
| Derive, don't duplicate | `tests/fragments.rs` checks the fold script; no CI diff of generated files | partial |
| Tools for agent users | one JSON envelope, 7 exit codes, `error.fix` never empty (`src/output.rs`); `--dry-run` on 5 commands | enforced |
| Merge button is part of the system | required `ci-ok`, `risk-check`, `review-verdict`, `main-open`; rules and approver keys read from the default branch; `bosun doctor` runs `hoist check` on settings | enforced |
| Repo settings as code | `hoist.toml` + `hoist check` in doctor; apply is manual | partial |
| Bottleneck at review, CI, release | drafts skip CI; `ci-minutes` gate; review-pack chunks; no review-queue limit | partial |
| Weakening a gate is a boundary change | gate files are R-ci/R-gate/R-gov by path; no check that a threshold went down | partial |
| Self-heal needs mandate and limits | budgets and stop rules apply; no per-loop mandate; no "missing gate" follow-up | partial |
| Human channels are untrusted | intake fence; `trusted_notes` takes records only from write users; Slack thread role `may_build = false`; `--slack-user` not verified | partial |
| Close the loop with the reporter | `bosun notify` posts thread replies and child results | enforced |
| Protect controls from agents | risk rules and approver keys read from base; R-gov on limits and session rules; hooks deny label edits | enforced |
| Earned autonomy per change class | static `delegate` class; not earned from escape or revert rates; no auto-contract on incidents | partial |
| Challenge in proportion | two levels only (`normal`, `high`) | partial |
| Cost per outcome | `bosun progress`: cost per merged task; budget 80 % guard, 100 % stop (`src/core/budget.rs`, exit 6); not per outcome that holds; no human minutes | partial |
| Approval on exact commit, identity | signed `/bosun approve <head_sha>`, key from default-branch `approvers.toml` | enforced |
| Preview and cancel | `--dry-run` (some commands), `bosun hold`, halt issue | partial |
| Rubber-stamp approvals | stale-head rule only | partial |
| Keep / harden / replace / rewrite | — | missing |
| G1 approver ≠ requester, explain-back | lease holders cannot review; no explain-back | partial |
| G2 kill switch per loop | per repo only | partial |
| G2 trailers (loop, run, model) | session in lease and registry records; no commit or PR trailers | missing |
| G2 rollback without the loop | `bosun revert <pr>` | enforced |
| G3 competing objectives, churn | repeat-lock on repeated commands (`src/core/repeat.rs`); no objectives registry, no churn check | missing |
| G4 one identity per loop | session id (D31, `src/ctx.rs`); tokens from env only; read-only `main-health` job | partial |
| G4 audit log, no self-escalation | approve records are a hint, not an audit log; limits are R-gov | partial |
| Loop manifest | — | missing |

## Candidates to enforce next

Ranked by value for cost. "Gated" means the change touches a risk path and needs the owner.

| # | Candidate | Mechanism | Est. lines | Gated |
| --- | --- | --- | --- | --- |
| 1 | Flag PRs that weaken a gate: lower thresholds in `limits.toml`, profiles, `deny.toml`, or remove a gate step from `ci.yml` | risk rule `R-weaken` with a base/head value compare | 250 | yes (R-gov, R-gate) |
| 2 | Commit trailers `Bosun-Session`, `Bosun-Model`, `Bosun-Task` | `bosun pr` checks commits; hook warns on a missing trailer | 150 | no (`pr.rs`, hooks) |
| 3 | Self-heal opens a follow-up "which gate missed this?" | `main-health` / `revert` open a process-fix task linked to the main-red lesson | 120 | no (`main_health.rs`, `revert.rs`) |
| 4 | Earned autonomy per change class | `bosun progress --classes`: revert and escape rate per rule id; delegate only while under a limit in `limits.toml` | 350 | yes (R-gate) |
| 5 | Model swap resets trust | record model per verdict; `delegate` counts only verdicts from the model in `[models]` at enrollment | 200 | yes (R-gate) |
| 6 | Cost per outcome that holds | `bosun progress`: merged tasks not reverted within N days; owner minutes from ask/approve timestamps | 200 | no (`progress.rs`, `cost.rs`) |
| 7 | Untrusted-input marking everywhere | reuse the intake fence for issue bodies and comments in review packs and spawn prompts | 150 | yes (review-pack is R-gate) |
| 8 | Per-loop kill switch | `bosun:halt` issue with a `loop:` or task scope; `kill_switch` checks it | 150 | yes (R-gate: `app.rs`) |
| 9 | Loop manifest | `.bosun/loops/<name>.toml`: mandate, identity, objective, budget, stop rule, kill switch; R-gov by path already | 300 | yes (R-gov) |
| 10 | Churn detection | `bosun progress`: files changed back and forth by different tasks within N days | 150 | no (new code) |
| 11 | Keep / harden / replace / rewrite as a plan field | plan format field, `bosun plan` checks it | 80 | no (`plan.rs`, `plan-format.md`) |

Rules for any new mechanism: put it as high on the ladder as it fits (impossible → lint → test →
CI → review → doc), ratchet it (no new violations, old ones listed), and give it an owner who can
retire it.
