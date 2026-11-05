# Plan: command guards (N2, nano-coder study)

project: bosun

Owner approved (owner-class: guard, hooks, `.claude/settings.json`, AGENTS.md, channel piece).
One task, one PR, at most 400 changed lines (tests do not count). Spec §4.

- [x] **T1 Command guards** · touch: `src/core/guard.rs`, `src/core/` (pure repeat logic),
  `src/hooks/pre_tool_use.rs`, `src/hooks/cache.rs`, `.claude/settings.json`,
  `channel/pieces/core/files/.claude/settings.json`, `AGENTS.md`,
  `channel/pieces/core/files/sections/agents.md`, `docs/spec/04-lifecycle.md`, tests, fragments.
  - Destructive-lock in `tool_rules` (every Bash segment, nested shells too): `rm -r/-R/-f…` of
    `/`, `/*`, `~`, `~/`, `$HOME`, `${HOME}`; `mkfs*`; `dd … of=/dev/*`; redirect into `/dev/sd*`
    etc.; `terraform destroy` (and `apply -destroy`); SQL `DROP DATABASE`/`DROP SCHEMA`
    (any case, when a SQL client runs it); fork bomb; `chmod/chown -R … /`. `rm -rf ./target` is fine.
  - `.claude/settings.json` gets `permissions.deny` for the plain forms (belt and braces).
  - Repeat-lock: the same Bash command (trimmed) N times in a row in one session: N=3 adds
    `additionalContext` (warning), N≥5 denies. Counter in `<git common dir>/bosun/repeat/<key>.json`
    `{session, command, count}` per session, agent and directory; a different command or an
    edit resets it. Pure counter in `src/core`.
  - AGENTS.md and `sections/agents.md` gain one line each about both locks.
