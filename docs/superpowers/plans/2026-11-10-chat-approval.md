# Chat approval, no-CI mode, setup auto-detect Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the owner approves every gated PR with one chat answer, from the first PR, with or
without CI; an orchestrator records it and (owner decision a) `bosun merge` merges without CI.

**Architecture:** a chat approval is a PR comment (`/bosun approve <head> chat` + marker) that
only an orchestrator session may write; risk-check counts it like an owner comment. A
PostToolUse hook stores the owner's AskUserQuestion answer in the lease cache dir; `approve
--chat` needs it. PreToolUse refuses the marker from anyone else. `[ci] provider = "none"`
moves risk/verdict/approval checks into `pr --ready` and `bosun merge`.

**Tech Stack:** Rust, existing crates only (serde, ureq, clap, tiny_http in tests). No new crate.

**Spec:** `docs/superpowers/specs/2026-11-10-chat-approval-design.md` (incl. "Calls made").
Spec sections touched: §1 (D13 row), §4 (`pr --ready`, merge), §8 (approvals), §14 (approval
batches).

## Global Constraints

- One task = one PR from `origin/main`, ≤400 changed lines (tests, lockfiles excluded; D-013),
  with `changelog.d/<branch>.md` and `docs/progress.d/<branch>.md` (branch `/` → `-`).
- stdout one JSON object; exit codes 0/1/2/3/4/5/6; `error.fix` never empty; `src/core` no I/O;
  tests offline via `tests/common`; tokens never printed.
- Chat record first line exactly `/bosun approve <40-hex head> chat`; marker line exactly
  `<!-- bosun:chat-approval v1 session=<id> head=<sha> question=<id> -->`.
- Orchestrator = registry `Member` with `depth == 1 && parent.is_none()`, or `origin` `local`
  or `slack`. Ids containing `/` (wave agents) never qualify.
- Question id = `<pr>-<short head (7)>-<unix secs>`.
- New config keys: `[approvals] chat: Option<bool>` (default true), `[ci] provider:
  Option<String>` in `{"github","circleci","none"}`; add to `tests/pinned_config.rs:260`
  `KNOWN_APPROVALS` (new release row) / known sections. Older pins reject them: release ordering as usual.
- Hook guard is best effort (option A): it stops confused workers, not liars. Say so in §8.
- Fast checks before each commit: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo nextest run --locked`.
- Dependency T0 (outside bosun): PR in keelstackhq/decisions amending D-008 (orchestrator may
  run `bosun merge` without CI), D-012 (chat record replaces the signature as the default
  form) and noting spec D13. Owner (or a session attached to that repo) opens it; it should
  merge before T2 and T10 merge. Not blocking writing T1–T9.

## Review Focus

- An edited chat comment (`updated_at != created_at`) or one naming an old head → not counted
  (T2 test `chat_record_edited_or_stale_not_counted`).
- A chat record whose `session=` is a spawned child or a `S/task` id → not counted even if the
  author is an owner (T2 test `chat_record_child_session_not_counted`).
- `approve --chat` when the head moved between the question and the answer → exit 3, no
  comment (T5 test `chat_head_moved_refused`).
- A worker posting the marker via Bash `gh api …/comments -f body=…bosun:chat-approval…` or
  `curl` → hook denies (T6 test `bash_gh_and_curl_marker_denied`).
- `bosun merge` on a PR whose head changed after the summary/approval → exit 3, no merge
  (T10 test `merge_head_moved_refused`).

---

### Task 1: core chat record + config key

**Gated:** yes (`src/core/approve.rs`). **Model:** opus. **Depends:** none.
**Branch:** `claude/chat-record-core`.

**Files:** Modify `src/core/approve.rs` (new fns below), `src/config.rs:132` (`Approvals.chat`),
`tests/pinned_config.rs` (`KNOWN_APPROVALS` += `chat`); Test: unit tests in
`src/core/approve.rs`, `tests/config.rs`.

**Interfaces — Produces:**
- `pub struct ChatRecord { pub head: String, pub session: String, pub question: Option<String> }`
- `pub fn chat_comment(head: &str, session: &str, at: &str, question: Option<&str>) -> String`
  (3 lines per Global Constraints; line 2 `Approved by the owner in chat (session <id>, <at>).`)
- `pub fn parse_chat_record(body: &str, head: &str) -> Option<ChatRecord>`: first trimmed line
  == `/bosun approve {head} chat`, marker `head=` == head, `session=` non-empty.
- `pub fn is_orchestrator(members: &[core::lease::Member], session: &str) -> bool` (rule above;
  newest `Member` for that session wins).
- `pub fn question_id(pr: u64, head: &str, unix: i64) -> String`.
- `Approvals::chat_enabled(&self) -> bool` (None → true).

- [x] Tests: `chat_comment_round_trips` (parse(chat_comment(h,s,t,Some(q)),h) == Some{h,s,Some(q)});
  `parse_rejects_other_head`, `parse_rejects_missing_marker`, `parse_rejects_plain_approve`
  (`/bosun approve <h>` → None); `orchestrator_root_local_slack` (depth1/no parent → true;
  origin local/slack depth 2 → true; spawned depth 2 parent Some → false; `S/1` → false;
  unknown session → false); `question_id_format` (`question_id(12,"abcdef1234…",1700000000)
  == "12-abcdef1-1700000000"`); config `approvals_chat_defaults_true`, `approvals_chat_false_parses`. Run: FAIL.
- [x] Implement; PASS. Fragments. Commit, push, open PR.

### Task 2: risk-check counts chat records (+ no-owners fallback)

**Gated:** yes (`src/cli/risk_check.rs`, `src/core/risk.rs`). **Model:** opus. **Depends:** T1; T0 before merge.
**Branch:** `claude/chat-record-count`.

**Files:** Modify `src/cli/risk_check.rs:875` (`approver`), `:1275` (add `chat_comment_by`),
`:41` (`owner_logins` fallback caller), `src/github/mod.rs` (trait: `fn org_role(&self, org:
&str, user: &str) -> Result<Option<String>>` = GET `/orgs/{org}/memberships/{user}` `.role`,
404 → None; `RepoInfo.owner: Option<RepoOwner{login, #[serde(rename="type")] kind}>`),
`tests/common/fake.rs` (route + `org_member(org,login,role)` seed, `owner` in repo JSON),
`docs/spec/08-risk.md` (approval paragraph ~:52-60: chat record is an owner approval, all
policies, R-weaken included; best-effort guard). Test: `tests/cli_chat_count.rs`.

**Interfaces — Consumes:** T1 `parse_chat_record`, `is_orchestrator`, `chat_enabled`;
`lease::members(&[Note])`. **Produces:** `pub fn chat_comment_by<'a>(comments: &'a [Comment],
owners: &[String], members: &[Member], head: &str) -> Option<&'a Comment>` (unedited, author
in owners, parsed, `is_orchestrator(members, rec.session)`). `approver` tries it after owner
comments, unless `chat_enabled()` is false. `pub fn fallback_owners(gh, author: &str) ->
Result<Vec<String>>` used only when `owner_logins` is empty: user repo → `[owner.login]`; org
repo → `[author]` if `org_role(org, author) == Some("admin")`, else `[]` (empty-owners path unchanged).

- [x] Tests (fake GitHub, gated PR, `risk-check` run): `chat_record_counts_as_owner`
  (owner comment via `comment_by` + registry `Join` root for `session_orch` → risk-check
  success, `data.approved_by` = owner); `chat_record_edited_or_stale_not_counted`;
  `chat_record_child_session_not_counted` (spawned depth 2, and `S/3`);
  `chat_record_non_owner_author_not_counted`; `chat_off_ignores_record` (`[approvals] chat =
  false` on base); `chat_counts_on_r_weaken`; `no_owners_file_user_repo_owner_counts`;
  `no_owners_file_org_admin_counts`, `no_owners_file_org_member_not_counted`;
  `via_app_author_still_counts` (cloud: comment `via_app: Some("claude")` counts for chat,
  unlike plain `/bosun approve`). Run: FAIL.
- [x] Implement; PASS; existing `tests/cli_risk*.rs`, `cli_signed_approve.rs` still pass. Spec §8. Fragments. Commit, push, PR.

### Task 3: `approve --list` returns `ask` + `next`

**Gated:** yes (approval code). **Model:** opus. **Depends:** T1.
**Branch:** `claude/approve-ask`.

**Files:** Modify `src/cli/approve.rs:113` (`board`), `:146` (`board_of`), `:261` (`exec`),
`:82` (`LIST_HINT`), `:462` (`redact` debug_assert: allow `next` in list mode),
`docs/spec/14-prereqs.md:60` and `docs/spec/08-risk.md:60` (approval batches). Test:
`tests/cli_approve_ask.rs`.

**Interfaces — Consumes:** T1 `question_id`; `Outcome::with_next(tool, args)` (src/output.rs:146);
the `slack_post` shape from `src/cli/notify.rs:598`. **Produces:** each `ready[]` item gains
`ask {question_id, question, options: ["Approve <short>", "Hold"]}`; question text names repo,
`#pr title`, risk rules hit, short + full head, verdict. `next`: one `AskUserQuestion`
(`questions[0]` with `multiSelect: true` when >1 ready, header `Approve`, options per PR
`Approve #<pr> <short>` + `Hold`) when `ctx.origin != Slack`; for Slack the `slack_post`
next with text ending `answer "approve <short>"`. Question header must stay ≤12 chars.

- [x] Tests: `list_one_ready_has_ask_and_next` (assert `next.tool == "AskUserQuestion"`,
  option labels, question contains full head and `acme/widget#12`); `list_two_ready_multiselect`;
  `list_slack_origin_next_is_slack_post` (`BOSUN_ORIGIN=slack`); `list_none_ready_no_next`
  (anti-goal: never block when nothing gated waits); question ids stable per `BOSUN_NOW`. Run: FAIL.
- [x] Implement; PASS; `tests/cli_approve*.rs` pass. Spec. Fragments. Commit, push, PR.

### Task 4: PostToolUse hook stores AskUserQuestion answers; holds

**Gated:** yes (hook + approval logic). **Model:** opus. **Depends:** T3.
**Branch:** `claude/hook-post-tool-use`.

**Files:** Create `src/hooks/post_tool_use.rs`, `src/hooks/answers.rs` (store); Modify
`src/cli/mod.rs:714` (`HookCmd::PostToolUse`, name `hook post-tool-use` in table ~:277),
`src/hooks/mod.rs:21` (dispatch), `.claude/settings.json` (PostToolUse matcher
`AskUserQuestion` → `bosun hook post-tool-use`), `channel/pieces/core/files/.claude/settings.json`, `src/cli/approve.rs` (list mode skips PRs held at the
same head; `data.held[]`). Test: `tests/cli_hook_answers.rs`.

**Interfaces — Produces:** answers file `<git common dir>/bosun/answers.json` (sibling of
`hooks::cache::path`), map `question_id → {answer: "approve"|"hold", head, pr, at}`.
- `pub fn post_tool_use::run(cwd: Option<&Path>, input: &Value) -> Option<String>` (never
  denies; reads `tool_input.questions[*]` + `tool_response` answers; a label `Approve #<pr>
  <short>` → approve, `Hold` → hold; question ids taken from the question text marker
  `[bosun:<question_id>]` T3 appends).
- `pub fn answers::get(dir: &Path, question_id: &str) -> Option<Answer>`;
  `pub fn answers::held(dir: &Path, pr: u64, head: &str) -> bool`;
  `pub struct Answer { pub answer: String, pub pr: u64, pub head: String, pub at: String }`.

- [x] Tests: `answer_approve_stored`, `answer_hold_stored`, `non_bosun_question_ignored`,
  `hook_output_is_hook_protocol` (no envelope, exit 0), `held_pr_not_reasked_same_head`,
  `held_pr_reasked_new_head`. Run: FAIL.
- [x] Implement (atomic write like `hooks/cache.rs:57`); PASS. Fragments. Commit, push, PR.

### Task 5: `bosun approve <pr>@<head> --chat`

**Gated:** yes (approval code). **Model:** opus. **Depends:** T1, T2, T4.
**Branch:** `claude/approve-chat`.

**Files:** Modify `src/cli/mod.rs:500` (`ApproveArgs`: `chat: bool`, `question: Option<String>`,
`slack_user: Option<String>`), `src/cli/approve.rs:191` (`run`: `--chat` path before the
cloud refusal at :216), `docs/spec/08-risk.md`. Test: `tests/cli_approve_chat.rs`.

**Interfaces — Consumes:** T1 `chat_comment`, `is_orchestrator`; T4 `answers::get`;
`core::approve::parse_target` (:95); `gh.add_comment`; `write_record` (:443); `autonomous_ok` (:698, uses `risk_check::autonomous_clean`) generalised to `fn clean_verdict_by_other(app, &Pull) -> Result<bool>`.
**Produces:** `fn approve_chat(app: &App, pr: u64, head: &str, args: &ApproveArgs) -> Result<Outcome, BosunError>`.
Refusals, in order: chat disabled → exit 5; not orchestrator → exit 5, fix names `bosun
handoff-pr`; head ≠ PR head → exit 3; no clean verdict from another session on head → exit 5;
no stored `approve` answer for `--question` matching pr+head (non-Slack) → exit 5, fix: ask
again; Slack: `--slack-user` not in `governance/owners.toml` slack ids (as `bosun answer`) →
exit 5. On success: one comment, `data {pr, head_sha, comment_id, question_id}`.

- [x] Tests: `chat_records_comment` (body == `chat_comment(...)`, one comment);
  `chat_child_refused` (`BOSUN_ORIGIN=spawned`, depth 2 → exit 5, fix contains `handoff-pr`,
  no comment); `chat_head_moved_refused`; `chat_without_clean_verdict_refused`;
  `chat_without_stored_answer_refused`; `chat_hold_answer_refused`; `chat_slack_owner_ok`,
  `chat_slack_non_owner_refused`; `chat_works_in_cloud` (`CLAUDE_CODE_REMOTE=true`, no owner token). Run: FAIL.
- [x] Implement; PASS. Spec §8. Fragments. Commit, push, PR.

### Task 6: PreToolUse guard against forged chat records

**Gated:** yes (`.claude`-adjacent hook logic). **Model:** opus. **Depends:** T1.
**Branch:** `claude/guard-chat-marker`.

**Files:** Modify `src/core/guard.rs:152` (`tool_rules`) and `:483` (`bash_rules`). Test:
unit tests in `src/core/guard.rs` + `tests/cli_hook.rs`.

**Interfaces — Produces:** `const CHAT_FIX: &str` ("only an orchestrator records a chat
approval: run `bosun approve <pr>@<head> --chat`"); `fn carries_chat_form(text: &str) -> bool`
(contains `bosun:chat-approval`, or a line matching `/bosun approve <40-hex> chat`).
`tool_rules`: any non-read-only `mcp__github__*` input string carrying it → deny. `bash_rules`:
command carrying it → deny unless the command's first word list is `bosun approve … --chat`
(no `;`, `&&`, `|`, `$(`, backtick).

- [x] Tests: `mcp_comment_with_marker_denied` (`add_issue_comment`, `update_issue_comment`,
  `create_pull_request` body); `bash_gh_and_curl_marker_denied`; `bosun_approve_chat_allowed`;
  `bosun_approve_chat_chained_denied` (`bosun approve 1@h --chat && gh api ...`);
  `plain_comment_allowed`. Run: FAIL.
- [x] Implement; PASS. Spec §4 hook paragraph. Fragments. Commit, push, PR.

### Task 7: user-scope hooks + session-start detection

**Gated:** yes (setup script/hooks). **Model:** sonnet. **Depends:** T4 (hook list incl. PostToolUse).
**Branch:** `claude/user-scope-hooks`.

**Files:** Modify `src/cli/setup_snippet.rs`, `src/cli/setup_bootstrap.sh` (after install:
`bosun setup-snippet --install-hooks`), `src/hooks/session_start.rs:20` (`run`), `:87`
(`context`), `src/hooks/mod.rs` (no-op on second run in the same tool call: key = tool_use_id
in the cache dir), `src/cli/doctor.rs` + `src/cli/status.rs` (same `next` when un-adopted),
`docs/spec/11-seed.md` setup lines. Test: `tests/cli_setup_snippet.rs`, `tests/cli_hook.rs`,
`tests/cli_doctor.rs`.

**Interfaces — Produces:** `setup-snippet --install-hooks [--settings <path>]` (default
`$HOME/.claude/settings.json`): merges SessionStart, Stop, PreToolUse, PostToolUse entries
by exact command string; idempotent; keeps unknown keys. `pub fn
session_start::unadopted(cwd: &Path) -> bool` (no `.bosun/version`). Un-adopted context line:
`bosun is not set up here: run \`bosun adopt --policy autonomous --owner <login>\` and open
the setup PR`; skips registry join and kill-switch calls (no network).

- [x] Tests: `install_hooks_into_empty_settings`, `install_hooks_idempotent` (run twice →
  byte-identical), `install_hooks_keeps_existing_entries`; `session_start_unadopted_no_network`
  (fake GitHub records zero requests, context contains `bosun adopt`);
  `session_start_adopted_unchanged`; `duplicate_hook_same_tool_use_noop`;
  `doctor_unadopted_next_adopt`. Run: FAIL.
- [x] Implement; PASS. Spec. Fragments. Commit, push, PR.

### Task 8: `[ci] provider` config + adopt detection

**Gated:** yes (`.bosun` template, config). **Model:** sonnet. **Depends:** none.
**Branch:** `claude/ci-provider`.

**Files:** Modify `src/config.rs:16` (`Config.ci: Ci`, `Ci { provider: Option<String> }`,
`deny_unknown_fields`; `Config::ci_provider(&self) -> CiProvider`), `src/cli/adopt.rs:124`
(`vars_for`: `ci_provider`), `channel/pieces/core/files/.bosun/bosun.toml.jinja`,
`src/cli/main_health.rs:19` and `src/cli/ci_minutes.rs:48` (no-op with `None`, `data.skipped:
"ci provider none"`), `tests/pinned_config.rs`. Test: `tests/config.rs`, `tests/cli_adopt.rs`,
`tests/cli_main_health.rs`, `tests/cli_gate_ci_minutes.rs`.

**Interfaces — Produces:** `pub enum CiProvider { Github, Circleci, None }` (missing key →
`Github`, today's behaviour); unknown value → config error exit 1 with fix listing the three.
Adopt writes `none` when the checkout has no `.github/workflows/*.y*ml` and no `.circleci/`,
or `--ci none`.

- [x] Tests: `ci_provider_default_github`, `ci_provider_bad_value_error`,
  `adopt_no_ci_writes_none`, `adopt_with_workflows_writes_github`, `main_health_skips_without_ci`,
  `ci_minutes_skips_without_ci`. Run: FAIL.
- [x] Implement; PASS. Spec §9 note. Fragments. Commit, push, PR.

### Task 9: no-CI `pr --ready` + `approve`/`main-open` read local checks

**Gated:** yes (risk/approval flow). **Model:** opus. **Depends:** T2, T5, T8.
**Branch:** `claude/pr-ready-no-ci`.

**Files:** Modify `src/cli/pr.rs:501` (`--ready` branch), `:272` (`check_local_gates` reused),
`src/cli/risk_check.rs` (expose `pub fn evaluate(app: &App, pull: &Pull, base: &str) ->
Result<RiskOutcome>` without posting a status, if not already split), `src/cli/approve.rs:146` + `src/core/approve.rs:72`
(`readiness()` uses `ready_summary` instead of statuses when provider none), `src/gates/main_open.rs:57`
(`judge_pr` result used locally), `docs/spec/04-lifecycle.md:35`. Test: `tests/cli_pr_no_ci.rs`.

**Interfaces — Produces:** `pub fn core::approve::ready_summary(head: &str, gates: &[String],
risk: &str, verdict: &str, approval: Option<&str>) -> String` (first line
`/bosun ready <head>`, marker `<!-- bosun:ready v1 head=<sha> -->`) and
`pub fn parse_ready_summary(body: &str, head: &str) -> bool`. With `CiProvider::None`, `--ready`:
gate stamps for head (else exit 5 listing missing gates) → local risk evaluation → clean
verdict from another session (exit 5) → chat record when gated (exit 5, `next` = `bosun approve
--list`) → main-open local judge → post summary once per head; no auto-merge, no status; task
→ queued; `data.merge_next: "bosun merge <pr>"`.

- [x] Tests: `ready_no_ci_missing_stamp_exit5`, `ready_no_ci_gated_without_chat_exit5`,
  `ready_no_ci_posts_summary_once`, `ready_no_ci_no_status_no_auto_merge` (fake: zero status
  POSTs, `auto_merge_enabled` false), `ready_github_provider_unchanged`,
  `approve_list_no_ci_uses_summary`. Run: FAIL.
- [x] Implement; PASS. Spec §4. Fragments. Commit, push, PR.

### Task 10: `bosun merge <pr>` (owner decision a)

**Gated:** yes (merge path, D13). **Model:** opus. **Depends:** T9; T0 merged first.
**Branch:** `claude/bosun-merge`.

**Files:** Create `src/cli/merge.rs`; Modify `src/cli/mod.rs:91` (`Command::Merge(MergeArgs
{pr: u64, method: Option<String>})`, name `merge`), `src/lib.rs:23` dispatch, `src/github/mod.rs`
(trait `fn merge_pull(&self, number: u64, sha: &str, method: &str) -> Result<()>` = PUT
`/pulls/{n}/merge` with `sha`, `merge_method`; 405/409 → `BosunError` exit 3), `tests/common/fake.rs`
(route + `merged(n)` accessor), `src/core/guard.rs` (bash: `bosun merge` allowed; `gh pr merge`,
`mcp__github__merge_pull_request` still denied), `docs/spec/01-summary.md:21` (D13 row),
`docs/spec/04-lifecycle.md:25,35`, AGENTS.md merge line. Test: `tests/cli_merge.rs`.

**Interfaces — Consumes:** T1 `is_orchestrator`; T9 `parse_ready_summary`, local risk/verdict/
chat checks; `app::main_red`. **Produces:** `pub fn merge::run(dir: &Path, args: &MergeArgs) ->
Result<Outcome, BosunError>`. Orchestrator only (exit 5, fix `handoff-pr`); kill switch → exit 4;
re-checks for the current head: provider none → ready summary + risk + verdict + chat approval
when gated + main-open; provider github/circleci → required checks `ci-ok`, `risk-check`,
`review-verdict`, `main-open` green (the `already_clean` case). Merges with `sha = head`
(method default `squash`), then task → merged via the existing `pr`/`reap` path; `data {pr,
head_sha, merged: true}`.

- [x] Tests: `merge_no_ci_ok`, `merge_head_moved_refused` (exit 3, no PUT), `merge_child_refused`,
  `merge_halted_exit4`, `merge_gated_without_chat_refused`, `merge_github_requires_green_checks`,
  `merge_github_already_clean_ok`, `guard_allows_bosun_merge_denies_gh_merge`. Run: FAIL.
- [x] Implement; PASS. Spec §1/§4. Fragments. Commit, push, PR.

### Task 11: docs — migration + approvers optional

**Gated:** no (docs). **Model:** sonnet. **Depends:** T5, T7, T10.
**Branch:** `claude/chat-approval-docs`.

**Files:** Modify `docs/adopt-existing-repo.md` (chat approval, `[ci] provider`, user-scope
hooks, `.bosun/approvers.toml` optional, `[approvals] chat = false` opt-out, release pin
ordering), `docs/defaults.md` (chat default true), AGENTS.md approval lines,
`docs/PROGRESS.md` Blocked: T0 decisions PR (owner, phone). Test: `tests/fragments.rs` passes.

- [x] Write; `cargo nextest run --locked --test fragments` PASS. Fragments. Commit, push, PR.

### Task 12: `bosun intake verify` accepts a chat record (D-014 as amended by decisions#19)

Follow-up from T2 (PR #488): spec §8 already says intake verify accepts the orchestrator's chat
record of the owner's yes (signature optional); the command needs GitHub reads it lacks today.
Gated, opus, depends on T2. One PR ≤400 lines: read the intake issue's comments, count a chat
record by the T2 rules (`count_chat`), keep signatures working; tests in `tests/cli_intake*.rs`.

## Notes from landed tasks

- T2 (#488): with no owners file, the org-repo fallback uses the repo `admin` permission
  (readable by the Actions token), not the org-membership API. Spec §8 has it.
- T2 minors left: no test for the failed-read path; an empty `owners.toml` also turns on the
  fallback; §8 refers to the T6 guard ahead of T6.
- T8 (#486) minors left for a follow-up: a kept non-table `ci = "none"` reads as github (exit 0,
  Config::parse rejects later); a circleci-only kept checkout still gets a hand step; `gate
  ci-minutes` exits 1 on an invalid bosun.toml (not in spec).
- T6 (#491): the guard also denies prose that names the marker or the chat form in PR bodies,
  file pushes and `git commit -m` (workaround: --body-file / -F). T11 documents it in spec §4.
