## 14. Front-loaded human prerequisites

The owner works mainly from a phone (org decision D-009). Every step of a request that only a
human can do is asked for at the start, in one checklist; agents then run to the end. The only
later human touch is the approvals the risk model needs (§8), and those come in batches.

**Declaring them.** A plan lists them in a `## Prerequisites` section (format:
`docs/prerequisites.md`): items `- H1: title` with `on:` (`phone` or `computer`), `steps:` (one
line), `verify:` (`owner`, `secret [ENV/]NAME`, `variable [ENV/]NAME`, `environment NAME`,
`file PATH`), `blocks:` (items or `<item>/<task>` of the plan), `who:` and `replaces:`. The plan
parser ignores the section; `core::prereqs::parse` reads it and every error has a line and a fix.
Plan text that reaches the owner cannot hold `<!--`, `-->` or Unicode Cc, Cf, Zl or Zp characters.

**The checklist.** `bosun prereqs <plan>... [--post] [--no-setup]` lists the items (computer
steps first), the missing `bosun doctor` steps (`D<n>`), a delegate key (`K1`) when a task needs a
delegate and none is enrolled, and the approvals at merge: per task an estimate from its touch
paths, as risk-check would judge them (an owner rule, a delegate rule on the critical floor, a
delegate and a two-verdicts rule, a `CRITICAL_RULES` match, an unreadable touch, or `risk: high`
need the owner; delegate rules alone a delegate), grouped into dependency batches. `data.message`
is the short text for the owner's chat. `--post` works from the plan on the default branch: a
local copy that differs is exit 3 and two checklist issues are exit 5, checked for every plan
before any write. With `--post` (after the kill switch) bosun keeps one
issue per plan, labelled `bosun:prereqs`, with tappable boxes; it closes when nothing is open.
All text an agent or a plan wrote is shown inert (`core::text::inert`: one line, an inline code
span).

**Trust: the plan decides, the issue only ticks.** What an item is (id, blocks, verify) comes only
from the plan file on the default branch. bosun reads API items through the API (a new
`Github::exists`); a 403 is its own state, and then the owner's tick decides. A tick counts only
for an owner item or a 403 item, on exactly one line whose `sum` (8 hex of the SHA-256 of title,
steps, verify and who) matches the plan; a missing, repeated or stale line is open. Each line's
marker is `<!-- bosun:prereq H1 sum=1a2b3c4d state=api -->`; `state` is informational. bosun
never ticks a box itself: an API item it verified stays `[ ]` and says "verified by bosun", so a
tick is always the owner's. A broken task -> epic -> plan link (a gone epic, an epic marker
without a plan, a task the plan does not have) fails closed; task and epic bodies are
agent-editable like the checklist. The
PreToolUse hook refuses an agent's issue-body write (gh issue/pr comment, gh api and curl writes,
the GitHub MCP issue tools) that carries one of bosun's markers (`bosun:prereq`, `bosun:task`,
`bosun:epic`); best effort, with its bypasses listed in `core::guard`.

**Gating.** A task an open item blocks does not start:
- `bosun plan import`/`amend` of a plan with prerequisites refreshes the checklist first (a dry run
  only reads), keeps such tasks `planned` (an epic too when its whole item is blocked) and lists
  them under `data.held`; the next import after the item is done promotes them.
- `bosun claim` refuses such a task (exit 5, nothing written), `bosun wave plan` leaves it out
  (`data.held`), and `bosun spawn` refuses an epic blocked as a whole item. They find the plan
  through the task's epic and fail closed (exit 5, fix `bosun prereqs <plan> --post`): a plan file
  that cannot be read on the default branch or does not parse, two checklist issues, or no
  checklist issue while a blocking item needs a tick. Only items that block the target count.

**No mid-run asks.** An agent that holds a task's lease and meets a human step nobody listed
runs `bosun hold <task issue>
"<step>"`: bosun records the hold on the task, adds a box `X<n>` to the plan's checklist, releases
the task and records a lesson; the claim gate keeps the task until the box is ticked. The
orchestrator relays the updated checklist once and folds the step into the plan (`replaces: X1`).

**Approval batches.** `bosun approve --list` gives `data.message` (at most 10 ready PRs with inert
titles and the reply `approve 12 15`), `data.message_id` (8 hex of the SHA-256 of the shown pairs'
sorted `<pr>@<head>` lines) and `data.message_pairs`. Only the latest relayed message is
answerable; PR numbers come only from the owner's chat; each (id, pr) is signed at most once (§8).
