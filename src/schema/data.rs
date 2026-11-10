//! The `data` of an `ok` envelope, per command (notation: see `super`).

/// Commands whose `data` is not described yet (any object, `{..}`), comma-separated.
pub const UNDESCRIBED: &str = "join, reap, wave plan, msg, keygen, test-integrity, review-pack, ci-log, ask, hold, \
    answer, lesson add, lesson hit, lesson capture, lesson triage, doctor, budget record, \
    budget check, budget report, plan check, plan import, plan amend, plan pr, fail, \
    resume, handoff, handoff-pr, progress, result, main-triage, init, adopt, uninstall, \
    pin, setup-snippet, revert, trailers, intake, intake check, intake sign, intake verify, \
    main-health, gate-missed, review-ci, notify, prereqs";

/// `(command, data)` of the described commands. Hooks print the hook protocol, no envelope.
pub const DATA: &[(&str, &str)] = &[
    (
        "claim",
        "{comment_id: int, issue: int, lease: {expires: str, session: str, touch: [str]}, lessons: [str], added?: [str]}",
    ),
    ("heartbeat", "{leases: [int], registry: bool}"),
    ("release", "{comment_id: int, issue: int, state: str}"),
    (
        "spawn",
        r#"{checks_passed?: int, epic?: int, model?: str, child?: str, from?: str, root?: str,
        environment_id?: str | null, handoff?: int, relay_waves?: int, then?: str, thread_ts?: str,
        intake?: int, warning?: str}"#,
    ),
    (
        "wait",
        r#"{changed: bool, polls: int, before: {..}, after: {..},
        heartbeat: {beats: int, error: str | null, leases: [int], left: [str],
            reaped: [{issue: int, session: str}], registry: bool}}"#,
    ),
    (
        "gate",
        r#"{gate?: str, stack?: str | null, command?: [str], packages?: [str], exit_status?: int,
        secs?: num, over_budget?: bool, skipped?: bool, reason?: str, stamp?: str | null,
        uncommitted?: [str], files?: [str], findings?: [any], base?: str, files_checked?: int,
        integrity?: str, head_sha?: str, main_red?: int | null, mainfix?: bool, pass?: bool,
        pr?: int, ci?: {base: str, event: str, provider: str},
        runs?: [{gate: str, stack: str | null, command: [str], packages: [str], exit_status: int,
            secs: num, over_budget: bool, skipped: bool}]}"#,
    ),
    (
        "approve",
        r#"{approved?: [{pr: int, head_sha: str, comment_id: int, repost?: bool,
            after_review?: int, key_id?: str, policy?: str | null, role?: str}],
        approve_command?: str | null, auth?: str, cloud?: bool, hint?: str,
        message?: str | null, message_id?: str | null, message_pairs?: [str] | null,
        ready?: [{pr: int, title: str, head_sha: str, files: [str], files_truncated: bool,
            risk_from_status: [str]}],
        skipped?: [{pr: int | null, why: str}], waiting?: [{pr: int, why: str}],
        skipped_fix?: str, main_open?: {head_sha: str, pr: int},
        bodies?: [{after_review: int | null, body: str}], body?: str,
        bootstrap_until?: str | null, head_sha?: str, key_id?: str, policy?: str | null,
        pr?: int, role?: str}"#,
    ),
    (
        "risk-check",
        r#"{pr: int, head_sha: str, level: str,
        matches: [{rule: str, why: str, approval: str, evidence: [str]}],
        out_of_touch: [str] | null, approved_by: str | null,
        approval_carried_from: str | null, verdict_carried_from: str | null,
        requirement?: str, policy?: str, verdicts?: [str], autonomous_verdicts?: [str],
        delegate_verdicts?: [str], delegate_blocked?: str | null}"#,
    ),
    ("verdict post", VERDICT),
    (
        "verdict comment",
        "{pr: int, head_sha: str, comment_id: int, applied_by: str}",
    ),
    ("verdict apply", VERDICT),
    (
        "pr",
        r#"{pr: int, created: bool, draft: bool, head_sha: str, auto_merge: bool,
        task_state: str | null, ready?: bool | "pending", merged?: bool, epic_closed?: bool,
        auto_merge_route?: str | null, enforced_checks?: str | null, note?: str,
        touch_check?: str, gates?: [{context: str, state: str}],
        pending_checks?: [{context: str, state: str}],
        trailers?: {checked: int, required: bool, unread: int,
            missing: [{sha: str, missing: [str]}]}}"#,
    ),
    (
        "status",
        r#"{repo: str, session: str, origin: str, halted: bool, integrity: str,
        integrity_reason?: str, main_red: int | null, process_red: int | null,
        issues_by_state: {[str]: int}, leases: [{issue: int, expires: str, session: str, touch: [str]}],
        live_sessions: [{session: str, origin: str, area: str | null, depth: int, expires: str}],
        budget_pct: num, daily_usd: num, spend_today_usd: num}"#,
    ),
    (
        "schema",
        "{format: \"json\", command?: str, schema?: {..}, envelope?: {..}, commands?: {[str]: any}}",
    ),
];

const VERDICT: &str =
    "{pr: int, head_sha: str, approved: bool, status: str, reasons: [str], task_state: str | null}";
