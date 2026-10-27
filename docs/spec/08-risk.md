## 8. Risk gating

`.bosun/risk.toml` rules:
- R-gov: "Governance files decide who approves what." Paths `.bosun/**`, `governance/**`,
  `.github/CODEOWNERS`, `.claude/**`, `.splice/**`, `REVIEW.md` and `docs/lessons.toml` (both are read into every review pack).
- Consumer repos (`channel/pieces/core/files/.bosun/risk.toml.jinja`): R-gov also lists `docs/agents/**`, `**/AGENTS.md` and `**/CLAUDE.md` (a nested agent-rule file is read by every later agent; `**/` matches the root file too).
- R-ci: "CI decides what is verified." Paths `.github/workflows/**`, `.github/actions/**`, `.circleci/**`, `.gitlab-ci.yml`, `.gitlab/ci/**`, `.buildkite/**`, `**/jenkinsfile` (any case), `azure-pipelines.y*ml`, `bitbucket-pipelines.yml`, `.travis.yml`, `.drone.yml`, `deny.toml`, `rust-toolchain.toml` and the legacy `rust-toolchain`.
- R-gate: "The gate code decides what passes." The risk, verdict, records, meta, glob and approve logic in `src/core` and their CLI commands, the test-integrity, gate and review commands, the config, lease, state and review logic, `src/gates/**`, `src/github/**`, `Cargo.toml`, `Cargo.lock`, `build.rs` (any directory), `.cargo/**`, `.config/**` (build inputs), `src/anthropic.rs`, `src/core/lessons.rs`, `src/core/budget.rs`, `src/sessions/**` (review feeders), `src/app.rs`, `src/output.rs`, `src/lib.rs`, `src/main.rs`, `profiles/**`, `npm/**`. A module is listed as `x.rs` and `x/**`. The full list is in `.bosun/risk.toml`.
- R-ci-review (`approval = "two-verdicts"`): "Shipped CI for other repos; two independent reviews instead of the owner." Path `channel/**` (the CI and governance `bosun init` ships).
- R-secrets: "A secret in the repo is a leak." Paths `**/secrets.toml`, `**/access.toml` and `**/*.sops.*` (`.sops.yaml` included); content `(?i)(api[_-]?key|secret|token)\s*=` or `AGE-SECRET-KEY-1` or `SOPS_AGE_KEY\s*[:=]` (a path or the content matches). It is critical, so it never goes to a delegate. Risk paths also match the lowercased path (a case-insensitive filesystem treats `Secrets.toml` and `.CircleCI/` as the same files), so patterns are written in lowercase.
- R-deps: "A new dependency adds code that nobody reviewed." `new_dependency = true`.
- R-prod-write: "A write to production cannot be undone by a revert." `capability = "prod-write"`.

Rule evaluation:
- risk-check unions the capabilities of every issue the PR body links (`Closes|Fixes|Resolves #N`, any case, and `<!-- bosun:task-pr N -->`), because the agent writes the body.
- risk-check adds the built-in match `R-unseen` ("bosun could not see the whole diff", so high) when GitHub lists fewer files than the PR changed, or lists a file with changed lines but without a patch (GitHub reports 0 changed lines for a binary file; the file name never exempts a file).
- risk-check evaluates the rules from the base branch, never from the PR head. A PR cannot change
  the rules that judge it.
- A rule only escalates; nothing de-escalates.
- The bosun version pin (`.bosun/version`) is R-gov.
- A rule with several conditions matches when any one does. Content rules read only the lines a PR adds, and their evidence names the file and line number, not the line, so a secret is not copied into the PR body. `bosun risk-check` reads capabilities from the `capabilities` list of the linked task's metadata and from `bosun:capability/<c>` labels on the PR.

Approval classes (mechanism): a rule has `approval = "owner"` (the default), `"two-verdicts"` or `"delegate"`; any other value is a parse error naming the rule (risk-check exits 1 and posts no status). A high PR needs the owner when any match needs the owner (a rule without `approval`, and the built-in R-unseen), else a delegate when any match is `delegate` (see delegated decisions below), else two verdicts. `risk-check` sets `data.requirement` (`owner`, `delegate` or `two-verdicts`) and, for two verdicts, `data.verdicts` (the sessions). The built-in rules use the two-verdicts class for R-ci-review only and never delegate; this repo's `.bosun/risk.toml` also makes R-ci, R-gate and R-deps `delegate` (risk.yml judges every PR with the release pinned in `.bosun/version`, so the rules on main must parse with the pin: `delegate` needs 0.2.1 or later; `tests/governance.rs` checks it). A test keeps the two files equal apart from that opt-in.

Two verdicts:
- The class passes when, on the PR's current head, verdict records of at least two distinct sessions have `approved = true` and `confidence >= 0.9`. An owner approval (below) also passes it. The status is `high risk, two verdicts needed (<n>/2): <ids>` (failure, exit 5) while waiting, and `high risk (<ids>), approved by two verdicts: <s1>, <s2>` when met. The owner status text is `high risk, owner approval needed: <ids>`.
- A record counts only when `bosun verdict apply` wrote it: the comment author is `github-actions[bot]`, the first line starts with `Review verdict: `, the record is the last non-empty line, and no other line is a record. A session's newest such record for the head decides, so a later disapproval withdraws its approval. A record without `confidence` never counts.
- Independence: the sessions `unknown` and empty never count, and neither does any session with a `lease` or `lease-end` record (past or live) on an issue the PR body links. `bosun pr` records no session for the PR author, so the lease is the only trace of who wrote the change.
- A verdict record is written with `GITHUB_TOKEN`, which starts no other workflow, so verdict.yml runs `bosun risk-check` for the PR after `bosun verdict apply` (exit 5, still waiting, does not fail the job). It has `statuses: write` for the status and `pull-requests: write` for the Risk note.
- Residual risk: the session in a verdict comment is self-declared, so one agent that posts two verdict comments under two session tags can still pass when neither tag holds a lease on the task. Later options: a GitHub App per reviewer, or one check run per reviewer.
- A PR that links no task (`Closes #N`) cannot satisfy the class by verdicts, as there is no lease to check the reviewers against: only an owner approval passes, and the status (`high risk, owner approval needed (links no task, so verdicts cannot count): <ids>`, so `bosun approve --list` finds it) and fix say so.
- More residual risks. A `review-ci` record (session `review-ci`, mode b, dormant) counts as one verdict, and review.yml does not re-run risk-check. Any workflow that runs with `GITHUB_TOKEN` (for example on a pushed branch) can post a comment of the shape `apply` writes as `github-actions[bot]`; this is the same trust class as the `statuses: write` limit below. The risk-check step in verdict.yml runs outside risk.yml's concurrency group, so a stale run can overwrite a newer risk-check status until the next trigger (liveness only).

High-risk PRs:
- A high PR needs an approval on the head SHA by a user in `governance/owners.toml`, and
  CODEOWNERS. Either counts: a GitHub review that approves the head SHA (the owner's newest
  approve, request-changes or dismissed review on that SHA), or a PR comment by the owner whose
  body, trimmed, is exactly `/bosun approve <head_sha>` with the full 40-character SHA. The
  comment path exists because GitHub forbids approving your own PR, and an owner may also be the
  account that opens the PRs. A comment binds to that SHA: a new push needs a new comment. An
  edited comment does not count (users with write access can edit other users' comments). Only a
  comment written with a plain user token counts: its `performed_via_github_app` is present and
  `null`. Agent sessions act as the same login through the Claude app, and GitHub stamps that
  app on their comments, so an agent cannot approve; a response without the field does not
  count either (fail closed). When
  the owner's newest decisive review on the head SHA requests changes, their comment does not
  count either. Withdrawal: an owner whose newest decisive review (APPROVED, CHANGES_REQUESTED
  or DISMISSED) on the head is CHANGES_REQUESTED or DISMISSED has withdrawn approval: their
  comment approval and every signed and delegate approval stop counting. Anyone with write
  access can dismiss a review (GitHub then reports DISMISSED, also for an APPROVED one), so a
  dismissal never lifts a withdrawal, and a DISMISSED review is never an approval. A request
  for changes ends only with a newer APPROVED review from that owner. After a dismissal, a
  plain-token, unedited `/bosun approve <head>` comment from that owner also ends it when its
  `created_at` is strictly later than the dismissal time, read from the issue timeline
  (`GET /issues/<n>/timeline`, the `review_dismissed` event whose `dismissed_review.review_id`
  is the decisive review's id). If the timeline cannot be read, has no matching event or a
  time does not parse, nothing counts as newer. With several owners, each owner's own newest
  decisive review decides for that owner's comment, and any owner's withdrawal blocks signed
  and delegate approvals. A signature, owner or delegate, never ends a withdrawal: it carries
  no time, so a re-posted old one would pass for new.
- Signed approvals let the owner approve from a Claude chat, whose GitHub writes the cloud
  proxy re-attributes to the Claude app. The orchestrator session holds an ed25519 private key
  in its own container (never shared) and, on the owner's chat message, posts a PR comment whose
  first line, trimmed, is `/bosun approve <head_sha>` and of which some later line, trimmed, is
  exactly `bosun-sig ed25519 <key id> <base64 64-byte signature>` (single spaces). Any author
  and any app attribution count, and edits do not matter: the signature is the proof. Other
  text (a footer) is ignored. The signed message is exactly the UTF-8 bytes
  `bosun-approve v1 <owner>/<repo>#<pr> <head_sha>` with no trailing newline, owner and repo
  lowercased. risk-check counts it, as `approved by key <id>`, only when the key id is in
  `.bosun/approvers.toml` read through the API at `refs/heads/<default branch>` (the full ref,
  so a tag or other ref of that name cannot stand in; never the PR head, the base SHA or the
  checkout, so a PR that adds its own key does not count), the signature
  verifies over the message built from this repo, this PR's number and its current head, and
  line 1 names that head. A signature for an old head, another PR or another repo does not
  verify. Each signature line is checked on its own; one that verifies is enough. A key stands
  for the owners: with no owners in `governance/owners.toml` at base, or while any owner's
  newest decisive review on the head requests changes or was dismissed, a signed approval does not count. Heads
  are 40-hex SHA-1, so a SHA-256 repo has the feature off. A missing or unreadable
  `.bosun/approvers.toml` turns the feature off. A malformed file (bad TOML,
  unknown fields, a duplicate or empty id, a key that is not 32 bytes of base64), an unknown
  key id and a bad signature count as no approval, never error the check, and are logged to
  stderr. The file is under R-gov. The crypto is `ring` (already in the tree through rustls).
  ```toml
  [[key]]
  id = "orchestrator-2026-10"
  ed25519 = "<base64 of the raw 32-byte public key>"
  ```
  Enroll or rotate a key with a PR that changes `.bosun/approvers.toml` (R-gov), approved by
  the owner on GitHub (a review or a plain-token comment). No signed approval counts, owner or
  delegate key, on a PR that changes or renames away `.bosun/approvers.toml`,
  `governance/owners.toml` or a `CODEOWNERS` file (root, `.github/`, `docs/`; `TRUST_FILES` in
  `src/core/approve.rs`), or on a PR whose diff bosun could not see whole (R-unseen: it may hide
  one), so a compromised key cannot enroll keys or owners; risk-check logs why to stderr. A removed key stops counting once that PR merges. Make a key with
  `openssl genpkey -algorithm ed25519 -out key.pem`; its public key is
  `openssl pkey -in key.pem -pubout -outform DER | tail -c 32 | base64`. risk.yml runs the
  release pinned in `.bosun/version`, so signed approvals count in CI once that pin is a
  release with this feature.
- `bosun approve <pr>@<head> --sign-key <pem>` is the signer's helper. It reads an ed25519 PKCS#8 PEM
  (as `openssl genpkey -algorithm ed25519` writes it; errors name the path only, the key is
  never printed or logged; a file readable by group or others gets a warning on stderr). The
  argument is `<pr>@<head>`, the head the owner approved (a bare number is exit 2, so a push in
  between is never signed unseen; `bosun approve --list` gives the heads and works in the
  cloud). It reads the PR's current head (another head is exit 3, nothing signed), finds the
  key's id by its public key in `.bosun/approvers.toml` on the default branch (not enrolled, or
  the file missing or malformed: exit 1, nothing posted, the fix names the public key or the
  file), and posts the signed comment with the session's own token
  (`GITHUB_TOKEN`); `data.approved` lists `pr`, `head_sha`, `key_id`, `comment_id`. It trusts
  the head only from an `https://` or loopback `BOSUN_GITHUB_API` (else exit 1). It is exit 4
  while bosun is halted, `--print` included. The cloud refusal below does not apply with `--sign-key`. `--print`
  puts the comment in `data.body` (with `pr`, `head_sha`, `key_id`, `role`) and posts nothing.
- Delegated decisions let one Claude session decide high-risk PRs itself; only critical
  changes go to the owner. A key in `.bosun/approvers.toml` has `role = "owner"` (the default,
  as above) or `role = "delegate"`; one public key under two ids is a malformed file. A delegate
  signs `bosun-delegate v1 <owner>/<repo>#<pr> <head_sha>` (same comment shape and rules as
  above: `bosun approve <pr>@<head> --sign-key <pem> --as delegate`; `--as` must be the key's
  enrolled role, else exit 1). Each role verifies only its own message, so a delegate signature
  never counts as an owner's, nor the reverse. risk-check counts it, as `approved by delegate
  <id>`, only when all hold:
  - every rule matched on the head is `approval = "delegate"` (any owner rule needs the owner,
    and so does a mix of delegate and two-verdicts rules: a delegate never stands in for two
    verdicts; the mode is off in a repo without `delegate` rules, and the built-in rules have
    none);
  - the PR is not critical (below);
  - bosun is not halted (a halt, or an error reading it, stops delegation, not owner
    approvals), and the PR targets the default branch;
  - the PR links tasks (`Closes #N`), and every linked issue is an open bosun task (`bosun:task`
    metadata, not a PR) with at least one lease record, so a decoy link cannot hide the task's
    capabilities or its lease holders;
  - at least one review verdict record on the head, written by `bosun verdict apply` (author
    `github-actions[bot]`, the shape of the two-verdicts class) from a session without a lease on
    a linked task, approves with confidence >= 0.9 and counts `major: 0` findings (the record
    carries `major` from this version on; an older record without it never counts; an
    approving verdict has no blocker); a session's newest record for the head decides;
  - owners exist at base and no owner has withdrawn approval on the head (withdrawal, above);
  - the signature checks of signed approvals above.
  The critical floor is hard-coded and holds whatever a rule's `approval` says: the rules
  R-secrets, R-prod-write and R-unseen (bosun could not see the whole diff, so it cannot rule
  out a critical file or a secret in it), and any changed file whose path or previous path
  (renames count both ways) is `.bosun/**` (approvers.toml and risk.toml included),
  `governance/**`, `CODEOWNERS` (root, `.github/`, `docs/`), `.github/workflows/**` or
  `.github/actions/**` (any workflow with `statuses: write` can post a `risk-check` status for
  any PR), `.circleci/**` or another CI provider's config (the R-ci list). Where the base has `src/cli/risk_check.rs` (bosun and its forks), bosun's code is
  critical too: all of `src/**` (the approval modules call most others, and any module can do
  I/O with the workflow's token once released), `Cargo.toml`, `Cargo.lock`, `build.rs` (any
  directory), `.cargo/**` and `npm/**` (`CRITICAL_PATHS` and `APPROVAL_CODE_PATHS` in
  `src/core/risk.rs`). On a critical PR every delegate match becomes an owner match, and the
  status reads `high risk, owner approval needed (critical: <rules and paths>): <ids>`;
  `data.critical` lists them. While a delegate may approve, the status reads `high risk, owner
  approval needed (or a delegate): <ids>` (so `bosun approve --list` lists it),
  `data.delegate_verdicts` lists the clean verdicts and `data.delegate_blocked` says why a
  delegate cannot approve now (null when it can). In bosun this leaves `deny.toml`, the toolchain
  files, `.config/**` and `profiles/**` to a delegate (R-ci and R-gate); R-deps is delegate-class
  but every manifest it reads is critical here. Releases stay human through the `release`
  environment. To turn delegation off at once, the owner deletes the delegate key from
  `.bosun/approvers.toml` (read live from the default branch); deleting the `approval =
  "delegate"` lines from `.bosun/risk.toml` applies to each open PR once its base moves. Both
  are critical, so only the owner approves that PR. Residuals: the verdict session is
  self-declared, so a steered delegate can post the one verdict itself under a session without a
  lease: the mode lets one session decide by design, and the verdict is a quality gate, not
  proof of independence. A workflow on a pushed branch can post a record as
  `github-actions[bot]` (Limits above). Files that match no rule (`hoist.toml`,
  `docs/setup-script.sh`, `tests/**` in this repo) are normal risk, so neither the
  owner nor the floor sees them; that is a follow-up for the rules. A self-made task issue
  with its own lease record still passes the task check, hiding the real task's capabilities
  (the same trust class as the self-declared session). No job that runs repo code (tests, build
  scripts, `.config/nextest.toml`) holds a token that can write statuses or issues: main-health's
  nightly job is `contents: read` without persisted credentials (`tests/workflows.rs`).
- `bosun approve` is the owner's shortcut, run on the owner's own machine (a local shell or a
  local Claude Code session, also through Remote Control), never in a Claude cloud session: the cloud agent proxy rewrites
  every GitHub write (app attribution and a footer), so an approval sent from there never counts,
  whatever the token. It
  reads `BOSUN_OWNER_TOKEN` (a fine-grained PAT of the owner) and makes all its calls with it,
  never with `GITHUB_TOKEN`; without it the command is exit 5. It sends the token only to
  `https://` or a loopback `BOSUN_GITHUB_API` (else exit 1) and redacts it from every message.
  Flow: the owner says "approve the ready PRs"; Claude runs `bosun approve --list`, shows the
  owner the ready list (PR, title, risk, files), then runs exactly `data.approve_command`, one
  line `bosun approve 199@<head> 200@<head>` (null when nothing is ready), and `data.hint` says
  to approve only by running `data.approve_command` or in the GitHub UI: a `/bosun approve` comment posted through the Claude
  GitHub tool goes through an app and carries a footer, so it never counts. `--list` shows each
  open PR whose `risk-check` status is `failure` with a description that starts `high risk,
  owner approval needed`, whose `review-verdict` is `success` and whose `ci-ok` is green. Only
  statuses created by `github-actions[bot]` count (anyone with write access can post a status
  under any context, an agent included; the newest such status per context decides; this raises
  the bar, it is no guarantee: a workflow on a pushed branch with `statuses: write` posts as
  `github-actions[bot]` too, see Limits), and only
  a `ci-ok` check run of the `github-actions` app (no commit status). The rule ids come from
  the status text and are printed as `risk_from_status` (advisory); `files` is the comparison
  of base and the listed head, so statuses, files and head describe one commit
  (`files_truncated` is true at GitHub's compare cap of 300 files: review the PR on GitHub then). PRs that wait
  for an owner but are not ready are listed under `waiting`. A bare PR number is a usage
  error: each argument names the head the owner saw. The command recomputes the list and
  posts `/bosun approve <head>` on each PR only if every named PR is ready at exactly that
  head; otherwise nothing is posted (exit 3, `data.not_ready`, for example `head changed`) and
  the owner needs a new `--list`. If a later comment fails, the ones posted stay recorded and
  `data.approved` lists them (exit 1). A PR whose owner has a review on that head that requests
  changes (the newest decisive review withdraws an approval comment) is not ready: it is in
  `waiting` (the hint names only a newer APPROVED review) and refused with exit 3. One whose
  request was dismissed is in `waiting` too ("needs a fresh plain owner comment", that is
  `bosun approve <pr>@<head>`, "or a GitHub review") but an explicit `<pr>@<head>` is allowed:
  it posts exactly the comment that ends the withdrawal. `--sign-key` refuses either kind
  (exit 3, the fix names the plain-comment or review way). Otherwise the comment is always posted, also when one that
  counts (the owner test of risk-check: unedited, plain user token, exact text) already exists,
  because the new comment re-runs risk-check if its run was lost; `data.approved` marks that
  entry `"repost": true`. In a Claude cloud session (`CCR_AGENT_PROXY_ENABLED` is `1`, `true`,
  `yes` or `on`, or `CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE` starts with `cloud`, any case) the
  agent proxy rewrites every GitHub write (app attribution, a footer), so no token
  yields a counting approval there: the posting modes without `--sign-key` are exit 5 before any call (run it on
  your own machine, or comment in the GitHub app), and `--list` still works with
  `data.cloud: true`. Both are exit 4 while bosun is halted. One `approve`
  record (`prs`, `at`, no session) goes on the registry issue; it is an audit hint, never
  authority: risk-check derives approval only from the owner's PR comment or review, or a
  signed approval.
  `--main-open <pr>` is for a PR that changes `.github/workflows/main-open.yml` (the base
  workflow cannot judge it; any other PR is exit 3). It posts the `main-open` status with the
  decision of `bosun gate main-open`, exit 4 when main is red and the PR is no mainfix, and
  records it as `main_open`. The comment still counts only for an owner in
  `governance/owners.toml`, on an unedited comment of a plain user token.
- The `risk` workflow runs on `pull_request_target` (opened, synchronize, reopened, edited), on
  reviews, and on PR comments (`issue_comment` created) that contain `/bosun approve`, so a
  comment re-evaluates `risk-check` without a push. `pull_request_target` and `issue_comment` run
  the workflow file from the base branch, so a PR cannot rewrite the job that judges it. The job
  never checks out or runs PR-head code: it runs the release pinned in `.bosun/version` of the
  PR's base SHA (on a comment, the default branch; built from it only while the pin says
  `unreleased`), so a base that does not compile cannot block its own mainfix, and reads the PR through the API. Event data reaches scripts through `env`
  only. Permissions: `contents: read`, `issues: read`, `statuses: write`, and
  `pull-requests: write` for the Risk note in the PR body. `tests/workflows.rs` asserts all of
  this. Limits: `pull_request_review` has no base-branch variant, so a review event runs the PR's
  copy of the file; and a same-repo PR can add a workflow of its own that requests
  `statuses: write`. Both are R-ci, so an owner sees them, but neither is blocked by bosun.
- bosun writes a Risk note into the PR body: the rules that matched, why, and what the owner (or, for two verdicts, the two reviewers) must check.
- `bosun ask` sends the question to the owner (GitHub mention, and Slack if set). Seed: the GitHub mention only, owners read from `governance/owners.toml` on `main`; the comment carries an `ask` record. There is no
  timeout and no automatic approval.
- A risky task is small: the target is under 100 changed lines.

Kill switch: bosun is halted while either source says so, and then every command that starts or
changes work (including `bosun pr` and `bosun verdict post`) is exit 4:
- An open issue with the label `bosun:halt`, opened by a login in `governance/owners.toml` (read
  from the default branch at `refs/heads/<branch>`, so a tag of that name cannot stand in; `bosun notify` likewise). An issue opened by anyone else is ignored. Any error reading the
  issues or the owners is exit 4 (fail closed), and so is an open halt issue while no owners
  are listed. Agents must never close a halt issue or remove its label; bosun does not yet
  enforce that (a token with Issues: write can), so against a misbehaving agent the owner also
  sets `BOSUN_HALT=1`, which agents cannot change. The owner creates the label once:
  `gh label create bosun:halt --repo <owner>/<repo> --color B60205`. To halt:
  `gh issue create --repo <owner>/<repo> --label bosun:halt --title "halt" --body "<why>"`; to
  resume, close the issue.
- The repo variable `BOSUN_HALT`, where the token can read it: `1` halts. 404 (unset) and 403 (not
  readable from this token, as in cloud sessions behind the agent proxy) are no signal from this
  source. Any other error reading it is exit 4. Only the owner sets it.

`bosun status` and the SessionStart context show whether bosun is halted; `bosun doctor` step 5
checks that the `bosun:halt` label exists and that the kill switch is readable.

Audit: `bosun progress --risk` lists the high PRs, who approved them, and the time to approval.
