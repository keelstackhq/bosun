# Chat approval, no-CI mode, setup auto-detect (design)

Owner request 2026-11-10. Status: approved by the owner 2026-11-10 with merge option (a).

## Intent

bosun must work from minute one, inside Claude only (local CLI, claude.ai/code cloud, Claude Tag
in Slack). Today it does not:

- In a cloud session every PR is opened under the owner's own login (through the Claude GitHub
  App), so GitHub never lets the owner approve it with a review.
- The fallback, a `/bosun approve <40-hex sha>` comment, is unmemorable.
- The chat form already exists, but the owner's yes only counts once the orchestrator signs it with
  an ed25519 key in `.bosun/approvers.toml`. Enrolling that key takes `bosun keygen --enroll` plus a
  PR that the owner must approve on GitHub: the same dead end.
- risk-check, review-verdict and main-open are GitHub Actions statuses. A repo without Actions (or
  before the setup PR merges) has no way to pass them.

Owner's rule: "if there is a need for an approval, Claude asks the user; if the user agrees, it is
approved." Owner chose (option A): **Claude's word is the evidence**, with one guard: **only an
orchestrator session may record an approval, and it names the exact head SHA the owner saw.**

## Goals

1. Every gated PR is approved by one chat answer: AskUserQuestion locally and in the cloud, a
   thread reply in Claude Tag. Nothing to type on GitHub, nothing to remember, no key.
2. The approval works from the first PR of a repo (the setup PR included).
3. bosun works in a repo without CI: gates, risk check, review and approval run in the session.
4. A cloud session in a repo bosun never touched finds out by itself what is missing and runs the
   setup; a set-up repo goes straight to work.

## Non-goals

- Cryptographic proof that the owner answered. Option A trusts the orchestrator. Signed keys stay
  as an optional, stronger mode for repos that already have them.
- GitHub-side protections (environments, release taps, secrets, branch rules): GitHub enforces
  those and only the owner can change them.

## Anti-goals (a failure even if it works)

- A worker session (a child, a reviewer) that can approve its own PR.
- An approval that still counts after a new push.
- A second approval step outside the chat (a GitHub tap, a comment, a command to paste).
- A session that blocks on the owner when nothing gated is waiting.

## Constraints

- Spec rules stay: one JSON object on stdout, exit codes, `src/core` pure, tests offline, tokens
  never printed, agents never force-push or tag.
- Org decisions D-008 (who merges), D-012 (approvals by form + signature) and spec D13 ("agents
  never merge") describe today's flow. Changing them needs a PR in keelstackhq/decisions (owner
  merges). The bosun side lands behind that.
- R-weaken stays owner-only. With this design "owner" means "the owner's chat yes", so it changes
  how the owner approves, not who.

## Approaches considered

- **A. Chat record, orchestrator-only (picked).** The orchestrator asks; on yes it runs
  `bosun approve <pr>@<head> --chat`, which posts an approval record. No key, works everywhere.
  Trade-off: a session that lies, or a confused orchestrator, could write a record without asking.
- **B. Chat record plus an automatic key.** bosun makes and enrolls a key on first use. Harder to
  fake, but the key is lost when a cloud container is reclaimed, and enrolling it still needs a
  trusted first step. Owner declined.
- Keep signatures only: what exists today. Fails goals 1 and 2.

## Design

### 1. Chat approval

**Asking.** `bosun approve` (list mode) already returns the PRs that wait for the owner. Each item
gains an `ask` object: the question text (repo, PR number and title, risk rules hit, short and full
head SHA, the reviewer's verdict) and two options, "Approve <short sha>" and "Hold". `next` holds
the matching AskUserQuestion call for local and cloud sessions. For a Claude Tag session it holds
a thread post (the existing `slack_post` shape) asking the owner to answer "approve <short sha>".
Batching stays: one question can cover several PRs (multi-select), as in D-009.

**Recording.** On yes the orchestrator runs `bosun approve <pr>@<head> --chat` (for Claude Tag also
`--slack-user <U…>`, checked against `governance/owners.toml`). bosun refuses unless:

- this session's registry record is an orchestrator: a root (depth 1, no parent) or origin
  `local` or `slack`. A spawned child is refused (exit 5, `fix`: hand the question to the parent
  with `handoff-pr`);
- the head is still the PR's head;
- the PR has a clean review verdict on that head from another session (unchanged rule).

It posts one PR comment:

```
/bosun approve <head> chat
Approved by the owner in chat (session <orchestrator id>, <UTC time>).
<!-- bosun:chat-approval v1 session=<id> head=<sha> question=<id> -->
```

**Counting.** risk-check (and the no-CI local check, section 2) count a chat record as the owner's
approval when: the comment is unedited; it names the exact head; its author is an owner login
(in the cloud it is written through the Claude app as the owner, which is expected here, unlike
plain `/bosun approve` comments); and the session it names has an orchestrator record in the
registry. A new push makes it stale, like every approval today.

**Guard against workers writing it.** The PreToolUse hook already sees `mcp__github__*` and Bash.
It refuses any comment body or command that carries the `bosun:chat-approval` marker or the `chat`
approval form, unless the command is `bosun approve … --chat` (which does its own checks). Like the
label-lock, this stops accidents and confused workers, not a session set on lying (option A).

**Optional hardening (recommended, small):** a PostToolUse hook on AskUserQuestion. When the
question carries bosun's question id, bosun stores the owner's answer in the session's lease cache.
`approve --chat` then requires a stored "Approve" answer for that id and head. A session can still
fake it by hand, but a forgotten question can't turn into a yes by accident.

**Policies.** `[approvals] policy` keeps its meaning (who besides the owner may approve). A chat
record is the owner's approval, so it counts under every policy, including on R-weaken and gated
files. Signed approvals keep working. `.bosun/approvers.toml` becomes optional.

### 2. No-CI mode

**Detection.** `.bosun/bosun.toml` `[ci] provider = "github" | "circleci" | "none"`. adopt and
init write it: `none` when the repo has no Actions workflows and no `.circleci/`, or the owner asks
for it. Default when the key is missing: today's behaviour.

**What runs where.** With `provider = "none"`, `bosun pr <task> --ready`:

1. requires every gate stamp for the pushed head (the gates already write stamps);
2. runs risk-check locally with the base branch's rules (the command exists; today CI runs it);
3. requires a clean verdict record from another session (`verdict comment` already works without
   a workflow: bosun reads the comment itself instead of waiting for the status);
4. requires a chat approval record when risk-check says the PR is gated;
5. posts one summary comment (gates, risk, verdict, approval, head) so a human can see why it merged.

**Merging.** This is the open decision (see below). With no required checks, GitHub's auto-merge
either merges at once or refuses ("clean status"), so bosun can't rely on it.

### 3. Setup auto-detect

The setup script runs as root before Claude starts: it can't ask the owner and should not write
uncommitted repo files. So:

- The bootstrap (`bosun setup-snippet`) also installs bosun's hooks in **user scope**
  (`~/.claude/settings.json`: SessionStart, Stop, PreToolUse calling `bosun hook …`). They then
  run in every repo, set up or not. A repo's own `.claude/settings.json` hooks are the same
  commands, so it de-duplicates by command.
- `bosun hook session-start` in a repo without `.bosun/version` prints one line of context:
  "bosun is not set up here: run `bosun adopt --policy autonomous --owner <login>` and open the
  setup PR" (the login from `gh`/the app's `get_me`). In a set-up repo it prints today's summary.
  `bosun doctor` and `bosun status` give the same `next` step.
- The setup PR is gated, so it gets one chat question like any other. That works because a chat
  record needs no prior file on main: risk-check reads owners from the PR's base, and without
  `governance/owners.toml` on main the record counts only for the repo owner (or org admins).

## Owner decisions (settled 2026-11-10)

1. **Merging without CI.** Spec D13 and org decision D-008 say agents never merge; bosun only turns
   on auto-merge. Without required checks auto-merge cannot gate anything. Options:
   (a) a `bosun merge <pr>` command, orchestrator only, that re-checks section 2 and merges
   (the hook keeps refusing every other merge path). Needs a D-008/D13 change. **Chosen by the owner.**
   (b) Without CI bosun stops at "ready" and the owner merges by tapping in the GitHub app. Keeps
   D13, adds a step outside the chat.
2. **Org decisions.** D-008 and D-012 (and spec D13 for option 1a) need a PR in
   keelstackhq/decisions. This session can't reach that repo; the owner (or a session attached to
   it) opens it.

## Left to the builder

Exact question wording, the registry check for "orchestrator", the question id format, the order
of the PRs (suggested: chat record + counting; hook guard; `approve` list `ask`/`next`; user-scope
hooks + session-start detection; no-CI `pr --ready`; `bosun merge` if 1a), spec updates (§4, §8,
§14), and migration notes in `docs/adopt-existing-repo.md`.

## Calls made after the builder check (owner may override)

- Trust: the hook guard is the only barrier against a worker posting a chat record (Bash `gh`/curl
  included in the guard's patterns, best effort). Accepted with option A.
- Orchestrator = registry record with depth 1 and no parent, or origin `local`/`slack`. Wave agents
  (`S/task` ids) and `create_session` children never qualify.
- Comment checks reuse the signed-approval path: `user.login` in owners, `updated_at == created_at`,
  first line `/bosun approve <40-hex head> chat`, marker line parsed for `session=` and `question=`.
- No owners file on the base: the record counts only for the repo's `owner.login` (user repo) or an
  org admin (org repo, membership API). Elsewhere the empty-owners halt stays.
- Precedence: a chat record is an owner approval; signed and GitHub-review approvals keep counting.
  A repo can turn chat off with `[approvals] chat = false` (default true from the release that
  ships it; older pins reject the key, so it follows the usual release ordering).
- Question id: `approve` list mode mints `<pr>-<short head>-<unix time>`; stored only in the
  PostToolUse cache. The hook hardening is in the first cut for AskUserQuestion; Claude Tag relies
  on `--slack-user` matching an owner's Slack id, as `bosun answer` does today.
- No-CI: bosun posts nothing as a status; its summary comment is the record, and
  `approve`/`main-open` read the local checks instead of statuses.
- `[ci] provider = "none"` also turns off main-health and the ci-minutes gate.
- User-scope hooks: setup-snippet merges by exact command string (idempotent); the hook entry
  points detect a second run in the same tool call via the lease cache and no-op.
  session-start in an un-adopted repo skips the registry and kill-switch calls.
- Hold: recorded in the cache with the head; not asked again for that head. A new head re-asks.
- Sequencing: bosun PRs may land before the decisions PR; the chat form only counts once a release
  that knows it is pinned, and the decisions PR should merge first.
