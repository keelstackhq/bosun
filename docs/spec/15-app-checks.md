## 15. Trusted check source: App check runs (B2)

Status: B2 built (plan `docs/superpowers/plans/2026-10-28-app-checks-team-owners.md`): the
check-run client, `[checks]`, the gate writer, reading back by App, the checks-mode refusals and
the doctor pin check. The workflows (B2W) are not switched yet. The default mode is `statuses`,
so nothing changes until a repo sets `[checks]`. Until the ruleset switch (Transition, step 6), §7, §8 and §9
describe what runs.

**The problem.** `risk-check`, `review-verdict` and `main-open` are commit statuses that the
workflows post with `GITHUB_TOKEN`, as the github-actions app (D32). Any `on: pull_request`
workflow that a same-repo PR adds or edits runs the PR's copy of that file and gets a
`GITHUB_TOKEN`; with `permissions: statuses: write` (or `checks: write`) in the file, it posts
`risk-check` success on its own head. A required check accepts a status or check run from any
source unless the ruleset pins the source, so a statuses-based gate can be forged by anyone who
can push a branch. Dropping `pull_request_review` from risk.yml closes one path, not this one.

**Builds on B1.** B1 (#312) removed `pull_request_review` from both risk.yml files, so
risk.yml runs only on `pull_request_target` and `issue_comment`, both from the default branch.
That is a precondition here: a `pull_request_review` job runs the PR's copy and evaluates the
environment against `refs/pull/<n>/merge`, so with `bosun-app` it would fail. An owner review (or
a withdrawal: request changes, dismiss) is judged on the next run: a push, a `/bosun recheck` PR
comment, or a re-run. The trade-off: an owner's request for changes or dismissal takes effect
only on that next run; until then a green risk-check from verdicts or a delegate can still
auto-merge, so an owner who withdraws also comments `/bosun recheck`. Optional (B2W/T5, the
owner's choice): a relay workflow on `pull_request_review` with `permissions: {}` and a keyed
`workflow_run` job that finds the PR by `head_sha` through the API, trusting no payload (UC-B2W-4).

**D33: the gates are App check runs, pinned in the ruleset.**

The App. One per org (or user account) that owns the repos, created by the owner (§14): for
example `keelstackhq-bosun-gate` (a slug is global). No webhook (Active off), no callback or setup
URL, "Only on this account", installed on the selected repos only. Repository permissions: Checks
read and write; Pull requests read and write (the verdict record comment); Contents read; Issues
read; Metadata read (mandatory); Commit statuses none; nothing else. Organization permission:
Members read (§16 teams; a user account has no teams and skips it). Only the gate jobs ever use
this App; any other bot App (the splice bot of B5, §17) is a separate App with its own key.

Which token writes what. The App token writes the three check runs, the verdict record comment and
the team reads. `GITHUB_TOKEN` keeps the Risk note body edit, the labels and the other comments:
a write with an App token starts workflows (a `GITHUB_TOKEN` write does not), and an App body
edit would start risk.yml again (`edited`). An App comment therefore never contains
`<!-- bosun:verdict -->`, `/bosun approve` or `/bosun recheck` (which would start a keyed
re-run and could loop), so no `if:` of ours matches it (a test asserts it;
a skipped job uses no runner).

Check runs. `name` is the context, `head_sha` the head judged, `output.title` the description
bosun posts today (so `bosun approve --list` reads the same text), `output.summary` that text plus
the matched rules. `pending` creates the run with `status: in_progress`; the verdict PATCHes that
run to `completed`. `success` is `conclusion: success`; `failure` and `error` are
`conclusion: failure` (an error's title starts `error:`). Never `neutral` or `skipped`: both
satisfy a required check. A newer run of the same name and App on the sha supersedes the older;
readers take the newest by id. The Re-run button sends `rerequested` to the App, which has no
webhook, so nothing happens: fix texts keep B1's wording ("comment `/bosun recheck` (or re-run
the risk workflow)"; for main-open and review-verdict, re-run that workflow).

**Source pinning.** The `main` ruleset's `required_status_checks` lists
`{context: "risk-check", integration_id: <App id>}`, and the same for `review-verdict` and
`main-open`. A status (from any token) or a check run from github-actions or any other App then
never satisfies them. `ci-ok` stays pinned to GitHub Actions (`integration_id` 15368): it must
come from a workflow, but any workflow counts, the PR's rewritten ci.yml or a new file with a
job named `ci-ok` included. So `ci-ok` means "the PR's own CI passed", never more. risk-check
covers the rest: such a PR edits `.github/workflows/**`, which is R-ci and on the critical floor
(§8), so it needs the owner, and that owner gate can no longer be forged. ci.yml grants no write
scope, so PR code it runs (tests, build scripts) cannot post either.

**The key lives in an environment.** Jobs get the token with `actions/create-github-app-token`
(pinned by full sha; `client-id: ${{ vars.BOSUN_APP_CLIENT_ID }}`,
`private-key: ${{ secrets.BOSUN_APP_KEY }}`, `permission-*` inputs downscope it per job). The key is a secret of the environment `bosun-app`
only, never a repo or org secret (those reach a same-repo PR's `pull_request` workflow). The
environment's deployment branch policy is "Selected branches and tags" with exactly `main`, no
tags, and no other protection rule (no required reviewers, no wait timer): the gate jobs run
on every PR event and comment, so any rule would hold each run. Jobs say
`environment: { name: bosun-app, deployment: false }`. The token step has
`continue-on-error: true`: a missing or broken key never stops the gate job, so `statuses` mode
ignores it, `dual` skips the check run (one stderr line) and `checks` exits 1 (no token).

Why it holds: GitHub gives environment secrets only to a job whose ref passes the policy. Since
2025-12-08 (GitHub changelog 2025-11-07, "Actions pull_request_target and environment branch
protections changes") `pull_request_target` always runs the workflow file and checkout commit of
the default branch, `GITHUB_REF` is the default branch, and environment rules evaluate against
the default branch; `pull_request`, `pull_request_review` and `pull_request_review_comment`
evaluate against `refs/pull/<n>/merge`, which never matches `main`. `issue_comment` and
`workflow_run` run on the default branch. A `push` or `workflow_dispatch` on a branch evaluates
that branch, and only the ruleset's merges reach `main`. So a PR's own job that names `bosun-app`
fails before its first step, and one that does not gets no key.

The `pull_request_target` ref, precisely. `GITHUB_REF` is the default branch, not the PR's base,
and `github.sha` is the default branch's tip, not `pull_request.base.sha`. A PR into another base
still runs main's file and passes the `main` policy. risk.yml keeps checking out
`pull_request.base.sha` (the rules that judge a PR are its base's) and keeps `branches: [main]`
(older GitHub Enterprise Server, without the change, is out of scope). A job holding the key
never checks out or runs PR code (`tests/workflows.rs`, for every job with the environment); the
key goes only to the token step, the token only to the bosun step. main-health's re-judge job
(§9) gets the environment, its test job does not.

**bosun changes (planned).**
- `.bosun/bosun.toml` `[checks]`: `mode = "statuses" | "dual" | "checks"` (default `statuses`),
  `app_id` and `app_slug` (required unless `statuses`; never GitHub Actions, id 15368 or slug `github-actions`, which any workflow can post as). Workflows read it from the base like the
  rest; unknown keys are errors, so a release that knows `[checks]` is pinned first.
- The token comes from env `BOSUN_APP_TOKEN` (never argv, never printed). Missing in `checks`
  mode: exit 1, the fix names the token step and the `bosun-app` environment. In `dual` the status
  is posted, the check run is skipped with one stderr line and `data.checks_skipped: true`.
- `src/github`: `create_check_run(GateRun) -> id` and `update_check_run(id, GateRun)` (name,
  head_sha, state `Pending | Success | Failure`, title, summary cut to 65,535 characters) on a
  second client built with the App token; `CheckRun` gains `app_id`; `check_runs_all(sha)` reads
  `filter=all`, paginated, so readers see every run per App (`check_runs(sha)` keeps GitHub's
  default, the newest per name and App, for the readers of `ci-ok`).
- One gate writer (`src/github/gate.rs`, `GateWriter::post(sha, context, state, description)`)
  posts a status, a check run or both by mode. Every post of the three contexts moves to it:
  risk-check (pending, verdict, error, the verdict carry), `verdict apply`/`post`, `gate
  main-open` (and so main-health), `review-ci`. The pending run is the one the verdict PATCHes
  (same process); risk-check's summary lists the matched rules. In `dual` a failed check-run
  write is one stderr line, never an error (the statuses are the authority); in `checks` it is
  exit 1. `bosun approve --main-open` stays on statuses until T5.
- Reading the gate's own state: in `checks` mode the newest check run of that name whose `app_id`
  is `[checks] app_id`, statuses ignored; in `statuses` and `dual` the statuses, as today, since
  they are the authority until the switch. Readers: risk-check (pending repair, carry), `bosun
  approve --list`, `review-pack` (`since_sha`), `bosun pr --auto-merge` (in `checks` mode a gate
  context counts only as a check run of the App and `ci-ok` only from github-actions; in
  `statuses` and `dual` a status or check run, as today). `Ctx::load` records the trusted App
  once per process; `gate_statuses` and `gate_history` (`src/github/gate.rs`) give the App's runs
  as statuses (title as the description, an `error:` title as `error`, an open run as `pending`),
  so the readers keep their logic. `bosun pr --auto-merge` reports the gates from them.
- Verdict records: in `checks` mode `verdict apply` writes the record with the App token, and
  only records by `<app_slug>[bot]` count (two verdicts, delegate, carry): the readers see that
  login as the trusted author and `github-actions[bot]` as nobody. The App's record body is
  defused (`<!--` and `/bosun` are broken up in the text; the record line is JSON-escaped, `\u002f` and `\u003c`, so it reads back the same) so it can never start a keyed job (UC-B2-7). This closes the §8
  residual "a workflow can post a record as github-actions[bot]". Older records stop counting at
  the switch; an open PR's reviewer posts the verdict comment again.
- `bosun verdict post` (a session's own token) cannot make the App's check run: in `checks` mode
  it is exit 1 with the fix `bosun verdict comment`. `bosun approve --main-open` (the owner's PAT)
  cannot either: in `checks` mode it is exit 3 with the fix "re-run the main-open workflow" (O4).
- `bosun pr` and `doctor` read hoist.toml `required_checks` entries in both forms (string or
  table, below); a `required_checks` key that cannot be read is a step-3 problem in `doctor` and
  a stderr line in `bosun pr` (it would otherwise fall back to the defaults silently). `doctor
  --remote` warns in `checks` mode (step 3, `warn`) when a gate context is not pinned to `app_id`
  in the rulesets (`GET /rules/branches/<b>`, `integration_id`), and when the session's token can
  list the Actions secrets (a sign it may hold the writes; no read proves a write scope is
  absent, so the precondition below stays an owner check). The rulesets' bypass list is not
  read: GitHub shows it only to admins, whom the secrets read already flags.
- Tests use the fake GitHub only: check-run create and update, `app_id` on runs, a same-name run
  from github-actions that must not count, each mode, the missing token.

**hoist (a change to request in the hoist repo, not done here).** `[github] required_checks`
accepts tables beside strings: `{ context = "risk-check", app = "keelstackhq-bosun-gate" }` or
`{ context = "ci-ok", integration_id = 15368 }`. hoist resolves `app` with an authenticated `GET /apps/{slug}`
(a private App needs it) to its id, writes `integration_id` into the ruleset, and `hoist check` reports a missing
or different source as drift.

**Transition.** Each step is its own PR or owner action:
1. Owner step H6: a bosun release with the check-run client, `[checks]`, dual posting and the
   hoist.toml table form; the pin moves to it.
2. Owner steps H1-H4 (§14): the App, its key, the environment, the variable and secret.
3. With B1 merged (risk.yml has no `pull_request_review` trigger), the `pull_request_target`,
   `issue_comment` and keyed `workflow_run` jobs of the workflows and channel copies get the
   environment and the token step; `[checks] mode = "dual"` with the App's id and slug. Both
   kinds post; the ruleset still requires statuses. Reviews are judged on the next run.
4. Verify: check runs of the App appear; a probe branch whose `pull_request` job names
   `bosun-app` fails (O1).
5. hoist ships pinning; hoist.toml lists the four checks with their sources. hoist.toml is
   R-gov and on the critical floor by then (B2/T6), so no agent PR can unpin a check.
6. Owner step H5: `hoist apply --area github` switches the ruleset (now the check runs count).
7. `[checks] mode = "checks"`: statuses stop. The same PR adds `permission-pull-requests: write`
   to verdict.yml's token step (in checks mode the App writes the verdict record comment; dual
   asks only `permission-checks: write`). Never before step 6: an unpinned ruleset would wait
   for statuses that no longer come, and every PR would block. Before it, a forge probe: a
   probe branch's `pull_request` job posts a `risk-check` check run and status with
   `GITHUB_TOKEN` (success), and its PR must stay blocked. That proves the pin, and O6's
   15368, live.

Rollback and recovery. Undo step 7 with a PR that sets `mode = "dual"` (it passes while the
check runs still post). Undo step 6 with `hoist apply` from hoist.toml with plain string entries
(unpinned); in `dual` the statuses then satisfy it at once. When main breaks the gates themselves
(main-open.yml, risk.yml or the pinned release fails, or the key is lost), every PR is blocked,
the fix PR included, and `bosun approve --main-open` cannot help in `checks` mode. Recovery is
an owner action: merge the fix PR by hand through the ruleset's bypass (an owner merge, which
D-006 allows; agents never merge), or unpin with hoist (undo step 6) and, in `checks` mode,
merge the `mode = "dual"` PR by bypass first, since no status is posted.

**Option, not adopted: code-owner review of `.github/**`.** The ruleset could set
`require_code_owner_review` with CODEOWNERS covering `.github/**` (it already names the owner
there), so every workflow edit needs a code owner's GitHub review whatever bosun says. Cost:
every workflow PR (each weekly splice update included) needs a human click, and only a GitHub
review counts: `/bosun approve` comments and signatures (§8) do not satisfy it. Agent PRs are
opened as the owner's login and GitHub forbids approving your own PR, so with one owner every
workflow PR would need a bypass, which is a human merge outside auto-merge (D13 and D-006 stay:
agents never merge). With B2 in place risk-check already needs the owner for these PRs and cannot
be forged. Revisit when at least two humans own the repo (§16) and agents open PRs as their own
account.

**Residuals.**
- A repo admin can change or delete the ruleset, the environment or the App. That is the owner's
  authority, not a bypass.
- A leaked App key (or token) forges all three gates until the key is revoked. The key is only in
  the environment and only the token step sees it; the jobs run actions and the npm release, so a
  compromised action or package in those jobs could take the token (pin actions by sha).
  Rotation: generate a new key in the App settings, `gh secret set BOSUN_APP_KEY --env
  bosun-app < new.pem`, re-run a gate workflow and see its check run, then delete the old key.
- hoist.toml holds the pins (and today the required checks), but no risk rule or floor covers
  it (`.bosun/risk.toml`, `src/core/risk_default.toml`, `CRITICAL_PATHS`), so an agent PR that
  drops a check or unpins it is normal risk, and the next owner `hoist apply` undoes it. That
  gap exists today, without B2. B2/T6 closes it, and can land at once: hoist.toml becomes R-gov
  and critical (owner, never delegate), here and in the channel (`hoist.toml.jinja` is the
  consumers' copy, `.bosun/risk.toml.jinja` their rules).
- Precondition: agents act as the owner's login, so the agent's GitHub credential must hold no
  Administration, Environments or Secrets write; else an agent can widen the environment's branch
  policy, add a repo secret, or edit the ruleset. An owner step (H7) checks it (the Claude
  GitHub App's permissions are set by its publisher: narrow its repos, move agents to a
  fine-grained token without those scopes, or accept and record the risk); `doctor --remote`
  warns where it can (above).
- A fork's PR gets no secrets in its own `pull_request` jobs, but its `pull_request_target` jobs
  do get the environment key, as every PR's do. That is safe only because those jobs run main's
  file and never check out or run PR code.
- main-health issues, CI-log and other non-gating statuses stay on `GITHUB_TOKEN`: out of scope.

**Open questions.**
- O1: GitHub's docs on `deployment: false` confirm that required reviewers still apply but do not
  say so for deployment branch policies. Step 4's probe settles it; if the policy is not applied,
  drop `deployment: false` (each run then records a deployment).
- O2: with several check runs of one name from one App on a sha, the required check is taken to
  follow the newest. Verify in the dual phase before step 7.
- O3: the App gets Issues read only, and Pull requests write only for the record comment; the
  Risk note and the labels stay on `GITHUB_TOKEN`. Moving them to the App would need Issues write
  and would start workflows on each write.
- O4: why `--main-open` exists (a PR that changes main-open.yml) and whether the base workflow
  judging it under `pull_request_target` now suffices; recovery when it breaks is above.
- O5: channel copies take the environment only where the App is set up (a splice template var:
  an empty `environment:` name is not documented as "none"); others stay in `statuses` mode.
- O6: 15368 as the GitHub Actions integration id comes from ruleset exports, not a GitHub doc:
  read it from the live ruleset before step 6.
