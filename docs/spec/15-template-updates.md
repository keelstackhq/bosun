## 15. Clean template updates

**Status: proposed, not built (design B5, 2026-10-28).** Nothing below is in the code. risk-check
works as §8 says until the owner decides the open choices at the end and the plan
`docs/superpowers/plans/2026-10-28-clean-template-update.md` lands. Until then "a rule only
escalates" (§8) holds without exception.

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
  its own GitHub App (owner decision 1). A push by anyone else to `splice/update` breaks C5,
  and C2 checks the content whoever pushed.
- **C6 Only declared files.** Every changed path, other than the changed lock files, is listed
  in the head lock's `[files]` with class `owned` or `shared`, or as a `[sections]` target.
  `seed` files and paths the lock does not list fail. A changed entry that is a symlink
  (mode 120000) or a submodule (160000) fails.
- **C7 The version is published upstream.** `template.commit` is an ancestor of, or equal to,
  the source repository's default branch (compare API: `behind` or `identical`). "The commit
  exists in the source repo" is not enough: GitHub serves commits of any fork through the
  parent's API. `template.version` agrees with the commit: `0.0.0+g<first 7 hex>` for a
  tracked branch, else a tag of the source that points at the commit. Options, per source:
  (a) default branch ancestor only (the minimum); (b) a published, non-draft, non-prerelease
  GitHub Release; (c) a tag signed by a key listed at base in `[template_update]`. The owner
  picks (decision 3).

C1-C7 come on top of the §8 limits: R-unseen (bosun could not see the whole diff), a halted
bosun, a PR not targeting the default branch, or any owner's withdrawal on the head (§8) mean
"not clean".

### What it grants

A clean template update gets the approval class `template`: the bot plus green gates (`ci-ok`,
`review-verdict`, `main-open` as `hoist.toml` requires), with no owner and no delegate. It
applies to a match only when its rule opts in at base (`template_update = true` on the
`[[rule]]`, default false) and the matched file is outside the template floor below. Any other
match keeps its class, and the strictest class wins as in §8. The status reads `high risk
(<ids>), clean template update: <source>@<7 hex>`, and `data.template_update` holds `{clean,
source, commit, reason}`.

**The template floor still needs the owner, also in a clean update:** the §8 critical floor
(`.bosun/**`, `governance/**`, CODEOWNERS, R-secrets, R-prod-write, R-unseen and, in bosun,
`src/**` and the manifests), every R-ci path (`.github/workflows/**`, `.github/actions/**`,
the other CI configs, `deny.toml`, the toolchain files) and `.claude/**`. Why: C1-C7 prove that
the PR is exactly what upstream published, not that upstream is safe. A compromised or careless
template could ship a workflow with `statuses: write` that posts `risk-check` success for every
PR, a step that leaks secrets, a looser `deny.toml`, or a `.claude/settings.json` hook that runs
commands in every agent session. One bad upstream commit would then reach every consumer with no
human look. So a bosun channel update that moves `.bosun/version` or a workflow still waits for
the owner; the class removes the owner only from pins and plain rendered files (the decisions
channel). Agent-instruction files (`CLAUDE.md`, `AGENTS.md`, `docs/agents/**`) are prompts
for every later agent; whether a clean update may change them without the owner is decision 2.

### Where the re-render runs

The re-render (C2, C4) runs as a separate job `template` in `risk.yml`, before the `risk` job,
only on `pull_request_target` for PRs whose author is `bot` and that change `.splice/**`. It
checks out the base SHA (never the head), installs the splice release named at base and checks
its sha256, fetches the source at `template.commit`, renders, and runs `splice check`. Its
permissions are `contents: read` with no persisted credentials and no secrets. It never runs
code from the PR or from the template: rendering is data only (splice must run no piece hooks
or scripts; a channel that needs them is never clean). The job outputs one value, the rendered
tree SHA (or empty on any failure). The `risk` job passes it to the pinned bosun
(`BOSUN_TEMPLATE_TREE`), which compares it with the head tree read through the API and checks
C1 and C3-C7 itself. So a fault in the render job can at most make a PR not clean, except a
splice bug that a hostile template exploits to print the head's tree SHA (a residual).

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
missing tree SHA; a tree mismatch; an API error or a 403; `mergeable` null; a commit not on the
source's default branch. A run without the render (a `pull_request_review` event, which runs
the head's copy of `risk.yml`) treats the PR as not clean, so its status may turn to "owner
needed" until the next push or re-run (liveness only). "Base" in C2 and C4 is the PR's base
SHA, and a head whose merge base is another commit is not clean. When main moves after the
bot's push, approval carry-over (§8) may carry the class over up to 5 clean merges of the
default branch, like any other approval; else the bot's next refresh re-renders.

### Interactions

- **R-ci.** Every R-ci path is in the template floor, so a template update that touches CI is
  never lowered, delegate-class R-ci paths included (decision 4 may give those to a delegate).
- **B2 (a dedicated GitHub App that posts `risk-check` as a check run; `docs/PROGRESS.md`
  Blocked item 5).** Today any workflow on a pushed branch can post a forged `risk-check`; the
  template class neither opens nor closes that. With B2, the render job's result can be a check
  run of that App, so later runs (reviews, comments) reuse it instead of falling back. The bot
  App that opens update PRs must not be the App that judges them: a leaked bot key must never
  post `risk-check`. Proposed order: B2 first (decision 5).
- **Two verdicts and delegates.** The template class never counts as a verdict or a delegate
  approval, and no signature is involved.

### Threat model

- **Compromised template source.** C1-C7 pass by design. The floor keeps workflows, CI, the
  bosun pin, hooks and governance with the owner; what remains (rendered docs and, by decision
  2, agent instructions) reaches main without a human. C7 option (b) or (c) adds an upstream
  human step.
- **Lock pointing at an attacker fork.** A changed `template.source` fails C1; a fork commit
  under the right source name fails C7 (not an ancestor of the default branch). The allowlist
  changes only by an R-gov PR at base.
- **Bot token leak.** The attacker can open a PR as the bot but must still pass C1-C7: the
  diff must equal a render of an allowlisted, published upstream commit. They can only push an
  older or newer published version (a downgrade). Proposed check: `template.commit` is a
  descendant of the base lock's commit (decision 6). The App key lives only in the splice
  workflow's secret; revoking it stops the class at once (C5 fails).
- **Agent posing as the bot.** Agents act as the owner's login through the Claude app, so they
  fail C5. An agent that pushes to `splice/update` breaks C5 for every later head.

### Owner decision needed

1. Create a splice bot GitHub App (Contents and Pull requests: write on this repo) and move
   `SPLICE_BOT_TOKEN` to its installation token; or keep the PAT and the class stays off.
2. May a clean update change agent-instruction files (`CLAUDE.md` sections, `AGENTS.md`,
   `docs/agents/**`) without the owner? Default proposed: yes for the decisions source only.
3. C7 per source: default-branch ancestor, a GitHub Release, or a signed tag (which keys).
4. Delegate-class R-ci paths (`deny.toml`, toolchain files) in a clean update: owner, or delegate.
5. Build after B2, or before it with the job-output hand-off above.
6. Refuse downgrades (new commit must descend from the base lock's commit)?
7. Class name and status text: a new `template` class (proposed), or plain `normal`.
8. Should a clean update also get `review-verdict` automatically, or still need a session review?
