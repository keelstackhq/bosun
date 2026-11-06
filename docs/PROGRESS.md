# Progress

## Status

Seed complete: S1-S10 done (plan `docs/superpowers/plans/2026-10-22-bosun-seed.md`). The seed review
(2026-10-22) found 1 blocker, 9 majors and 3 minors (issues #20-#32, label `review-finding`); fixes
A-E closed them. bosun v0.5.0 is released (npm `@keelstackhq/bosun` and platform packages,
crates.io `keel-bosun`, GitHub Release) and pinned in `.bosun/version`; gate CI installs the release
pinned in each PR's base commit and checks a PR's own pin.

Open follow-ups: a `bosun result` command for child results (D14); the merge queue (M3); F12, work
outside the plan has no `bosun pr` path (proposal: `bosun pr --no-task` for `docs/**` and
`plans/**` diffs, bookkeeping record, no lease or state). F7 stays: splice update PRs change
governance paths, so each needs `/bosun approve <sha>`. A building, review or fixing issue closed
without a merge costs one PR listing per `reap`; a PR linked only by the bosun marker is seen by
`bosun pr`, not `reap` or `result`. Label-lock misses: spec §4.

History (one line per task) is in `docs/progress-done.md`; agents do not read it.

## Blocked

Human-only items, in order. Each has the exact action.

1. **Repo settings (checklist steps 3 and 5).** After the S10 PRs merge, from a checkout of main
   with an admin token: `hoist plan --area github`, then `hoist apply --area github --yes`. It
   sets auto-merge, delete-branch-on-merge and the required checks `ci-ok`, `risk-check`,
   `review-verdict` and `main-open` from `hoist.toml` (no merge queue yet, see Status). From
   then on every PR needs a `review-verdict` status: post it with `bosun verdict post`.
   Create the kill-switch label: `gh label create bosun:halt --repo keelstackhq/bosun --color
   B60205` (an owner's open issue with it halts bosun). `BOSUN_HALT` is optional; a token that
   cannot read it (403) gets no signal from it.
2. **Cloud environment setup script (checklist step 6, D23).** claude.ai/code → environment
   settings → setup script: paste `docs/setup-script.sh` (again after each pin bump: it now
   installs `@keelstackhq/bosun@0.3.0` from npm), in every environment, the owner's included.
3. **Routines (checklist step 11).** `bosun doctor` prints the three prompts in
   `data.routines`: janitor (every hour, Haiku), curator (daily, Sonnet), notify (every
   hour, Haiku; only with `[slack]`). Create them in claude.ai → Routines.
4. **Workflow secrets into main-only environments (PR fix/secrets-environments).** A same-repo
   PR's own `on: pull_request` workflow can read repository secrets, so each secret moves into
   an environment that only `main` may deploy to. Until then the jobs still read the repository
   secrets (a job's environment falls back to them), so nothing breaks in between. Do the steps
   in order. **Do step 1 before this PR merges:** release.yml runs on every push to `main`, the
   merge included, and with `BOSUN_RELEASE=enabled` that run creates `release-pr` without
   protection (splice.yml does the same with `splice-bot` on its next run, Monday 06:17 UTC). An
   environment can exist before any workflow names it. Steps 2 and 3 can follow at any time.
   1. Create `release-pr` and `splice-bot` with deployment branches = selected → `main`. Run
      again, the `POST deployment-branch-policies` call fails harmlessly when the policy exists:
      ```bash
      for e in release-pr splice-bot; do
        gh api -X PUT repos/keelstackhq/bosun/environments/$e --input - <<EOF
      {"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
      EOF
        gh api -X POST repos/keelstackhq/bosun/environments/$e/deployment-branch-policies \
          -f name=main -f type=branch
      done
      ```
   2. Move each secret into its environment (a new value: a secret cannot be read back):
      `gh secret set SPLICE_BOT_TOKEN --env splice-bot --repo keelstackhq/bosun`, and
      `gh secret set HOIST_RELEASE_TOKEN --env release-pr --repo keelstackhq/bosun` if you use it.
   3. Delete the repository-level copies: `gh secret delete SPLICE_BOT_TOKEN --repo
      keelstackhq/bosun` (and `HOIST_RELEASE_TOKEN` if it exists). `gh secret list --repo
      keelstackhq/bosun` then shows neither.

   The splice bot token itself stays optional (D-004): without it splice.yml opens the weekly
   `splice/update` PR with `GITHUB_TOKEN`, and that PR runs no CI, so `ci-ok` never reports on
   it. Token: fine-grained, keelstackhq/bosun, Contents and Pull requests: write.
5. **Trusted source for the gates (owner decision, B2 designed in spec §15).** Every workflow
   posts statuses as the github-actions app. `risk.yml` no longer runs a PR's copy (the
   `pull_request_review` trigger is gone; an owner's GitHub review is honoured on the next run: a
   push, a `/bosun recheck` comment or a re-run; `/bosun approve <sha>` and signatures work as
   before). But any `on: pull_request` workflow that a same-repo PR adds or edits (ci.yml or a
   new file) still gets a token that can post commit statuses, so it can post a forged
   `risk-check` success on its own head. R-ci does not stop this: the owner gate is that status.
   Until B2 lands only same-repo push access limits it. The close (spec §15): check runs of a
   dedicated GitHub App, its key in a `main`-only environment, the ruleset pinned to the App.
   Owner steps, in order H1-H4, H6, H7, H5 of
   `docs/superpowers/plans/2026-10-28-app-checks-team-owners.md` (App, key, environment,
   variable and secret, release, agent credential check, then `hoist apply --area github`).
6. **Review mode b (M8): off, no API key.** The owner decided (2026-10-24) not to add an Anthropic
   API key. Reviews stay mode a: a session runs `bosun review-pack`, posts the verdict with `bosun
   verdict comment`, and `verdict.yml` applies it. `.github/workflows/review.yml` (PR #183) stays
   dormant: the job still runs on eligible PRs and skips, setting no status and making no API call,
   while `[review] mode = "subagent"`. There is no owner step.
   UC-M8-3 (#145) stays blocked and is not planned: recording mode b needs the key.
7. **Pending UC tests in CI (#179): done.** The owner chose (b): no pending UC tests; UC tests
   land with their code (PR #199). Nothing left to do.
8. **Slack (M6).** The code is on main; bosun holds no Slack token (S1). These steps turn it on.
   - Cut a release that contains `notify`, `answer` and `spawn --slack-thread` and pin it in
     `.bosun/version`, and set `BOSUN_VERSION` in `docs/setup-script.sh` to the same version in
     that PR (the script does not read the pin); then paste the script again (item 2). The pinned
     v0.1.0 has none of them.
   - Create a private Slack channel that is not shared with another workspace or organisation and
     has no guests. bosun refuses to post when the caller's channel facts say otherwise, but it
     cannot prove them (accepted residual, spec §10).
   - Connect Claude Tag (Claude in Slack) to the workspace and invite it to that channel. Its
     cloud environment needs the setup script. No PAT and no `BOSUN_ORIGIN`: GitHub comes
     through the Claude GitHub App (grant the repo to Claude Tag). Each session runs
     `bosun join --origin slack --slack-channel C… [--slack-thread <ts>]`; `bosun doctor`
     step `claude-tag` checks it and warns about a PAT in the environment.
   - Set `[slack] channel = "C…"` in `.bosun/bosun.toml` with an R-gov PR. The defaults are
     `spawners = "owners"` and `may_spawn = true`; set `may_spawn = false` if threads must not
     spawn, and `notify = [...]` to post only some kinds (empty means all).
   - Add `slack = "U…"` to your `[[owner]]` table in `governance/owners.toml` (R-gov). Without it
     no answer counts and no thread may spawn; `bosun doctor` warns.
   - Create the notify Routine (item 3), or replace its prompt, with the prompt in `bosun doctor`
     data.routines from the new release, and attach the Slack connector to it: the Routine reads
     the channel facts and posts with its Slack tool.
9. **Owner approvals.** The default (D-012): the session asks the owner with a form in the
   Claude Code chat; on approve the orchestrator posts the signed `bosun approve` comment
   (`bosun approve <pr>@<head> --sign-key <pem>`; details below). Fallbacks: the owner's own
   comment `/bosun approve <head sha>` (the full 40-character SHA, typed exactly, nothing else,
   never edited: an edited comment never counts, post a new one) or a GitHub review approving
   that head, sent with the owner's own credentials. A newer review of yours requesting changes
   on that head cancels them. Two fallback ways work:
   - the GitHub web or mobile app: type the comment on the PR (a review works only on a PR you
     did not open; agent PRs are opened as your login, so use the comment);
   - `bosun approve` on the owner's own machine (a local shell or a local Claude Code session,
     also when driven through Remote Control), with `BOSUN_OWNER_TOKEN` set there: a
     fine-grained PAT of the owner for keelstackhq/bosun with Pull requests and Issues
     read/write (Commit statuses read/write only for `--main-open`; reads of this public repo
     need nothing more, a private repo also needs Contents, Commit statuses and Checks read).
     Run `bosun approve --list`, check the ready list (PR, title, risk, files), then run
     exactly `data.approve_command` (`bosun approve 199@<sha> 200@<sha>`). A PR whose head
     moved in between is refused (exit 3) and needs a new `--list`.
   Never from a Claude cloud session, not even with a PAT: its agent proxy rewrites every GitHub
   write (app attribution and a Claude footer), so risk-check ignores it, and `bosun approve`
   refuses there (exit 5). The Claude GitHub tool does the same. Never put the PAT in a cloud
   environment.
   The signed approval (spec §8): the orchestrator, on your answer to the form, runs `bosun approve <pr>@<head> --sign-key <pem>` for the head you approved, and the signature from the key in
   `.bosun/approvers.toml` on main counts whoever posts it. Approving the PR that adds that file
   on GitHub is the enrollment; rotate a key the same way. It counts in CI once `.bosun/version`
   pins a release with this feature (cut a release, then bump the pin).
   Delegated decisions (spec §8) need the same release: risk.yml and verdict.yml run the pinned
   release, and only new verdict records carry the `major` count a delegate needs.
10. **New repos with bosun (M7, after v0.2.0 is released).** Per repo: create it with one commit
    and the Claude GitHub App; in a session with splice, bosun and a `GITHUB_TOKEN` (push and
    pull requests), `bosun init --stack rust --owner <login> [--plan plan.md]`; review and merge
    the setup PR by hand (its base has no bosun workflows, so risk-check and review-verdict
    cannot post: a one-time bootstrap); `hoist apply --area github --yes` from a checkout of main
    with an admin token; `gh label create bosun:halt --color B60205`; the secret
    `SPLICE_BOT_TOKEN`; check `governance/owners.toml` and `.github/CODEOWNERS`; paste
    `docs/setup-script.sh` into the cloud environment; create the Routines from `bosun doctor`.
    In a hull template-rust repo, the two bosun hooks come from hull's `.claude/settings.json`
    (hull#22); until then init lists them as an owner step.
    A private repo on a free plan has no rulesets: the required checks are advisory and
    auto-merge (D13) cannot work, so the owner merges by hand; make the repo public or use a paid
    plan (`bosun doctor` says "cannot verify" then). Each weekly splice update PR is R-gov and
    needs `/bosun approve <sha>`. In a Claude cloud session `bosun init` pushes to
    `claude/bosun-init` (or `--branch`).
11. **CircleCI weekly gates (circleci-only consumers).** With the channel pieces `circleci` and
    `circleci-only`, main-health has no weekly GitHub run: in the repo's CircleCI project settings →
    Triggers, add a scheduled pipeline on `main`, weekly (for example Monday 03:17 UTC), so the
    bosun workflow runs every gate on all of main; its `ci-ok`/`bosun-ok` status feeds main-health.
