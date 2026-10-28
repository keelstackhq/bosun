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
GitHub's docs say the list includes members of child teams, so nested teams count; a repo that
wants direct members only lists users. bosun does not read the `role` of a member: maintainers
and members count alike. Logins compare case-insensitively. The read needs Members read, which
`GITHUB_TOKEN` cannot have: it uses `BOSUN_APP_TOKEN` (§15) when set, else the command's token
(a session or owner token of an org member may read it). More than 1000 members is an error.

**Caching.** Within one process, once per `org/slug`, behind the owner reader
(`owner_logins`), so risk-check's several owner reads cost one expansion. Never on disk and never
across runs: a membership change counts from the next run.

**Failure: fail closed.** A 403, 404, error or bad page in the expansion means no member of that
team counts, never an error that passes. User entries still count. The command says so:
`data.owner_teams_unreadable` lists the teams, one stderr line, and where an owner was needed
the fix is "give the bosun App Members read and pass BOSUN_APP_TOKEN, or list the owner by login
in governance/owners.toml". The status text appends `(team <org>/<slug> unreadable)`. The kill
switch is the inverse: an open `bosun:halt` issue by a login that is no listed user while a team
is unreadable halts (exit 4), as any error reading the owners does today.

**Where owners count.** Everywhere a login is checked against the owners, an expanded member is
an owner:
- GitHub reviews: a member's APPROVED review of the head approves; a member's CHANGES_REQUESTED
  or DISMISSED review withdraws (§8), so any member can block.
- `/bosun approve <head>` comments: by a member, with the same tests (unedited, plain user
  token, exact text).
- Signed approvals: keys stand for the owners; "no owners at base" means no listed user and no
  readable member.
- `bosun approve` with `BOSUN_OWNER_TOKEN`: the PAT's login must be a user entry or a member
  (the PAT then needs organization Members read); otherwise exit 5 with the fix above.
- `bosun ask` and `notify` mention users and `@<org>/<slug>`; Slack IDs come from user entries only.

**Residual.** Team membership is set in the org, not in a PR: an org owner or a team maintainer
who adds someone to the team makes them a bosun owner without an R-gov PR. Use a team whose
maintainers are the owners themselves.

**Tests (fake GitHub).** Parse: user, team, both, neither, `slack` on a team, another org.
Expansion: two pages, a child-team member, case. One members call per run with several owner
reads. Approvals: a member's review, comment and request for changes; a non-member's do not count.
Failure: 403 and 404 give no team members, `data.owner_teams_unreadable` and the fix; a listed
user still approves. Kill switch: a member's halt issue halts; an unreadable team and a
non-user's halt issue halt. `bosun ask` mentions the team.
