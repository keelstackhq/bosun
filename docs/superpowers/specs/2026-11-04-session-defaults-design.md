# Session defaults: tools and standing rules in every cloud session

Status: R1-R5 built in bosun#386; R6 (release) and H follow. Answers two owner questions (2026-11-04).

## Design

**Q1. What a cloud container needs.** The shared environment "keelstack" runs
`docs/setup-script.sh` (as root, before the checkout exists). It installs Node 24, Rust, and
the pinned splice, hoist, berth, and bosun. bosun needs nothing more. The core piece's
`.claude/settings.json` runs its hooks. The platform injects a GitHub token for each session as
`GH_TOKEN`/`GITHUB_TOKEN`, so the environment holds no PAT. hull is used once, at repo creation.
hoard is not needed in sessions. hull's `session-start.sh` only installs dependencies. hull's
template `.claude/settings.json` declares superpowers (`obra/superpowers-marketplace`) and
caveman (`akramsystems/caveman`) in `enabledPlugins`. The bosun channel does not, so repos that
hull did not create (bosun is one) do not get them. This session had neither plugin. The rules
text exists nowhere. hull's AGENTS.md says "don't compact", which conflicts with the owner's
rule. R5 settles it.

**Q2. Mechanism: two layers, with no paste and no PAT.**
1. *Tools.* hull keeps its plugin block. For repos that hull did not create, the channel core
   adds the same block. If hull already wrote one, splice leaves it alone, so `.claude/settings.json`
   has a single writer. The plugins come from a bosun-hosted, sha-pinned marketplace `keelstack`
   (`.claude-plugin/marketplace.json`; each plugin's source commit is pinned there). Settings and
   the setup script pin that marketplace to a bosun commit; drift fails `bosun doctor` and the setup script. Plugins run their own hooks in every session, so `.claude/**` stays owner class. The
   setup script prewarms the same pins with `claude plugin install --scope user`, using its own
   copy of the pins, because no checkout exists yet. On failure it warns and does not fail.
   One install fits every repo in the environment. `bosun doctor` (R4) catches drift between
   the setup script's pins and the repo's pins.
2. *Rules.* `bosun hook session-start` adds a rules block to `additionalContext`. It does this
   on the success path and on the error path, with no GitHub call. The role comes only from
   `Origin`: spawned (`CLAUDE_CODE_CHILD_SESSION`) means worker. Any other origin, relay roots
   included, means orchestrator.
   - **Orchestrator:** split the work into dedicated sessions (`bosun spawn`, `[models]`).
     Archive each child once its `result` is in. Tell the owner when to run /compact, or relay (§5.2).
   - **Worker:** use superpowers (brainstorm, then plan) and caveman, with `[models]`
     subagents. Report in at most 10 lines. Spawn no sessions.
   - **All:** be as token-efficient as possible. Write only what is needed.
   The text is in `.bosun/session-rules.toml` (owner). The built-in default text is in
   `src/core/session_rules.rs`. R1 adds that file to R-gov, because it steers every agent.

**Rejected.** `~/.claude/CLAUDE.md` from the setup script (no roles; every change needs a re-paste);
hull AGENTS.md (hull repos only); the channel's `agents.md` (no roles); an output style (per
user, replaces part of the system prompt); a new piece (the core piece ships the hook).

**Settled (owner, 2026-11-04).** Caveman applies everywhere, the orchestrator's chat with the owner
included. Long context: the agent picks /compact or a fresh start and tells the owner which; hull's
"start fresh rather than compacting" changes to match. Plugin sources come from the bosun-hosted
`keelstack` marketplace, which pins each plugin commit; settings and the setup script pin the marketplace
to a bosun commit, and drift fails `bosun doctor` and the setup script.

## Tasks

| Id | Task | Touch | Risk |
| --- | --- | --- | --- |
| R1 | `core::session_rules` (parse, defaults, role from `Origin`); hook emits it on both paths; add `src/core/session_rules*` to R-gov (risk.toml, channel jinja, spec 08); spec §4 | src/core, src/hooks, .bosun/risk.toml, channel, docs/spec | owner (R-gov) |
| R2 | `status`/`reap` `next`: `archive_session` per live child with a `result` record, skipping relay roots (D29) | src/cli/status.rs, reap.rs, spec §5 | low |
| R3 | plugin block with source-SHA pins in channel core + bosun `.claude/settings.json` (skip when present); setup-script prewarm | .claude/, channel core, docs/setup-script.sh | owner |
| R4 | `bosun doctor`: plugins present, pins match the repo's | src/cli/doctor.rs | low |
| R5 | channel seed `.bosun/session-rules.toml` with the owner's text; settle the compact rule with hull | channel/pieces/core | R-ci-review (two verdicts) |
| R6 | release bosun; bump `.bosun/version`, setup-script `BOSUN_VERSION` and sha pins | .bosun, docs/setup-script.sh | owner |
| H | owner: paste the new setup script into "keelstack" (computer); `bosun doctor` verifies | — | human |

Order: R1, R2 and R3 in parallel; then R4 and R5; R6 last; then H. Spike before R3: in a hull
repo's cloud session, check whether project `enabledPlugins` load with no prompt. If they do,
drop the prewarm.
