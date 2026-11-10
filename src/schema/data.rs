//! The `data` of an `ok` envelope, per command (notation: see `super`): `<command> = <notation>;`.
//! An entry ends at `;` and a newline, so a literal may hold `;` or ` = `. A notation that is another command's name reuses that command's shape. Hooks print the hook
//! protocol, no envelope, so they have no entry.

pub const DATA: &str = r#"
join = {session: str, origin: str, area: str | null, depth: int, expires: str,
    registry_issue: int, slack: {channel: str, role: str, thread_ts?: str} | null};
reap = {reaped: [{issue: int, session: str}], left: [str], merged: [int], epics_closed: [int],
    epics_error?: str, archive_error?: str};
wave plan = {epic: int, shape: str, warnings: [str], run: [{issue: int, plan_id: str, title: str,
    touch: [str], workspace: str, bundle?: [int], closes?: [int]}], held: [{issue: int, task: str,
    needs: [str]}], waiting: [{issue: int, reason: str}]};
msg = {session: str, chars: int};
keygen = {id: str, role: str, path: str, public_key: str, enrolled: bool, toml: str, steps: [str]};
test-integrity = {base: str, files_checked: int, integrity: str, integrity_reason?: str,
    findings: [{file: str, kind: str, base: int, head: int | null, allowed: bool}], allow?: [str],
    task?: int};
review-pack = {pr: int, title: str, head_sha: str, base_sha: str, since_sha: str | null,
    changed_files: int, chunks: [[str]], delta_fallback: str | null, delta_not_in_pr: [str],
    delta_not_in_pr_count: int, drift: {actual: int, planned: int} | null, files: [{filename: str,
    status: str, additions: int, deletions: int, patch: str | null}], instructions: str,
    review_md: str | null, lessons: [str], ucs: [str], untrusted: [str], task: {..} | null,
    bundle_tasks: [{..}], note?: str};
ci-log = {pr: int, head_sha: str, jobs: [{name: str, conclusion: str, url: str, log: str}]};
ask = {issue: int, comment_id: int, owners: [str], drafted: [int], pr_draft_error: str | null};
hold = {issue: int, id: str, checklist: {number: int, url: str}, released: bool, drafted: [int],
    pr_draft_error: str | null, lesson_error: str | null, warnings: [str]};
answer = {issue: int, comment_id?: int, state?: str, ask?: int, key_id?: str, moved?: bool,
    not_moved?: str, truncated?: bool, chat?: str};
lesson add = {id?: str, issue?: int, count?: int, global_count?: int, dup?: str, upstream?: bool};
lesson hit = {id: str, hits: int, comment_id: int | null};
lesson capture = {issue: int, kind: str, signature: str, created: bool, reopened: bool, hits: int};
lesson triage = {issue: int, as: str, lesson?: str, process_red?: int, task?: int};
doctor = {offline: bool, missing: int, items: [{step: int, name: str, status: str, detail: str,
    fix: str, content?: str, environment?: str, run?: str}], routines: [{name: str, model: str,
    schedule: str, prompt: str}], environments?: [{status: str, detail: str, fix: str, name?: str,
    job?: str}], hoist?: {exit_code: int, release_inferred: bool, drift: [str], notes: [str],
    skipped: [str], unknown: [str]}, notify?: {skipped: bool, reason: str}};
budget record = {usd: num, provider?: str, session?: str, model?: str | null, issue?: int,
    blocked?: bool, over_task_cap?: bool, task_total_usd?: num, prs?: [int], recorded?: [int],
    already_recorded?: [int], usd_per_pr?: num, lesson?: {created?: bool, hits?: int, issue?: int,
    error?: str, fix?: str} | null};
budget check = {daily_usd: num, spent_today_usd: num, pct: num, level: str, stopped: [str]};
budget report = {month: str, written: [str], skipped_no_pr: [int], rows: [{pr: int,
    issue: int | null, title: str, outcome: str, merged_at: str | null, usd: num | null,
    tokens: any, models: [str] | null, sessions: [str] | null}], totals: {usd: num, prs: int,
    prs_with_spend: int, merged: int, merged_with_spend: int, held: int, held_with_spend: int,
    reverted: int, cost_per_merged_usd: num | null, cost_per_held_usd: num | null,
    top_prs: [{pr: int, title: str, usd: num}]}};
plan check = {file: str, title: str, project: str | null, items: [{id: str, title: str, area: str,
    shape: str, shape_reasons: [str], size: int, tasks: int, ucs: int, gated: [str]}],
    warnings: [{line: int, message: str, fix: str}]};
plan import = {file: str, dry_run: bool, warnings: [str], skipped?: [str], epics: [{id: str,
    action: str, issue: int | null, state?: str, closed?: bool}], tasks: [{id: str, action: str,
    issue: int | null, state: str | null, closed?: bool}], held: [{task: str, needs: [str]}],
    on_default: [any], on_default_open: [any], promoted?: [str], would_promote?: [str],
    closed?: [str], would_close?: [str], reopened?: [str], would_reopen?: [str], updated?: [str],
    would_update?: [str]};
plan amend = plan import;
plan pr = {claim: {..} | null, intake: {..}, pr: {..}};
fail = {issue: int, signature: str, streak: int, stopped: bool, rule?: str, state?: str,
    escalate?: {from: str, to: str}, lesson?: {created: bool, hits: int, issue: int} | null,
    lesson_error?: str};
resume = {session: str, joined: bool, relay_waves?: int, lost: [{issue: int, fix: str,
    conflicts: [{..}]}], tasks: [{issue: int, title: str, state: str, action: str, pr: int | null,
    branch: str | null, head_sha: str | null}], handoff?: {at: str, session: str, comment: int,
    lines: [str]} | null};
handoff = {issue: int, comment: int, session: str, count: int, lines: [str]};
handoff-pr = {pr: int, head_sha: str, parent: str, risk_paths: [str], review_minors: [str]};
progress = {days?: int, since?: str, merged?: int, held?: int, reverted?: int, spend_usd?: num,
    cost_per_merged_task_usd?: num | null, cost_per_held_task_usd?: num | null,
    task_cost_avg_usd?: num | null, tasks?: [{issue: int, title: str, closed_at: str, usd: num}],
    lessons?: {open: int, blocking: int, advice: int, mechanized: int, hits_last_7_days: int,
    hits_by_week: [{week_start: str, hits: int}]}};
result = {epic: int, outcome: str, pr: int | null, comment_id: int, task_state: str,
    tasks_merged: [int], lease_ended: bool, left: bool, epic_closed: bool,
    epic_already_closed: bool, partial_skipped: str | null, state_skipped: str | null,
    epic_error?: str};
main-triage = {good: str, bad: str, run: str, runs: int, candidates: int, first_bad: {pr: int,
    sha: str, subject: str} | null, note?: str};
init = {branch: str, commit: str, ref: str, source: str, channel: str, pieces: [str], risk: str,
    plan: str | null, own_ci: bool, claude_md: bool, excluded: [str], changes: [{action: str,
    class: str, path: str}], hoist_checks_added: [str], hoist_merge_queue: str | null,
    owner_steps: [str], warnings: [str], pr: {number: int, head_sha: str} | null, pr_title?: str,
    pr_body?: str, intake?: str, routines?: [{name: str, model: str, schedule: str, prompt: str}]};
adopt = {dry_run: bool, policy: str | null, stacks: [str], commands: [str], owner_steps: [str],
    changes: [{action: str, path: str, fix?: str}], labels: [{action: str, name: str}],
    registry: {action: str, reason?: str, number?: int}};
uninstall = {dry_run: bool, files: [{action: str, path: str, fix?: str}], labels: [{action: str,
    name: str, fix?: str}], registry: {action: str, number?: int}};
pin = {unchanged: [str], changed?: [{tool: str, from: str, to: str}], behind?: [{tool: str,
    from: str, to: str}], files?: [str], branch?: str, title?: str, pr?: int | {number: int,
    head_sha?: str} | null, pr_lookup?: str, left?: [str], steps?: [str]};
setup-snippet = {script: str, paste_into: str, repo_script: str, client_message: str};
revert = {pr: int, revert_pr: int, reverted_sha: str, branch: str, base: str,
    main_red: int | null, gate_missed: {created: bool, issue: int}, warning: str | null};
trailers = {hook?: str, stamped?: bool, session?: str, revert?: str | null, commits?: [{sha: str,
    subject: str}]};
intake = {issue: int, created: bool, plan: str, item: str | null, source: str};
intake check = {goal: str, plan: str | null, repo: {owner: str, name: str, visibility: str},
    stack: [str], pieces: [str], bosun_pieces: [str], publish: [str], approvals: str, notify: str,
    slack_channel: str};
intake sign = {key: str, mode: str, nonce: str, expires: str, sig: str};
intake verify = {verified: bool, repo: str, key: str, mode: str, nonce: str, expires: str,
    intake: {..}};
main-health = {result: str, changed: bool, opened: bool, closed: bool, main_red: int | null,
    note: str, rejudged: int, rejudge_errors: [any], skipped_unknown: [int],
    conflicts: [{issue: int, pr: int, posted: bool, fix: str}], conflict_errors: [{error: str,
    issue?: int}], gate_missed: {created?: bool, issue?: int, error?: str} | null,
    lesson: {created: bool, hits: int, issue: int} | null};
gate-missed = {issue: int, created: bool, key: str};
review-ci = {pr: int, head_sha?: str, since_sha?: str | null, model?: str, skipped?: str,
    dry_run?: bool, approved?: bool, status?: str, reasons?: [str], task_state?: str | null,
    findings?: [{severity: str, file: str, line: int | null, claim: str, evidence: str}],
    findings_count?: int, findings_comment?: {id: int} | null, verdict?: {..},
    usage?: {input_tokens: int, output_tokens: int, usd: num}, spend?: {issue?: int | null,
    usd?: num, error?: str, fix?: str}};
notify = {channel?: str, skipped?: bool, reason?: str, acked?: str, already?: bool,
    seat?: {channel: str, role: str, thread_ts: str | null} | null, pending?: [{key: str,
    kind: str, text: str, ack: str, thread_ts: str | null}], errors?: [{kind: str, message: str}]};
prereqs = {message: str, open: int, warnings: [str], plans: [{file: str, plan_path: str,
    title: str, message: str, open: int, issue: {number: int, url: str, created: bool} | null,
    items: [{id: str, title: str, who: str, on: str, steps: str, verify: str, sum: str,
    state: str, done: bool, blocks: [str]}], approvals: [{batch: int, task: str, needs: str,
    why: [str]}]}]};
claim = {comment_id: int, issue: int, lease: {expires: str, session: str, touch: [str]},
    lessons: [str], added?: [str]};
heartbeat = {leases: [int], registry: bool};
release = {comment_id: int, issue: int, state: str};
spawn = {checks_passed?: int, epic?: int, model?: str, child?: str, from?: str, root?: str,
    environment_id?: str | null, handoff?: int, relay_waves?: int, then?: str, thread_ts?: str,
    intake?: int, warning?: str};
wait = {changed: bool, polls: int, before: {..}, after: {..}, heartbeat: {beats: int,
    error: str | null, leases: [int], left: [str], reaped: [{issue: int, session: str}],
    registry: bool}};
gate = {gate?: str, stack?: str | null, command?: [str], packages?: [str], exit_status?: int,
    secs?: num, over_budget?: bool, skipped?: bool, reason?: str, stamp?: str | null,
    uncommitted?: [str], files?: [str], findings?: [any], base?: str, files_checked?: int,
    integrity?: str, head_sha?: str, main_red?: int | null, mainfix?: bool, pass?: bool, pr?: int,
    checks_skipped?: bool,
    ci?: {base: str, event: str, provider: str}, runs?: [{gate: str, stack: str | null,
    command: [str], packages: [str], exit_status: int, secs: num, over_budget: bool,
    skipped: bool}]};
approve = {approved?: [{pr: int, head_sha: str, comment_id: int, repost?: bool,
    after_review?: int, key_id?: str, policy?: str | null, role?: str}],
    approve_command?: str | null, auth?: str, cloud?: bool, hint?: str, message?: str | null,
    message_id?: str | null, message_pairs?: [str] | null, ready?: [{pr: int, title: str,
    head_sha: str, files: [str], files_truncated: bool, risk_from_status: [str]}],
    skipped?: [{pr: int | null, why: str}], waiting?: [{pr: int, why: str}], skipped_fix?: str,
    main_open?: {head_sha: str, pr: int}, bodies?: [{after_review: int | null, body: str}],
    body?: str, bootstrap_until?: str | null, head_sha?: str, key_id?: str, policy?: str | null,
    pr?: int, role?: str};
risk-check = {pr: int, head_sha: str, level: str, matches: [{rule: str, why: str, approval: str,
    evidence: [str]}], out_of_touch: [str] | null, approved_by: str | null,
    approval_carried_from: str | null, verdict_carried_from: str | null, requirement?: str,
    policy?: str, verdicts?: [str], autonomous_verdicts?: [str], delegate_verdicts?: [str],
    delegate_blocked?: str | null, checks_skipped?: bool};
verdict post = {pr: int, head_sha: str, approved: bool, status: str, reasons: [str],
    task_state: str | null, checks_skipped?: bool};
verdict comment = {pr: int, head_sha: str, comment_id: int, applied_by: str};
verdict apply = verdict post;
pr = {pr: int, created: bool, draft: bool, head_sha: str, auto_merge: bool,
    task_state: str | null, ready?: bool | "pending", merged?: bool, epic_closed?: bool,
    auto_merge_route?: str | null, enforced_checks?: str | null, note?: str, touch_check?: str,
    gates?: [{context: str, state: str}], pending_checks?: [{context: str, state: str}],
    trailers?: {checked: int, required: bool, unread: int, missing: [{sha: str, missing: [str]}]},
    epic_error?: str};
status = {repo: str, session: str, origin: str, halted: bool, integrity: str,
    integrity_reason?: str, main_red: int | null, process_red: int | null,
    issues_by_state: {[str]: int}, leases: [{issue: int, expires: str, session: str,
    touch: [str]}], live_sessions: [{session: str, origin: str, area: str | null, depth: int,
    expires: str}], budget_pct: num, daily_usd: num, spend_today_usd: num,
    archive_error?: str};
schema = {format: "json" | "ts", command?: str, schema?: {..}, envelope?: {..},
    commands?: {[str]: any}, ts?: str};
"#;
