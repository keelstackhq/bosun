## 16. Owner teams (B4)

Status: designed, not built (plan `docs/superpowers/plans/2026-10-28-app-checks-team-owners.md`,
item B4, after B2's App exists).

**Bus factor.** One human owner is a bus-factor-1 risk. When the only owner is unavailable,
nothing that needs an owner merges: high-risk PRs stay red, a delegate cannot stand in on the
critical floor (§8), and an owner's open halt issue stays open. That is the intended fail-closed
behaviour, not a fault. List at least two owners, or a team.

**Schema.** `governance/owners.toml` takes team entries beside user entries:

```toml
[[owner]]
github = "urbanisierung"
slack = "U012345"

[[owner]]
team = "keelstackhq/maintainers"
```

Each `[[owner]]` has exactly one of `github` or `team` (both or neither is a parse error naming
the table). `team` is `<org>/<slug>`, and `<org>` must be the repo's owner (case-insensitive):
another org's team is a parse error, since the App cannot read it. `slack` belongs to a user
entry only. The file is R-gov and a trust file (§8) as today. Unknown keys are errors, so a
release that knows `team` is pinned before a team entry lands.

**Expansion.** At check time `GET /orgs/{org}/teams/{slug}/members?per_page=100`, every page.
GitHub's docs say that list includes members of child teams, and a maintainer of any child team
could then make owners. So bosun first reads `GET /orgs/{org}/teams/{slug}/teams`: a team with
child teams fails closed (below; the fix says to list a team without child teams, or the users).
bosun does not read a member's `role`: maintainers and members count alike. Logins compare
case-insensitively. More than 1000 members is an error.

**Which contexts can expand.** The reads need Members read, which `GITHUB_TOKEN` cannot have.
They use `BOSUN_APP_TOKEN` (§15) when set, else the command's own token. Only the gate jobs
(risk.yml, verdict.yml, main-open, main-health's re-judge) get `BOSUN_APP_TOKEN`; sessions never
do. So in session-run commands (the kill switch of every command, `bosun approve --list`, `ask`,
`notify`) and in `bosun approve` with `BOSUN_OWNER_TOKEN`, a team is unreadable unless that
token can read the org's members (an org member's token with `read:org` or Members read).

**Caching.** Within one process, once per `org/slug`, behind the owner reader
(`owner_logins`), so risk-check's several owner reads cost one expansion. Never on disk and never
across runs: a membership change counts from the next run.

**Failure: fail closed.** Child teams, or a 403, 404, error or bad page in either read, means no member of that
team counts, never an error that passes. User entries still count. The command says so:
`data.owner_teams_unreadable` lists the teams, one stderr line, and where an owner was needed
the fix is "give the bosun App Members read and pass BOSUN_APP_TOKEN, or list the owner by login
in governance/owners.toml". The status text appends `(team <org>/<slug> unreadable)`.
Withdrawals fail closed too: while any team is unreadable, a CHANGES_REQUESTED or DISMISSED
review on the head by any login that is not a listed user, and whose `author_association` is
OWNER, MEMBER or COLLABORATOR (a team member is at least MEMBER; an outsider on a public repo
cannot block), counts as an owner's withdrawal (§8: it blocks what any owner's withdrawal
blocks, such as verdicts, delegates, signed approvals and a carried approval, `anywhere`
included), since its author may be a member. A listed user's own approval still counts, as with
any other owner's withdrawal. Residual: GitHub may report a member whose org membership is
private as CONTRIBUTOR to a token that cannot see it, so their withdrawal is missed while the
team is unreadable; team members should make their membership public. The kill
switch is the inverse: an open `bosun:halt` issue by a login that is no listed user while a team
is unreadable halts (exit 4), as any error reading the owners does today. Since sessions usually
cannot read a team, a halt issue by any non-user then halts sessions: list at least one owner
by login so an owner's own halt issue is judged without the team.

**Where owners count.** Everywhere a login is checked against the owners, an expanded member is
an owner:
- GitHub reviews: a member's APPROVED review of the head approves; a member's CHANGES_REQUESTED
  or DISMISSED review withdraws (§8), so any member can block.
- `/bosun approve <head>` comments (the fallback; the default is the chat form and the
  orchestrator's signed comment, D-012): by a member, with the same tests (unedited, plain user
  token, exact text).
- Signed approvals: keys stand for the owners; "no owners at base" means no listed user and no
  readable member.
- `bosun approve` with `BOSUN_OWNER_TOKEN`: the PAT's login must be a user entry or a member
  (the PAT then needs organization Members read); otherwise exit 5 with the fix above.
- `bosun ask` and `notify` mention users and `@<org>/<slug>`; Slack IDs come from user entries only.

**Residual.** Team membership is set in the org, not in a PR: an org owner or a maintainer of
the team who adds someone makes them a bosun owner without an R-gov PR. Use a team without child
teams whose maintainers are the owners themselves.

**Tests (fake GitHub).** Parse: user, team, both, neither, `slack` on a team, another org.
Expansion: two pages, case; a team with a child team counts no members. One members call per run with several owner
reads. Approvals: a member's review, comment and request for changes; a non-member's do not count.
Failure: 403 and 404 on either read give no team members, `data.owner_teams_unreadable` and the fix; a listed
user still approves. With the team unreadable, a non-user's CHANGES_REQUESTED or DISMISSED
review by a MEMBER withdraws (two verdicts present: the check fails); a CONTRIBUTOR's or NONE's
does not. Kill switch: a member's halt issue halts; an unreadable team and a
non-user's halt issue halt. Without `BOSUN_APP_TOKEN` and with a session token that cannot
read members, `approve --list` and `ask` report the team unreadable. `bosun ask` mentions the team.
