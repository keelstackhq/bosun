## 2. Scope and sub-projects

SP1: bosun, the orchestration helper (this document). SP2: the agent runtime, in its own repo, is
bosun's first consumer and is designed in a later session (out of scope, D25). This document gives
only the interface that SP2 uses: `bosun init --stack rust --plan runtime-plan.md`.

In scope for bosun: the work lifecycle, spawning and limits, verification gates, risk gating,
main-health, cost, learning, Slack intake, bootstrap and reuse. Not in scope: an LLM client, a
model host, a database, a web UI. bosun uses the GitHub API and asks the session for
claude-code-remote calls (D29). It does not run models.
