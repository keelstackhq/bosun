## 14. Front-loaded human prerequisites

The owner works mainly from a phone (proposed org decision D-009). Every step of a request that only a
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
the task and records a lesson; `bosun claim` refuses the task until the box is ticked (`bosun wave plan`
does not read holds yet, so a held task can still be offered; its claim fails). Ids are unique
for the plan's life (a new one skips the plan's `replaces` ids). The orchestrator relays the
updated checklist once and folds the step into the plan (`replaces: X1`, in an item that blocks
the held task; `replaces` never releases another task's hold).

**Approval batches.** `bosun approve --list` gives `data.message` (at most 10 ready PRs with inert
titles and the reply `approve 12 15`), `data.message_id` (8 hex of the SHA-256 of the shown pairs'
sorted `<pr>@<head>` lines) and `data.message_pairs`. Only the latest relayed message is
answerable; PR numbers come only from the owner's chat; each (id, pr) is signed at most once (§8).

**Daily digest (§18, `approvals_batch = "daily"`).** `bosun digest` (the orchestrator, once a day)
reads the `approve --list` board and the needs-human issues, and writes nothing. `data.message`
is one D-012 chat form for the repo: its first line `Daily digest <owner>/<repo> <id>`, every
owner-waiting PR (`#<pr>@<short head>`, its `R-` rule ids, the inert title), the open asks
(answered with `bosun answer <issue> --chat`), budget asks (none until plan PR 7) and the reply
`<id> approve 12 15`. While main is red, mainfix PRs are in `data.bypass`,
not in the digest; release-environment taps never wait for it. What a tokenless listing could
not read is in `data.skipped`. `data.header` is `/bosun digest <owner>/<repo> <day> <id>
<pr>@<head>...` (`day`: the digest's UTC date; `id`: as `message_id` above over the pairs and a
`day <day>` line, so each day's digest has its own id; an unkeyed checksum that catches copy errors, not
forgery: the orchestrator relays the header of the message it showed). `bosun digest --answer`
(after the kill switch) takes that header as the first line, then the owner's reply verbatim: a
header that does not check or names another repo is exit 2. The owner quotes the id they saw
(the form's last line, pasted as is, is a valid reply): a reply that quotes no id is exit 2, one
whose quoted ids are not all the header's id is exit 3, so a reply never pairs with another
digest's heads. A PR the owner names counts only when the digest showed it and it still waits at that
head, else it is in `data.dropped` (a moved head is listed in the next digest). Each approval is
a `next` call `bosun approve <pr>@<head> --sign-key <pem>` for the key holder: the digest signs
no approval. A raise `<id> raise <epic> +<usd>` counts only with `--sign-key` and a key enrolled
as owner on the default branch: `data.raises` carries the signature of `bosun-raise v1
<owner>/<repo> digest <id> day <YYYY-MM-DD> reply <sha256 of the normalized reply> epic <n> usd
+<usd>`: `day` is the header's, the hash covers the reply lines after the header (normalized as
`answer --chat` text), and the same raise twice in one reply counts once, so a reply relayed
again signs the same bytes (plan PR 7 records raises and counts one message once). Without
`--sign-key`, or with a key enrolled but not as owner, the raise is in `data.refused`, exit 0,
approvals unchanged; an unreadable key or approvers file is exit 1, and `--sign-key` without
`--answer` is exit 2. One reply raises an epic at most once: two amounts for one epic are exit 2.
A raise counts at most once per (digest id, epic). bosun records nothing here: plan PR 7 records
raises, counts each (digest id, epic) once, refuses a raise whose digest day is stale (a bounded
age), checks the epic and caps the amount. Other
reply lines are in `data.unknown`, never guessed. Not yet: needs-human PRs from `heal` (plan PR
3a) and the "brake on" lead line (plan PR 5).

**App check runs (B2, §15).** Once per org, mostly from a computer, as H1-H7 of
`docs/superpowers/plans/2026-10-28-app-checks-team-owners.md`, in this order: H1 create the App
(the permissions of §15, webhook off, installed on the chosen repos only); H2 generate its
private key; H3 create the environment `bosun-app` whose deployment branch policy allows only
`main`, with no other protection rule (no required reviewers, no wait timer: they would hold
every gate run); H4 set the variable `BOSUN_APP_CLIENT_ID` (the App's client id) and the
secret `BOSUN_APP_KEY` in that environment, never as a repo or org secret; H6 release B2 and pin
it; H7 check that the agents' credential has no Administration, Environments or Secrets write;
H5, after the dual phase, switch the ruleset with `hoist apply --area github`. A repo in another org makes its own App.

**Zero-touch onboarding.** With an intake file (spec §3, §11), the per-repo human steps become
one-time org steps: the Claude GitHub App and the provisioner app on all repositories, the shared
"keelstack" cloud environment with `docs/setup-script.sh`, and the provision repo with its
environment secrets (H1-H5 of `plans/zero-touch-onboarding.md`). Per repo, three things stay
human: `hoist release bootstrap` for a new package, the `release` tap, and approvals per D-014
(draft). `bosun init --intake` returns the remaining steps in `data.owner_steps`.
