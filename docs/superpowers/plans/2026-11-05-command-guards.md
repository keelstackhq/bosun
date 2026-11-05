# Plan: command guards (N2, nano-coder study)

project: bosun

Owner approved (owner-class: guard, hooks, `.claude/settings.json`, AGENTS.md, channel piece).
One task, one PR, at most 400 changed lines (tests do not count). Spec §4.

- [x] **T1 Command guards** · touch: `src/core/guard.rs`, `src/core/repeat.rs`,
  `src/hooks/pre_tool_use.rs`, `.claude/settings.json`, `channel/pieces/core/files/.claude/`,
  `AGENTS.md`, `channel/pieces/core/files/sections/agents.md`, `docs/spec/04-lifecycle.md`,
  tests, fragments.
  - Destructive-lock in `tool_rules` (forms in spec §4); `.claude/settings.json` denies the
    plain forms too.
  - Repeat-lock: the same Bash command in a row per session, agent and directory warns at 3 and
    is denied from 5; read-only polls are not counted; another command or tool call resets it.
  - AGENTS.md and `sections/agents.md` gain one line each about both locks.
