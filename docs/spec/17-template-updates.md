## 17. Clean template updates

**Status: proposed, not built (design B5, 2026-10-28).** Nothing below is in the code. risk-check
works as §8 says until the owner decides the open choices at the end and the plan
`docs/superpowers/plans/2026-10-28-clean-template-update.md` lands. Until then "a rule only
escalates" (§8) holds without exception. §15 (App check runs, B2) is a hard prerequisite: this
design merges after the B2/B4 design and is built only after §15 is built.

### Problem

`.github/workflows/splice.yml` opens one `splice/update` PR a week from a bot
(`SPLICE_BOT_TOKEN`). It changes the splice lock (`.splice/channels/<name>.toml`, R-gov) and the
files the channel renders: here `docs/agents/org-decisions.md` and a `CLAUDE.md` section; in a
consumer of the bosun channel also `.bosun/version`, workflows and `.claude/settings.json`. So
each update is high risk and waits for the owner, also when it only moves a pin.

### The criterion

A PR is a *clean template update* only when all of C1-C7 hold on its current head. risk-check
decides it with the rules, owners and config read at base (§8), never at the head.

- **C1 Lock diff.** The PR changes at least one lock file `.splice/channels/*.toml` and adds,
  removes or renames none. bosun parses the base and head copy of each changed lock. With
  `template.version`, `template.commit` (or `rev`) and the derived `[files]` and `[sections]`
  tables removed, both documents are equal (`format`, `splice`, `[channel]`,
  `template.source`, `[vars]`, `[[pieces]]`). `template.source` is byte-equal to an entry of the
  allowlist in `.bosun/risk.toml` at base:
  ```toml
  [template_update]
  sources = ["gh:keelstackhq/decisions//channel"]
  bot = "<app-slug>[bot]"          # C5
  paths = [".splice/channels/decisions.toml"]   # the only paths the class may lower
  splice = "0.2.0"                 # the splice release that re-renders (pinned at base)
  splice_sha256 = "<hex>"          # of that release's linux-x64 binary
  ```
  A source string is compared exactly, owner and repo lowercased; a source the API reports
  under another `full_name` (a renamed or moved repo) is not on the list. The derived tables
  are checked by C2, not here.
- **C2 Re-render equals the head.** From the base tree, with the base lock changed only in
  `template.version` and `template.commit` (taken from the head lock), the pinned splice
  renders the channel. The resulting git tree (`git write-tree`, file modes included) equals
  the PR head's tree. This covers the lock's derived tables and every rendered file. Any
  hand edit, extra file, mode change or missing file makes the trees differ.
- **C3 No conflicts.** GitHub reports the PR `mergeable: true` (`null`, still computing, is
  not clean), and no added line in any file starts with `<<<<<<< `, `=======` (alone) or
  `>>>>>>> `.
- **C4 Base is in sync.** The pinned splice's `splice check` on the base tree exits 0, so the
  base has no hand drift that a re-render would hide or carry.
- **C5 Author is the bot, by app.** The PR author is the login in `bot`, with `type: Bot`. A
  `<slug>[bot]` login exists only for a GitHub App; no user can register it, and an agent
  (which acts as the owner's login through the Claude app) cannot open a PR as it. The head
  branch is `splice/update` in this repository (never a fork), and every commit of the PR has
  that login as author and committer with `verification.verified: true` (GitHub signs the
  commits an App makes through the API; if splice pushes with plain git, its commits must move
  to the API first). A login check alone is refused: `SPLICE_BOT_TOKEN` is today a
  fine-grained PAT, so its PRs carry a user login that agents share. The bot therefore needs
  its own GitHub App (owner decision 1). Its key is not a repo secret, which every same-repo
  `pull_request` workflow can read, so anyone who can push a branch could mint bot tokens.
  It lives in an environment `splice-bot` with a main-only deployment branch policy, and only
  the splice.yml job names that environment (as §15 keeps the gate App key). A push by anyone
  else to `splice/update` breaks C5, and so does a merge of main by anyone but the bot, the
  update-branch button included: the bot refreshes the PR instead. C2 checks the content
  whoever pushed.
- **C6 Only declared files.** Every changed path, other than the changed lock files, is listed
  in the head lock's `[files]` with class `owned` or `shared`, or as a `[sections]` target.
  `seed` files and paths the lock does not list fail. A changed entry that is a symlink
  (mode 120000) or a submodule (160000) fails. Limit: a deleted path is not in the head lock,
  so a PR that deletes a file is never clean (an upstream removal goes to the owner).
- **C7 The version is published upstream.** `template.commit` is an ancestor of, or equal to,
  the lock's `[channel] track` ref in the source repository:
  `GET /repos/{o}/{r}/compare/{track}...{commit}` has status `behind` or `identical`. "The
  commit exists in the source repo" is not enough: GitHub serves commits of any fork through
  the parent's API. This relies on the source refusing force-pushes to `track` (a ruleset
  there); a force-pushed ref could make an attacker commit an ancestor for a while.
  `template.version` agrees with the commit: `0.0.0+g<first 7 hex>` for a tracked branch,
  else a tag of the source that points at the commit. Options, per source:
  (a) `track` ancestor only (the minimum); (b) a published, non-draft, non-prerelease
  GitHub Release; (c) a tag signed by a key listed at base in `[template_update]`. The owner
  picks (decision 3).

C1-C7 come on top of the §8 limits: R-unseen (bosun could not see the whole diff), a halted
bosun, a PR not targeting the default branch, or any owner's withdrawal on the head (§8) mean
"not clean".

### What it grants

A clean template update gets the approval class `template`: the bot plus green gates (`ci-ok`,
`review-verdict`, `main-open` as `hoist.toml` requires), with no owner and no delegate. It
opts in per path, not per rule: a match is lowered only when every file it matched is in
`[template_update] paths` at base and outside the template floor below. (A per-rule opt-in
would not work: the lock is under `.splice/**`, R-gov, so R-gov would opt in and lower
`REVIEW.md`, which steers verdicts, and `docs/lessons.toml`, which feeds agents.) A floor path
listed in `paths` never lowers, and risk-check logs it to stderr. Any other match keeps its
class, and the strictest class wins as in §8. The status reads `high risk
(<ids>), clean template update: <source>@<7 hex>`, and `data.template_update` holds `{clean,
source, commit, reason}`.

**The template floor still needs the owner, also in a clean update:** the §8 critical floor
(`.bosun/**`, `governance/**`, CODEOWNERS, R-secrets, R-prod-write, R-unseen and, in bosun,
`src/**` and the manifests), every R-ci path (`.github/workflows/**`, `.github/actions/**`,
the other CI configs, `deny.toml`, the toolchain files), `.claude/**`, `REVIEW.md`,
`docs/lessons.toml` and the §8 trust files. Why: C1-C7 prove that
the PR is exactly what upstream published, not that upstream is safe. A compromised or careless
template could ship a workflow with `statuses: write` that posts `risk-check` success for every
PR, a step that leaks secrets, a looser `deny.toml`, or a `.claude/settings.json` hook that runs
commands in every agent session. One bad upstream commit would then reach every consumer with no
human look. So a bosun channel update that moves `.bosun/version` or a workflow still waits for
the owner; the class removes the owner only from pins and plain rendered files (the decisions
channel). Agent-instruction files (`CLAUDE.md`, `AGENTS.md`, `docs/agents/**`) are prompts
for every later agent. In bosun the only rendered files are such files (`org-decisions.md`
and a `CLAUDE.md` section), so the whole benefit here is to skip a human look at files every
agent reads. Default: they stay out of `paths` unless C7 (b) or (c) is chosen for the source
(decision 2); with the default, a bosun decisions update still needs the owner.

### Where the re-render runs

The re-render (C2, C4) runs as a separate job `template` in `risk.yml`, before the `risk` job,
only on `pull_request_target` for PRs whose author login is `bot` and whose `user.type` is
`Bot`, and that change `.splice/**`. Before any git or splice call it checks that
`template.commit` matches `^[0-9a-f]{40}$`. It checks out the base SHA (never the head),
installs the splice release named at base and checks its sha256, fetches the source at
`template.commit`, renders, and runs `splice check`. Its permissions are `contents: read` with
no persisted credentials, no secrets and no App key. It never runs code from the PR or from
the template: rendering is data only (splice must run no piece hooks
or scripts; a channel that needs them is never clean). The job outputs one value, the rendered
tree SHA (or empty on any failure). The `risk` job has `needs: template` and `if: always()`
(with its own conditions), so a skipped or failed render never skips risk-check. It passes
the value to the pinned bosun (`BOSUN_TEMPLATE_TREE`), which compares it with the head tree
read through the API and checks C1 and C3-C7 itself. With the gate App key of §15, risk-check
then posts a per-head `template-render` check run (conclusion `success` or `failure`; the
summary names the base SHA and the rendered tree). Every later risk-check on that head (the
re-run in verdict.yml after `bosun verdict apply`, reviews, comments) reads the newest
`template-render` check run of the gate App, pinned by App id as §15 pins its checks, and
counts it only while its base SHA is the PR's current base. A check run of any other app,
or a commit status of that name, is ignored. Without §15 a later run has no render and no
unforgeable record of one, so the class would flip to "owner needed" as soon as
`review-verdict` lands; that is why §15 is a hard prerequisite. So a fault in the render job
can at most make a PR not clean, except a splice bug that a hostile template exploits to print
the head's tree SHA (a residual).

Accepted liveness limit: any merge to main changes the PR's base SHA and voids the
`template-render` result, so the class falls back to "owner needed" until the bot refreshes
the PR on the new base. Keying the result on the head's merge base would not help: C2 and C4
require that merge base to be the current base, so an old render cannot judge a new base. The
limit fails closed; a `workflow_dispatch` of splice.yml refreshes at once.

Versions: bosun is the release in `.bosun/version` at base (as for every risk-check); splice is
`[template_update] splice` at base, so a PR cannot choose the renderer that judges it. A splice
upgrade is an R-gov PR.

### Cost

One extra job per bot-PR push on `pull_request_target`, about 1 billed minute (download splice,
fetch one source commit, render, write-tree). With one bot push a week that is about 4-8 minutes
a month; a skipped job (other PRs, `issue_comment`) costs nothing (`docs/ci-minutes.md`).

### Failure modes (all fail closed)

Any of these leaves the PR in the class §8 gives it today, with `data.template_update.clean =
false` and a `reason`, never an `error` status: no `[template_update]` table or a parse error
in it; a splice download, checksum, fetch, render or check failure; a job timeout; an empty or
missing tree SHA; a `template.commit` that is not 40 lowercase hex; a tree mismatch; an API
error or a 403; `mergeable` null; a commit not on the `track` ref; no `template-render` check
run of the gate App for this head and base. "Base" in C2 and C4 is the PR's base SHA, and a
head whose merge base is another commit is not clean. The class is not carried over (§8
approval carry-over): a merge of main by anyone but the bot fails C5. When main moves, the
bot's next refresh (the weekly run, or a `workflow_dispatch` of splice.yml) re-renders (the
liveness limit above).

### Interactions

- **R-ci.** Every R-ci path is in the template floor, so a template update that touches CI is
  never lowered, delegate-class R-ci paths included (decision 4 may give those to a delegate).
- **§15, App check runs (B2: a GitHub App posts `risk-check`, `review-verdict` and `main-open`
  as check runs pinned by `integration_id`).** A hard prerequisite (above): the
  `template-render` check run is posted by the keyed risk job, never by the render job, and
  read by App id. The bot App that opens update PRs must never be the gate App that judges
  them (§15 keeps them separate): a leaked bot key must never post `risk-check` or
  `template-render`.
- **Two verdicts and delegates.** The template class never counts as a verdict or a delegate
  approval, and no signature is involved.

### Threat model

- **Compromised template source.** C1-C7 pass by design. The floor keeps workflows, CI, the
  bosun pin, hooks, review rules and governance with the owner; what remains (the files in
  `paths`: pins, rendered docs and, by decision 2, agent instructions) reaches main without a
  human. C7 option (b) or (c) adds an upstream
  human step.
- **Lock pointing at an attacker fork.** A changed `template.source` fails C1; a fork commit
  under the right source name fails C7 (not an ancestor of `track`). The allowlist
  changes only by an R-gov PR at base.
- **Bot token leak.** The attacker can open a PR as the bot but must still pass C1-C7: the
  diff must equal a render of an allowlisted, published upstream commit. They can only push an
  older or newer published version (a downgrade). Proposed check: `template.commit` is a
  descendant of the base lock's commit (decision 5). The App key lives only in the `splice-bot`
  environment (main-only); revoking it stops the class at once (C5 fails).
- **Agent posing as the bot.** Agents act as the owner's login through the Claude app, so they
  fail C5. An agent that pushes to `splice/update` breaks C5 for every later head.

### Owner decision needed

1. Create a splice bot GitHub App (Contents and Pull requests: write on this repo), its key in
   the `splice-bot` environment (main-only); or keep the PAT and the class stays off. In a repo
   that uses the bosun channel the App also needs Workflows: write, since Contents: write
   cannot push rendered workflow files; those files stay in the floor either way.
2. May a clean update change agent-instruction files (`CLAUDE.md` sections, `AGENTS.md`,
   `docs/agents/**`) without the owner? Default proposed: no, unless C7 (b) or (c) is chosen.
3. C7 per source: `track` ancestor, a GitHub Release, or a signed tag (which keys).
4. Delegate-class R-ci paths (`deny.toml`, toolchain files) in a clean update: owner, or delegate.
5. Refuse downgrades (new commit must descend from the base lock's commit)?
6. Class name and status text: a new `template` class (proposed), or plain `normal`.
7. Should a clean update also get `review-verdict` automatically, or still need a session review?
8. Which paths go in bosun's `[template_update] paths` (follows from 2 and 3).
