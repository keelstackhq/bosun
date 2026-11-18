//! What the PreToolUse hook refuses (spec §4 edit hook, label-lock; D13 merge-lock). Pure: the
//! hook passes the tool call, the repo-relative path, the current branch and the live leases.

use std::cell::{Cell, RefCell};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::core::glob;
use crate::core::lease::{Conflict, Lease};
use crate::core::meta::OPEN;

mod interp;
mod literals;
mod prose;
pub use interp::{Script, script_rules};
mod writes;
pub use writes::bash_writes;

const MERGE_FIX: &str = "agents never merge, tag or force-push (spec D13): open the PR with \
    `bosun pr <issue>` and let `bosun pr <issue> --auto-merge` hand it to GitHub after the gates \
    (a PR with no task: `bosun pr --pr <n> --auto-merge`)";

/// Whether a refusal reason is about a merge, tag or push, so that the "never merge, tag or
/// force-push" advice fits it. Reasons that carry their own fix, or are about something else,
/// do not get it.
fn tail_applies(why: &str) -> bool {
    const OTHER: [&str; 6] = [
        "xargs",
        "cannot see",
        "alias",
        "secrets and variables",
        "awk",
        "workflow run",
    ];
    why != PUSH_CONFIG && why != UNREAD_PUSH && !OTHER.iter().any(|o| why.contains(o))
}

/// The tools that write one file, and the input key that names it.
pub fn edited_path(tool: &str, input: &Value) -> Option<String> {
    let key = match tool {
        "Edit" | "Write" | "MultiEdit" => "file_path",
        "NotebookEdit" => "notebook_path",
        _ => return None,
    };
    input.get(key)?.as_str().map(str::to_string)
}

/// Edit-lock: a path inside a live lease of another session. The edited path is a literal
/// path (`app/[id]/page.tsx` has brackets), so each lease glob is matched against it and it is
/// never parsed as a glob itself. A lease with a bad glob covers everything. An agent may edit
/// the paths its own orchestrator leased (`S/task` runs inside `S`); see spec §4. A session that
/// holds a lease with a touch list must also stay inside it (`own_touch_lock`); `exists` says
/// whether the path is already a file (the hook checks, core stays pure).
pub fn edit_lock(path: &str, exists: bool, leases: &[Lease], me: &str) -> Option<String> {
    foreign_lock(path, leases, me).or_else(|| own_touch_lock(path, exists, leases, me))
}

/// The leases held by exactly `me` that list a touch. The hook sees only the plain session id,
/// so a per-task lease (`S/x`) never restricts `S`, and a wave orchestrator edits freely.
fn own_leases<'a>(leases: &'a [Lease], me: &str) -> Vec<&'a Lease> {
    leases
        .iter()
        .filter(|l| l.session == me && !l.touch.is_empty())
        .collect()
}

/// An existing file that no own glob (nor a lockfile companion) covers is refused. Fragments and
/// new files are free. A bad glob in an own lease covers nothing.
fn own_touch_lock(path: &str, exists: bool, leases: &[Lease], me: &str) -> Option<String> {
    let own = own_leases(leases, me);
    let first = own.first()?;
    if !exists || crate::core::touch::is_fragment(path) {
        return None;
    }
    let touch: Vec<String> = own.iter().flat_map(|l| l.touch.clone()).collect();
    let all = crate::core::touch::companions(&touch);
    let valid: Vec<glob::Glob> = all
        .iter()
        .filter_map(|g| glob::Glob::parse(g).ok())
        .collect();
    if valid.iter().any(|g| g.matches(path)) {
        return None;
    }
    let bad = if valid.len() < all.len() {
        " One of your globs is invalid: re-claim with valid globs."
    } else {
        ""
    };
    Some(format!(
        "{path} is outside your lease (touch: {}). Add it with `bosun claim {} --touch {path}` \
         (exit 3 if another session holds it), or leave the file alone.{bad}",
        touch.join(", "),
        first.issue
    ))
}

/// The first half of `edit_lock`: the path lies in a live lease of another session.
pub fn foreign_lock(path: &str, leases: &[Lease], me: &str) -> Option<String> {
    let hit = leases
        .iter()
        .filter(|l| !is_mine(&l.session, me))
        .find_map(|l| {
            let theirs = match glob::parse_all(&l.touch) {
                Ok(globs) => globs.iter().find(|g| g.matches(path))?.as_str().to_string(),
                Err(_) => "**".to_string(),
            };
            Some(Conflict {
                issue: l.issue,
                session: l.session.clone(),
                mine: path.to_string(),
                theirs,
                expires: l.expires,
            })
        })?;
    Some(format!(
        "{path} is claimed: {}. Claim your own task with `bosun claim`, or wait for that lease \
         (`bosun wait --issue {}`)",
        hit.describe(),
        hit.issue
    ))
}

/// A lease holder whose files `me` may edit: itself, its orchestrator (`S` for `S/task`), or a
/// per-task session of its own (`S/19` for the hook, which sees only the plain `S`).
fn is_mine(holder: &str, me: &str) -> bool {
    let orch = crate::core::lease::orchestrator_of;
    holder == me || holder == orch(me) || orch(holder) == me
}

/// The checked-out branch of the git target a command names, given as `git` global options
/// (`-C dir`, `--git-dir=..`, `--work-tree=..`) applied after the hook's own directory; an
/// empty list is the hook's directory. `None` when it cannot be read.
pub type BranchAt<'a> = &'a dyn Fn(&[String]) -> Option<String>;

/// The read-only tools of the GitHub MCP server (names without `mcp__github__`), which never set a
/// label. A new tool is not on the list until someone reads what it does.
const READ_ONLY_GITHUB: &[&str] = &[
    "get_me",
    "get_file_contents",
    "get_commit",
    "get_label",
    "get_tag",
    "get_latest_release",
    "get_release_by_tag",
    "get_check_run",
    "get_job_logs",
    "get_team_members",
    "get_teams",
    "get_copilot_job_status",
    "list_branches",
    "list_commits",
    "list_issues",
    "list_issue_types",
    "list_issue_fields",
    "list_pull_requests",
    "list_releases",
    "list_tags",
    "list_repository_collaborators",
    "actions_list",
    "actions_get",
    "search_code",
    "search_commits",
    "search_issues",
    "search_pull_requests",
    "search_repositories",
    "search_users",
    "issue_read",
    "pull_request_read",
];

/// Label-lock and merge-lock; `None` allows the call. `branch_at` reads the checked-out branch
/// of the directory a git command acts on (after `cd dir &&` and `git -C dir`), and
/// `default_branch` is the repo's default branch.
pub fn tool_rules(
    tool: &str,
    input: &Value,
    branch_at: BranchAt,
    default_branch: &str,
) -> Option<String> {
    if tool == "Bash" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        return bash_rules(command, branch_at, default_branch);
    }
    // Any other MCP server's write tool is refused when it carries the chat form too: only
    // read-only verbs (`get_`, `list_`, `search_`, ...) are skipped.
    if let Some(rest) = tool.strip_prefix("mcp__")
        && !tool.starts_with("mcp__github__")
    {
        let name = rest.rsplit("__").next().unwrap_or(rest);
        let read_only = [
            "get_", "list_", "search_", "read_", "find_", "query_", "fetch_",
        ]
        .iter()
        .any(|v| name.starts_with(v));
        if !read_only && strings(input).iter().any(|t| carries_chat_form(t)) {
            return Some(format!(
                "`{tool}` writes a chat approval record: {CHAT_FIX}"
            ));
        }
    }
    if let Some(name) = tool.strip_prefix("mcp__github__") {
        if matches!(name, "merge_pull_request" | "enable_pr_auto_merge") {
            return Some(format!("`{tool}` is refused: {MERGE_FIX}"));
        }
        // Reading or listing sets no label, even when the filter names a state label. Only known
        // read-only tools are skipped; a tool not listed is judged below (fails closed).
        if READ_ONLY_GITHUB.contains(&name) {
            return None;
        }
        if strings(input).iter().any(|t| carries_chat_form(t)) {
            return Some(format!(
                "`{tool}` writes a chat approval record: {CHAT_FIX}"
            ));
        }
        // A label tool is judged on its whole input; any other tool only on its label fields,
        // so a PR or issue body that names a state label in prose is not a label edit.
        let set = if name.contains("label") {
            input.to_string()
        } else {
            label_fields(input)
        };
        if set.contains(STATE) {
            return Some(format!(
                "`{tool}` sets a {STATE}* label and is refused: {LABEL_FIX}"
            ));
        }
    }
    None
}

/// The marker syntax bosun's parsers read (owner checklist `bosun:prereq|prereqs`, task and epic
/// metadata): an HTML comment opener, then the name, then a space, `{` or the comment end.
/// Prose that only mentions `bosun:task`, and `bosun:task-pr`, are not markers.
static MARKER: LazyLock<Regex> = LazyLock::new(|| marker_re(&format!("{NOT_TASK}|task")));

/// The marker names a registered child may not write; [`MARKER`] is these plus `task`, so the
/// two cannot drift.
const NOT_TASK: &str = "prereqs?|epic";

fn marker_re(names: &str) -> Regex {
    Regex::new(&format!(r"<!--\s*bosun:(?:{names})(?:[\s{{]|-->|$)")).expect("marker regex")
}

/// MCP tools that write an issue body or comment, and the input fields that hold the text.
const ISSUE_TEXT_TOOLS: [&str; 4] = [
    "issue_write",
    "update_issue",
    "add_issue_comment",
    "update_issue_comment",
];

/// The same without `task`: what a registered child may not write either.
static MARKER_NOT_TASK: LazyLock<Regex> = LazyLock::new(|| marker_re(NOT_TASK));

/// `task_ok`: the `task` marker does not count (a registered child may write it).
fn has_marker(text: &str, task_ok: bool) -> bool {
    if task_ok { &MARKER_NOT_TASK } else { &MARKER }.is_match(text)
}

/// What a registered child may write: one new issue carrying one `task` marker.
#[derive(Debug, PartialEq)]
pub struct ChildTask {
    /// The epic the marker names; the child must hold its spawn lease.
    pub epic: u64,
    /// The `(owner, repo)` an MCP call targets; `None` for `gh issue create` (this repo).
    pub repo: Option<(String, String)>,
}

/// Metadata keys a child may set. `allow` and `capabilities` widen what the gates let the task
/// do (test-integrity grants, the prod-write floor), so only a plan PR may set them.
const CHILD_KEYS: [&str; 7] = ["item", "task", "epic", "touch", "depends", "kind", "risk"];

/// Is `write` a registered child's creation of ONE new issue with ONE well-formed `task`
/// marker? Only `gh issue create` without `-R/--repo` (no nested shell, no other issue write in
/// the command) and the MCP `issue_write` with method `create` qualify; edits, comments, API
/// calls and other repos never do. The marker must be JSON with only [`CHILD_KEYS`] and an
/// `epic`; the caller checks that the child's spawn lease is on that epic.
pub fn child_task_create(tool: &str, input: &Value) -> Result<ChildTask, String> {
    let (text, repo) = match tool {
        "mcp__github__issue_write" => {
            let create = input.get("method").and_then(Value::as_str) == Some("create")
                && input.get("issue_number").is_none();
            if !create {
                return Err("a child may only create a new issue with the task marker".into());
            }
            let field = |k: &str| input.get(k).and_then(Value::as_str).map(String::from);
            let repo = field("owner").zip(field("repo"));
            (
                field("body").unwrap_or_default(),
                Some(repo.ok_or("the call names no owner/repo")?),
            )
        }
        "Bash" => {
            let command = input.get("command").and_then(Value::as_str).unwrap_or("");
            if !nested(command).is_empty() {
                return Err("a child may not write the marker through a nested shell".into());
            }
            let words = shell_words(command);
            // The whole command is the one `gh issue create`: no `cd dir &&` or other segment
            // that could change which repo `gh` acts on, and no `$` or backtick that expands to
            // text the checks below never see (`$'\x3c'`, `$VAR`).
            let segs: Vec<&[String]> = words.split(|w| w == SEP).collect();
            let [seg] = segs.as_slice() else {
                return Err("a child's command is the single `gh issue create`".into());
            };
            if !writes_issue_text(seg) {
                return Err("a child may only run `gh issue create` with the marker".into());
            }
            if seg.iter().any(|w| w.contains('$') || w.contains('`')) {
                return Err("a child's `gh issue create` may not use `$` or backticks".into());
            }
            // `gh` itself must open the command: not `./gh` or a path, and no variable prefix
            // (`GH_REPO=evil/x gh ...`, `GH_HOST=`) or wrapper that redirects it.
            let at = seg.first().filter(|w| w.as_str() == "gh").map(|_| 0);
            let ok = at.is_some_and(|at| {
                seg.get(at + 1).map(String::as_str) == Some("issue")
                    && seg.get(at + 2).map(String::as_str) == Some("create")
            });
            if !ok {
                return Err("a child may only run `gh issue create` with the marker".into());
            }
            if seg
                .iter()
                .any(|a| a == "-R" || a.starts_with("-R") || a.starts_with("--repo"))
            {
                return Err("a child may only create issues in this repo (no -R/--repo)".into());
            }
            (seg.join(" "), None)
        }
        _ => return Err("a child may only create an issue with the task marker".into()),
    };
    if MARKER_NOT_TASK.is_match(&text) || MARKER.find_iter(&text).count() != 1 {
        return Err("a child may write exactly one task marker and no other".into());
    }
    // The marker bosun itself reads (`core::meta::parse_task_meta`): the last `OPEN`, up to the
    // next `-->`. Prose that merely names `bosun:task` cannot stand in for it, and a second
    // `OPEN` is refused, so the JSON checked here is the JSON bosun parses.
    if text.matches(OPEN).count() != 1 {
        return Err("a child may write exactly one `<!-- bosun:task {...} -->` marker".into());
    }
    let after = text
        .rfind(OPEN)
        .map(|i| &text[i + OPEN.len()..])
        .and_then(|r| r.find("-->").map(|e| r[..e].trim()))
        .ok_or("the task marker has no closing -->")?;
    let meta: Value = serde_json::from_str(after).map_err(|e| format!("task marker JSON: {e}"))?;
    let obj = meta
        .as_object()
        .ok_or("the task marker is not a JSON object")?;
    if let Some(k) = obj.keys().find(|k| !CHILD_KEYS.contains(&k.as_str())) {
        return Err(format!(
            "a child may not set `{k}` in the task marker (allow and capabilities come from a plan PR)"
        ));
    }
    // Kinds with a lighter gate path (a main fix, a process fix) are for a plan PR to grant.
    if let Some(kind) = obj.get("kind").and_then(Value::as_str)
        && !["feature", "bookkeeping", "flake"].contains(&kind)
    {
        return Err(format!(
            "a child may not set kind `{kind}` (only feature, bookkeeping or flake)"
        ));
    }
    let epic = obj
        .get("epic")
        .and_then(Value::as_u64)
        .ok_or("the task marker needs the `epic` you were spawned for")?;
    Ok(ChildTask { epic, repo })
}

/// Does one simple command (words, quotes removed) write an issue body or comment through
/// `gh`, `hub` or `curl`?
fn writes_issue_text(words: &[String]) -> bool {
    let name = |w: &String| w.rsplit('/').next().unwrap_or(w).to_string();
    let Some(at) = words
        .iter()
        .position(|w| ["gh", "hub", "curl"].contains(&name(w).as_str()))
    else {
        return false;
    };
    let args: Vec<&str> = words[at + 1..].iter().map(String::as_str).collect();
    let has = |f: &[&str]| args.iter().any(|a| f.contains(a));
    let flag = |long: &str| args.iter().any(|a| a.starts_with(&format!("{long}=")));
    // `-b`, `-b TEXT`, `-bTEXT`, `--body`, `--body=TEXT` (not `--body-file`).
    let body = has(&["--body"])
        || flag("--body")
        || args
            .iter()
            .any(|a| a.starts_with("-b") && !a.starts_with("--"));
    // gh and curl use the last method flag; any write method among them counts as a write.
    let methods: Vec<&str> = args
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            if ["-X", "--method", "--request"].contains(a) {
                args.get(i + 1).copied()
            } else {
                a.strip_prefix("--method=")
                    .or_else(|| a.strip_prefix("--request="))
                    .or_else(|| a.strip_prefix("-X"))
            }
        })
        .collect();
    let write = |v: &str| ["POST", "PATCH", "PUT"].contains(&v.to_ascii_uppercase().as_str());
    let explicit = (!methods.is_empty()).then(|| methods.iter().any(|m| write(m)));
    if name(&words[at]) == "curl" {
        return explicit == Some(true)
            || args.iter().any(|a| {
                a.starts_with("-d") || a.starts_with("--data") || a.starts_with("--json")
            });
    }
    match args.first().copied() {
        Some("issue") => has(&["edit", "comment", "create", "close"]) && body,
        Some("pr") => has(&["comment"]) && body,
        Some("api") if args.get(1) == Some(&"graphql") => args
            .iter()
            .any(|a| a.to_ascii_lowercase().contains("mutation")),
        Some("api") => match explicit {
            Some(w) => w,
            None => {
                has(&["-f", "-F", "--field", "--raw-field"])
                    || flag("--field")
                    || flag("--raw-field")
            }
        },
        _ => false,
    }
}

/// Does `command`, or a command nested in it (`bash -c`, `eval`, `$( )`, groups), write issue
/// text that carries a marker?
fn writes_marker(command: &str, depth: usize, task_ok: bool) -> bool {
    depth <= MAX_DEPTH
        && (shell_words(command)
            .split(|w| w == SEP)
            .any(|seg| writes_issue_text(seg) && has_marker(&seg.join(" "), task_ok))
            || nested(command)
                .iter()
                .any(|n| writes_marker(n, depth + 1, task_ok)))
}

/// Markers-lock (design F4): refuses a write of an issue body or comment that carries a bosun
/// marker (`MARKER`: `<!-- bosun:prereq|prereqs|task|epic`; prose that names one is allowed).
/// Only writes are judged: `gh|hub issue edit|comment|create|close` and `gh pr comment` with
/// `--body`/`-b` (also `-bTEXT`), `gh|hub api` with a write method or field flag (never with an
/// explicit GET; with several method flags any write method counts; `gh api graphql` only with a
/// `mutation`), `curl` with a write method or a data
/// flag (`-d`, `--data*`, `--json`), the same inside `bash -c`, `eval` and `$( )`, and the MCP
/// tools in `ISSUE_TEXT_TOOLS` (their `body` and `comment` fields only). Best effort. Known and
/// accepted bypasses: `--body-file`, `--input`, `curl` or `gh api` with the text in a variable or
/// a file, a marker split by quotes (`'bosun:'"task"`) or built by `printf` in `$( )`, a
/// `gh api graphql -F query=@file` mutation (the text is in the file), wget,
/// httpie and python urllib. The real guarantees are elsewhere: bosun re-reads the checklist and
/// trusts only its own writes.
pub fn prereqs_lock(tool: &str, input: &Value) -> Option<String> {
    prereqs_lock_for(tool, input, false)
}

/// [`prereqs_lock`] for a caller that may write the `task` marker (`task_ok`: a registered child
/// session, spec §4): the checklist and epic markers stay refused.
pub fn prereqs_lock_for(tool: &str, input: &Value, task_ok: bool) -> Option<String> {
    let hit = match tool.strip_prefix("mcp__github__") {
        Some(name) => {
            ISSUE_TEXT_TOOLS.contains(&name)
                && ["body", "comment"]
                    .iter()
                    .filter_map(|k| input.get(k).and_then(Value::as_str))
                    .any(|t| has_marker(t, task_ok))
        }
        None if tool == "Bash" => {
            let command = input.get("command").and_then(Value::as_str).unwrap_or("");
            has_marker(command, task_ok)
                && !read_only(command, 0)
                && writes_marker(command, 0, task_ok)
        }
        None => false,
    };
    hit.then(|| {
        "bosun's markers (owner checklist, task and epic metadata) are written by bosun only \
         (bosun prereqs --post writes the checklist); fix the plan instead"
            .to_string()
    })
}

const CHAT_FIX: &str = "only an orchestrator records a chat approval: run \
    `bosun approve <pr>@<head> --chat` (prose that only names the marker: put it in a file, \
    `--body-file` or `git commit -F`)";

/// The chat approval marker, or the record's first line `/bosun approve <40-hex> chat`, anywhere
/// in the text (any case, any spacing). Broader than `core::approve::parse_chat_record`, so
/// whatever the risk-check counter would count, the guard refuses.
static CHAT_FORM: LazyLock<Regex> = LazyLock::new(|| {
    let marker = regex::escape(crate::core::approve::CHAT_MARKER);
    Regex::new(&format!(
        r"(?i){marker}|/bosun\s+approve\s+[0-9a-f]{{40}}\s+chat\b"
    ))
    .expect("chat form regex")
});

fn carries_chat_form(text: &str) -> bool {
    CHAT_FORM.is_match(text) || CHAT_FORM.is_match(&decode_escapes(text))
}

/// `text` with `\uXXXX` and `\xNN` escapes (JSON, printf, `$'..'`) replaced by their character,
/// so a record cannot be spelled past the matcher.
fn decode_escapes(text: &str) -> String {
    static ESC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\\u([0-9a-fA-F]{4})|\\x([0-9a-fA-F]{2})").expect("escape regex")
    });
    ESC.replace_all(text, |c: &regex::Captures| {
        let hex = c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str());
        u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .map_or_else(String::new, |ch| ch.to_string())
    })
    .into_owned()
}

/// Every string value in `v`, at any depth.
fn strings(v: &Value) -> Vec<&str> {
    match v {
        Value::String(s) => vec![s.as_str()],
        Value::Object(m) => m.values().flat_map(strings).collect(),
        Value::Array(a) => a.iter().flat_map(strings).collect(),
        _ => Vec::new(),
    }
}

/// Chat-lock (D-018): a command that carries the chat form is refused unless it is read-only
/// throughout with no `bosun` word (`grep bosun:chat-approval src` passes). The legitimate
/// `bosun approve <pr>@<head> --chat` carries no form: bosun writes the record itself. Best
/// effort, like the label-lock: text in a file (`--body-file`, `--input`), a variable, or split
/// by quotes is missed.
fn chat_rule(command: &str) -> Option<String> {
    if !carries_chat_form(command) {
        return None;
    }
    let reads = read_only(command, 0) && !shell_words(command).iter().any(|w| base(w) == "bosun");
    (!reads).then(|| format!("the command writes a chat approval record: {CHAT_FIX}"))
}

/// [`chat_rule`] for the text of a script file: a script is never "read-only throughout", so
/// any script carrying the form is refused.
pub(super) fn chat_rule_script(code: &str) -> Option<String> {
    carries_chat_form(code).then(|| format!("it writes a chat approval record: {CHAT_FIX}"))
}

/// The command with quotes and backslashes removed (`bos''un`, `bo\\sun` read as `bosun`) and
/// split into words at whitespace and shell separators.
fn dequoted_words(command: &str) -> Vec<String> {
    let plain: String = command
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    plain
        .split(|c: char| c.is_whitespace() || ";&|()`$<>".contains(c))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether the command may run `bosun approve … --chat`: the words `approve` and `--chat` both
/// appear (dequoted, nested or not). No `bosun` word is required, so a spelling of the command
/// name cannot slip past. Over-matches on purpose: the hook lets only an orchestrator through.
pub fn records_chat_approval(command: &str) -> bool {
    let words = dequoted_words(command);
    ["approve", "--chat"]
        .iter()
        .all(|w| words.iter().any(|x| x == w))
}

/// Whether a command that records a chat approval also sets its own identity
/// (`BOSUN_SESSION=…`, `BOSUN_ORIGIN=…`, `CLAUDE_CODE_…=…`): refused for everyone.
pub fn chat_identity_override(command: &str) -> bool {
    records_chat_approval(command)
        && dequoted_words(command).iter().any(|w| {
            let name = w.split('=').next().unwrap_or("");
            w.contains('=') && (name.starts_with("BOSUN_") || name.starts_with("CLAUDE_CODE_"))
        })
}

/// Whether a command drops or overrides the child-session flag next to a `bosun` word: an
/// unset of it, a cleared environment, or an assignment to it. Best effort (lexical, like the
/// rest of the chat-lock): refused for everyone, since no honest command needs to hide that it
/// runs as a child. `-u`, `-i` and `-` count only as options of an `env` word (its options end
/// at the first word that is neither an option nor `NAME=value`), and `unset` only as the first
/// word of a command, so `grep -i x && bosun status` is not tamper.
pub fn child_flag_tamper(command: &str) -> bool {
    let words = dequoted_words(command);
    if !words.iter().any(|w| base(w) == "bosun") {
        return false;
    }
    let flag = |w: &String| w.contains("CLAUDE_CODE_CHILD_SESSION");
    if words.iter().any(|w| flag(w) && w.contains('=')) {
        return true;
    }
    command
        .split(|c: char| ";&|\n()`".contains(c))
        .map(dequoted_words)
        .any(|seg| {
            let named = seg.iter().any(flag);
            if seg.first().is_some_and(|w| w == "unset") && named {
                return true;
            }
            let Some(at) = seg.iter().position(|w| base(w) == "env") else {
                return false;
            };
            let (mut unsets, mut clears, mut skip) = (false, false, false);
            for w in &seg[at + 1..] {
                if std::mem::take(&mut skip) {
                    continue;
                }
                if w == "-u" {
                    unsets = true;
                    skip = true;
                } else if w.starts_with("-u") || w.starts_with("--unset") {
                    unsets = true;
                } else if matches!(w.as_str(), "-i" | "--ignore-environment" | "-") {
                    clears = true;
                } else if !w.starts_with('-') && !w.contains('=') {
                    break;
                }
            }
            (unsets && named) || clears
        })
}

/// The values under any key whose name contains `label`, at any depth, as JSON text.
fn label_fields(v: &Value) -> String {
    match v {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| {
                if k.to_ascii_lowercase().contains("label") {
                    v.to_string()
                } else {
                    label_fields(v)
                }
            })
            .collect(),
        Value::Array(a) => a.iter().map(label_fields).collect(),
        _ => String::new(),
    }
}

const STATE: &str = "bosun:state/";
const LABEL_FIX: &str = "state labels change only through bosun commands (spec §4); use \
    `bosun claim`, `bosun release`, `bosun pr` or `bosun verdict post|comment`";

fn label_reason() -> String {
    format!(
        "a command that names a {STATE}* label may run only read-only commands (grep, cat, jq, \
         git show|log|diff, gh issue|pr list|view, gh search, bosun ...) and no `$` expansion: \
         {LABEL_FIX}"
    )
}

/// Nested commands are checked first (merge-lock; the label-lock judges them too), to this depth:
/// each `$( )`, backtick span, `( )` group (jq parens too: quotes are not parsed), `sh -c` and
/// `eval` is a level. Deeper commands are refused with `too_deep`.
const MAX_DEPTH: usize = 8;

fn too_deep() -> String {
    format!(
        "commands nested deeper than {MAX_DEPTH} levels (`$( )`, backticks, `( )` groups, \
         `sh -c`, `eval`) are refused, since the guard cannot judge them: run the inner \
         commands first and pass their results in variables"
    )
}

fn bash_rules(command: &str, branch_at: BranchAt, default_branch: &str) -> Option<String> {
    chat_rule(command)
        .or_else(|| decrypt_rule(command, 0))
        .or_else(|| destructive_rule(command, 0))
        .or_else(|| check(command, branch_at, default_branch, 0, None))
        .or_else(|| {
            let prose = prose::strip_prose(command);
            (prose.contains(STATE) && !read_only(&prose, 0)).then(label_reason)
        })
}

/// A `git push` the merge-lock lets through (push-gates, spec §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    /// `-C` options of the shell's `cd`s and git's own `-C`; `None`: the guard cannot tell.
    pub dir: Option<Vec<String>>,
    /// The local refs pushed (refspec sources); empty: the current branch.
    pub srcs: Vec<String>,
}

/// Where the walk reports each push it lets through.
pub(crate) type PushSink<'a> = &'a dyn Fn(Push);

/// The sinks of shell pushes (their `cd`s are tracked) and of pushes found in interpreter code
/// (whose directory the scan cannot tell).
#[derive(Clone, Copy)]
pub(crate) struct Sinks<'a> {
    pub shell: PushSink<'a>,
    pub code: PushSink<'a>,
    /// A push read in quotes or a `( )` group, which the nested walk may report with its `cd`.
    pub outer: PushSink<'a>,
}

/// Every `git push` of `command` that the merge-lock allows, in shell scripts it runs and code
/// it runs too (the same walk as the merge-lock). A refused command may list fewer. Interpreter
/// code is scanned as words, so a push found only there is one push whose directory is unknown;
/// when the shell walk found pushes, the code scan (which sees the same text) adds none.
pub fn pushes(
    command: &str,
    read: &dyn Fn(&str) -> Option<Script>,
    default_branch: &str,
) -> Vec<Push> {
    let (shell, code, outer) = (
        RefCell::new(Vec::new()),
        Cell::new(false),
        RefCell::new(vec![]),
    );
    let sinks = Sinks {
        shell: &|p: Push| shell.borrow_mut().push(p),
        code: &|_: Push| code.set(true),
        outer: &|p: Push| outer.borrow_mut().push(p),
    };
    // The branch only decides refusals, which `tool_rules` already made.
    let branch_at = |_: &[String]| Some(String::new());
    let _ = check(command, &branch_at, default_branch, 0, Some(&sinks));
    let _ = interp::script_walk(command, read, &branch_at, default_branch, Some(&sinks));
    let shell = shell.into_inner();
    // An outer push is dropped only when the nested walk reported the same refs.
    let outer = outer.into_inner().into_iter();
    let kept = outer.filter(|p| !shell.iter().any(|s| s.srcs == p.srcs));
    let mut out: Vec<Push> = Vec::new();
    for p in shell.iter().cloned().chain(kept.collect::<Vec<_>>()) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    if out.is_empty() && code.get() {
        out.push(Push {
            dir: None,
            srcs: Vec::new(),
        });
    }
    out
}

fn check(
    command: &str,
    branch_at: BranchAt,
    default_branch: &str,
    depth: usize,
    sinks: Option<&Sinks>,
) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(too_deep());
    }
    // A fetch or pull next to other shell commands reads an environment the guard cannot see.
    if let Some(why) = fetch_not_plain(command)
        .filter(|_| depth == 0)
        .or_else(|| glob_rule(command, depth == 0))
    {
        return Some(format!("`{}` is refused ({why})", command.trim()));
    }
    for inner in nested(command) {
        if let Some(why) = check(&inner, branch_at, default_branch, depth + 1, sinks) {
            return Some(why);
        }
    }
    let here = Target {
        dir: Some(&[]),
        alone: false,
        branch_at,
        default_branch,
        sink: sinks.map(|s| s.code),
    };
    if let Some(why) = interp::inline_rule(&split_heredocs(command).0, &here) {
        return Some(why);
    }
    // `cd` moves later git commands: `Some(args)` as `-C` options, `None` when the directory
    // cannot be told (git commands then count as on the default branch: fail closed). A `cd`
    // counts only when the shell surely runs it in this shell: after the start, `;` or a
    // newline, or inside an `&&` chain that the git command continues; never in a pipeline
    // or a `( .. )` group. `GIT_DIR`, `GIT_WORK_TREE` or `core.worktree` make it unknown.
    let mut dir: Option<Vec<String>> = Some(Vec::new());
    let (mut prev, mut chained) = ("", false);
    for (segment, sep) in segments_with_sep(command) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        if ["GIT_DIR", "GIT_WORK_TREE", "core.worktree"]
            .iter()
            .any(|v| segment.contains(v))
        {
            dir = None;
        }
        let start = segment.as_ptr() as usize - command.as_ptr() as usize;
        let before = &command[..start];
        if words.first() == Some(&"popd") {
            dir = None;
        } else if matches!(words.first(), Some(&("cd" | "pushd"))) {
            let grouped = before.matches('(').count() > before.matches(')').count()
                || segment.contains(['(', ')']);
            let runs = matches!(prev, "" | ";" | "\n" | "&&");
            let keeps = matches!(sep, "&&" | ";" | "\n" | "");
            dir = match (dir, cd_target(&words[1..])) {
                (Some(mut d), Some(t)) if runs && keeps && !grouped => {
                    d.extend(["-C".to_string(), t]);
                    chained |= prev == "&&";
                    Some(d)
                }
                _ => None,
            };
        } else {
            let target = Target {
                dir: dir.as_deref(),
                alone: depth == 0 && alone(command),
                branch_at,
                default_branch,
                // Inside quotes (`bash -c '..'`) or a `( )` group, the nested walk may report
                // the push with its own `cd`s; `pushes` keeps this one only if it does not.
                sink: sinks.map(|s| {
                    let opens = before.matches('(').count() > before.matches(')').count();
                    if opens || in_quotes(before) {
                        s.outer
                    } else {
                        s.shell
                    }
                }),
            };
            // A command name the shell expands (`$g`, `${G:-gh}`, `$(echo gh)`, a backtick) cannot
            // be read. Text inside quotes or a heredoc is data, not a command.
            let named = words
                .iter()
                .find(|w| !(w.contains('=') && !w.starts_with('-')) && !WRAPPERS.contains(w));
            if depth == 0
                && !in_quotes(before)
                && !command.contains("<<")
                && let Some(w) =
                    named.filter(|w| w.trim_start_matches(['"', '\'']).starts_with(['$', '`']))
                && let Some(name) = built_name_rule(w, segment, &target)
            {
                return Some(format!(
                    "`{}` is refused (its command name is built by the shell and could be \
                     `{name}`, which agents may not run to merge, tag or push): write the \
                     command name out, not a variable or substitution",
                    segment.trim()
                ));
            }
            if matches!(prev, "|" | "|&") && reads_pipe(&words) {
                return Some(format!(
                    "`{}` is refused (a script piped into a shell cannot be read): download it \
                     to a file, read it, then run the file",
                    segment.trim()
                ));
            }
            if let Some(why) = merge_rule(&words, segment, &target) {
                if why == PUSH_ALONE {
                    let fix = alone_fix(&words);
                    return Some(format!("`{}` is refused ({fix})", segment.trim()));
                }
                return Some(format!(
                    "`{}` is refused ({why}){}",
                    segment.trim(),
                    if why.contains("awk") {
                        ": run the command itself, not through awk's system()".to_string()
                    } else if tail_applies(why) {
                        format!(": {MERGE_FIX}")
                    } else {
                        String::new()
                    }
                ));
            }
        }
        // A `cd` that ran only if an `&&` chain held may have been skipped once the chain
        // ends: later commands then run where the shell started.
        if chained && !matches!(sep, "&&" | "") {
            dir = None;
        }
        prev = sep;
    }
    None
}

/// Label-lock (spec §4): a guard against accidental or casual state-label edits, not a
/// sandbox. A command that names a state label must be read-only throughout, and the lock
/// fails closed: every command in it, at every nesting level, must be on the allowlist
/// (`segment_ro`), with no `$` expansion outside single quotes; `bosun` is on it, so
/// `bosun claim 3 # bosun:state/x` passes.
///
/// A plain command (no `$( )`, backticks, `${ }`, `$'..'`; see `plain_segments`) is split with
/// shell quoting, so a quoted `|` or `(` in a grep or jq argument is data, and the body of a
/// heredoc with a quoted delimiter is data. Anything else is split at every `; & |` and
/// newline, quoted or not, any segment with `$` or a backtick is refused, and every `( )` group
/// and backtick span is judged on its own too: stricter, never looser.
///
/// Known misses: a label split by quotes or escapes (`"bosun:"state/x`, `bosun:state\/x`), a
/// label built at run time (`printf`), a label passed through a file, and repo or user config,
/// or files, written by the same or an earlier command (a git pager, diff driver or
/// fsmonitor, a script on `PATH`); writes into `.git/` or a gitconfig are refused, other
/// paths are not.
fn read_only(text: &str, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    if let Some(segments) = plain_segments(text) {
        return segments.iter().all(|s| {
            !s.grouped
                && !s.dollar
                && segment_ro(&s.words, s.glob)
                && s.subs.iter().all(|t| read_only(t, depth + 1))
        });
    }
    // Backticks nest only when escaped; unescaped, every real span is a gap between two
    // consecutive backticks. An odd count is an unterminated span.
    let ticks: Vec<&str> = text.split('`').collect();
    if text.contains("\\`") || ticks.len().is_multiple_of(2) {
        return false;
    }
    let gaps = ticks.get(1..ticks.len() - 1).unwrap_or_default();
    gaps.iter().all(|g| read_only(g, depth + 1))
        && nested(text).iter().all(|n| read_only(n, depth + 1))
        && rough_segments(text).iter().all(|seg| {
            let words: Vec<String> = shell_words(seg).into_iter().filter(|w| w != SEP).collect();
            !seg.contains(['$', '`'])
                && !seg.contains("()")
                && segment_ro(&words, seg.contains(['*', '?', '[', '{']))
        })
}

/// Split at every `;`, `|`, `&` and newline, quoted or not; `>&`, `<&`, `&>` and `>|` are
/// redirections, not separators.
fn rough_segments(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let (mut out, mut start) = (Vec::new(), 0);
    for i in 0..b.len() {
        let redirect = match b[i] {
            b'&' => (i > 0 && matches!(b[i - 1], b'<' | b'>')) || b.get(i + 1) == Some(&b'>'),
            b'|' => i > 0 && b[i - 1] == b'>',
            b';' | b'\n' => false,
            _ => continue,
        };
        if !redirect {
            out.push(&text[start..i]);
            start = i + 1;
        }
    }
    out.push(&text[start..]);
    out
}

/// One simple command of a plain command line: its words with quotes removed; whether an
/// unquoted `(` or `)` appeared (a subshell, a function, `case`: never read-only); whether a
/// `$` appeared outside single quotes (a variable, `$[..]`: never read-only); and whether an
/// argument has an unquoted glob or brace character (it may expand to an option).
#[derive(Default)]
struct PlainSegment {
    words: Vec<String>,
    grouped: bool,
    dollar: bool,
    glob: bool,
    /// Command substitutions in the unquoted heredoc bodies of this command: they run.
    subs: Vec<String>,
}

/// The simple commands of a plain command line, split the way bash splits it: `'..'`, `".."`
/// and `\` quoting, `#` comments, `\`-newline joins, redirections (`>&`, `&>`, `>|`) are not
/// separators, and heredoc bodies are skipped. `None` for anything this does not model
/// exactly (command or parameter substitution, `$'..'`, an unterminated quote or heredoc, an
/// unquoted-delimiter heredoc body with a `$name`, `${` or an unterminated substitution): the caller then
/// splits more roughly. The substitutions of an unquoted heredoc body run, so they are kept in
/// `PlainSegment::subs` to be judged; the rest of the body is data.
fn plain_segments(text: &str) -> Option<Vec<PlainSegment>> {
    let c: Vec<char> = text.chars().collect();
    let (mut out, mut seg) = (Vec::new(), PlainSegment::default());
    let (mut word, mut started) = (String::new(), false);
    // Pending heredocs on this line: delimiter, `<<-` (strip tabs), quoted delimiter.
    let mut heredocs: Vec<(String, bool, bool)> = Vec::new();
    let flush = |word: &mut String, started: &mut bool, seg: &mut PlainSegment| {
        if *started {
            seg.words.push(std::mem::take(word));
            *started = false;
        }
    };
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            '\'' => {
                let end = c[i + 1..].iter().position(|&x| x == '\'')? + i + 1;
                word.extend(&c[i + 1..end]);
                started = true;
                i = end + 1;
                continue;
            }
            '"' => {
                let mut j = i + 1;
                loop {
                    match *c.get(j)? {
                        '"' => break,
                        '`' => return None,
                        '$' if matches!(c.get(j + 1), Some('(' | '{')) => return None,
                        '$' => {
                            seg.dollar = true;
                            word.push('$');
                        }
                        '\\' => {
                            match *c.get(j + 1)? {
                                '\n' => {}
                                n @ ('$' | '`' | '"' | '\\') => word.push(n),
                                n => word.extend(['\\', n]),
                            }
                            j += 2;
                            continue;
                        }
                        x => word.push(x),
                    }
                    j += 1;
                }
                started = true;
                i = j + 1;
                continue;
            }
            '\\' => {
                let n = *c.get(i + 1)?;
                if n != '\n' {
                    word.push(n);
                    started = true;
                }
                i += 2;
                continue;
            }
            '`' => return None,
            '$' if matches!(c.get(i + 1), Some('(' | '{' | '\'' | '"')) => return None,
            '$' => {
                seg.dollar = true;
                word.push('$');
                started = true;
            }
            '#' if !started => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '\n' => {
                flush(&mut word, &mut started, &mut seg);
                out.push(std::mem::take(&mut seg));
                i += 1;
                for (delim, tabs, quoted) in std::mem::take(&mut heredocs) {
                    let mut body = String::new();
                    loop {
                        if i >= c.len() {
                            return None;
                        }
                        let end = c[i..]
                            .iter()
                            .position(|&x| x == '\n')
                            .map_or(c.len(), |p| p + i);
                        let line: String = c[i..end].iter().collect();
                        i = (end + 1).min(c.len());
                        let bare = if tabs {
                            line.trim_start_matches('\t')
                        } else {
                            &line
                        };
                        // Bash joins a backslash-newline before it matches the delimiter.
                        if !quoted && (line.len() - line.trim_end_matches('\\').len()) % 2 == 1 {
                            return None;
                        }
                        if bare == delim {
                            // The body is data, but its substitutions run: judge them.
                            let subs = heredoc_subs(&body)?;
                            out.last_mut()?.subs.extend(subs);
                            break;
                        }
                        if !quoted {
                            body.push_str(&line);
                            body.push('\n');
                        }
                    }
                }
                continue;
            }
            ';' | '|' => {
                flush(&mut word, &mut started, &mut seg);
                out.push(std::mem::take(&mut seg));
            }
            '&' if c.get(i + 1) == Some(&'>') => flush(&mut word, &mut started, &mut seg),
            '&' => {
                flush(&mut word, &mut started, &mut seg);
                out.push(std::mem::take(&mut seg));
            }
            '(' | ')' => {
                flush(&mut word, &mut started, &mut seg);
                seg.grouped = true;
            }
            '<' | '>' => {
                flush(&mut word, &mut started, &mut seg);
                let rest: String = c[i..c.len().min(i + 3)].iter().collect();
                if rest == "<<<" {
                    seg.words.push(rest);
                    i += 3;
                    continue;
                }
                if rest.starts_with("<<") {
                    i += 2;
                    let tabs = c.get(i) == Some(&'-');
                    i += usize::from(tabs);
                    while matches!(c.get(i), Some(' ' | '\t')) {
                        i += 1;
                    }
                    let (mut delim, mut quoted) = (String::new(), false);
                    while let Some(&x) = c.get(i) {
                        match x {
                            '\'' | '"' => {
                                let end = c[i + 1..].iter().position(|&y| y == x)? + i + 1;
                                delim.extend(&c[i + 1..end]);
                                quoted = true;
                                i = end + 1;
                                continue;
                            }
                            '\\' => {
                                delim.push(*c.get(i + 1)?);
                                quoted = true;
                                i += 2;
                                continue;
                            }
                            x if x.is_whitespace() || ";&|<>()`$".contains(x) => break,
                            x => delim.push(x),
                        }
                        i += 1;
                    }
                    if delim.is_empty() {
                        return None;
                    }
                    seg.words.extend(["<<".to_string(), delim.clone()]);
                    heredocs.push((delim, tabs, quoted));
                    continue;
                }
                // `>`, `>>`, `>&`, `>|`, `<`, `<&`, `<>`: one operator word.
                let mut op = ch.to_string();
                if let Some(&n) = c.get(i + 1)
                    && (n == ch || matches!(n, '&' | '|') || (ch == '<' && n == '>'))
                {
                    op.push(n);
                }
                i += op.len();
                seg.words.push(op);
                continue;
            }
            x if x.is_whitespace() => flush(&mut word, &mut started, &mut seg),
            x => {
                seg.glob |= !seg.words.is_empty() && matches!(x, '*' | '?' | '[' | '{');
                word.push(x);
                started = true;
            }
        }
        i += 1;
    }
    if !heredocs.is_empty() {
        return None;
    }
    flush(&mut word, &mut started, &mut seg);
    out.push(seg);
    Some(out)
}

/// The command substitutions (`$( )` and backtick spans) in an unquoted heredoc body; the rest
/// of the body is text. A backslash escapes `$` and a backtick. `None` for anything not modelled
/// exactly (`$name`, `${ }`, an unterminated span, a span with a quote, `#`, `<<`, newline,
/// backslash or `case`): the caller then judges the whole command roughly.
fn heredoc_subs(body: &str) -> Option<Vec<String>> {
    let c: Vec<char> = body.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < c.len() {
        match c[i] {
            '\\' => i += 1,
            '`' => {
                let end = c[i + 1..].iter().position(|&x| x == '`')? + i + 1;
                out.push(c[i + 1..end].iter().collect::<String>());
                i = end;
            }
            // A variable may hold text that a later expansion runs (`${x@P}`, the F13 probes):
            // only a bare `$` (before a space or the end) is plain text.
            '$' if c
                .get(i + 1)
                .is_some_and(|n| *n != '(' && !n.is_whitespace()) =>
            {
                return None;
            }
            '$' if c.get(i + 1) == Some(&'(') => {
                let (mut level, mut j) = (0, i + 1);
                loop {
                    match *c.get(j)? {
                        '(' => level += 1,
                        ')' => level -= 1,
                        _ => {}
                    }
                    j += 1;
                    if level == 0 {
                        break;
                    }
                }
                out.push(c[i + 2..j - 1].iter().collect::<String>());
                i = j - 1;
            }
            _ => {}
        }
        i += 1;
    }
    // Fail closed: the span end above counts raw parens, so a span with anything that can hide
    // or fake a `)` (a quote, a comment, a nested heredoc, a newline, an escape, a `case`
    // pattern) is not modelled.
    let odd = |t: &String| {
        t.contains(['\'', '"', '#', '\\', '\n']) || t.contains("<<") || t.contains("case")
    };
    (!out.iter().any(odd)).then_some(out)
}

/// Variables an assignment-only segment may not set: they change which program runs, or make
/// an allowlisted command run one.
fn risky_var(name: &str) -> bool {
    matches!(
        name,
        "PATH"
            | "IFS"
            | "ENV"
            | "CDPATH"
            | "PAGER"
            | "MANPAGER"
            | "PROMPT_COMMAND"
            | "PS4"
            | "HOME"
    ) || ["LD_", "GIT_", "GH_", "LESS", "BASH", "XDG_"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// The variable a `NAME=value` (or `NAME+=value`) word assigns.
fn assigned(word: &str) -> Option<&str> {
    let (name, _) = word.split_once('=')?;
    let name = name.strip_suffix('+').unwrap_or(name);
    let mut cs = name.chars();
    let first = cs.next()?;
    ((first.is_ascii_alphabetic() || first == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then_some(name)
}

/// The `git show|log|diff|blame|status` options the label-lock vouches for: none runs a
/// program or a pager. Matched exactly (git takes abbreviations, which are refused), the name
/// before any `=`.
const GIT_OPTIONS: &[&str] = &[
    "--",
    "--abbrev-commit",
    "--after",
    "--all",
    "--author",
    "--before",
    "--branch",
    "--cached",
    "--check",
    "--date",
    "--decorate",
    "--exit-code",
    "--first-parent",
    "--follow",
    "--format",
    "--graph",
    "--grep",
    "--max-count",
    "--merges",
    "--name-only",
    "--name-status",
    "--no-color",
    "--no-ext-diff",
    "--no-merges",
    "--no-patch",
    "--no-textconv",
    "--numstat",
    "--oneline",
    "--patch",
    "--porcelain",
    "--pretty",
    "--quiet",
    "--reverse",
    "--short",
    "--shortstat",
    "--since",
    "--staged",
    "--stat",
    "--until",
    "-b",
    "-p",
    "-q",
    "-s",
    "-w",
];

/// A vouched git option: one of `GIT_OPTIONS`, `-<n>` or `-n<n>`.
fn git_option_ok(arg: &str) -> bool {
    let name = arg.split_once('=').map_or(arg, |(k, _)| k);
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    GIT_OPTIONS.contains(&name)
        || arg.strip_prefix('-').is_some_and(digits)
        || arg.strip_prefix("-n").is_some_and(digits)
}

/// The label-lock allowlist. A segment is read-only when its command (quotes removed, no
/// path) is a text tool (grep, cat, jq, ...: no option of theirs runs a program), or, with no
/// glob or brace in its arguments (`glob`, which could expand to an option), `bosun`,
/// `git show|log|diff|blame|status` (only `-C dir` and `--no-pager` before it, only
/// `GIT_OPTIONS` after it), or read-only `gh` (`issue|pr list|view|status`, `pr diff|checks`,
/// `search`, `label list`). Assignments before a command are allowed only for `bosun`; a
/// segment of assignments alone is read-only unless it sets a `risky_var`. Keywords, groups,
/// functions, `xargs`, `less` (its `!`), `sort` and `rg` (options that run programs),
/// `git grep` (`-O`) and every other command are not.
fn segment_ro(words: &[String], glob: bool) -> bool {
    if words
        .iter()
        .any(|w| w.contains("/dev/tcp/") || w.contains("/dev/udp/"))
    {
        return false;
    }
    // Output redirected into git config (`.git/`, `gitconfig`, `git/config`) can make a later
    // allowlisted git command run a program.
    let into_git_config = words.iter().enumerate().any(|(i, w)| {
        let target = w.trim_start_matches(['>', '|', '&']);
        let target = match (w.starts_with('>'), target.is_empty()) {
            (false, _) => return false,
            (true, true) => words.get(i + 1).map_or("", String::as_str),
            (true, false) => target,
        };
        target.contains(".git/") || target.ends_with("gitconfig") || target.ends_with("git/config")
    });
    if into_git_config {
        return false;
    }
    let n = words.iter().take_while(|w| assigned(w).is_some()).count();
    let Some(cmd) = words.get(n) else {
        return !words.iter().filter_map(|w| assigned(w)).any(risky_var);
    };
    if n > 0 && cmd != "bosun" {
        return false;
    }
    let args: Vec<&str> = words[n + 1..].iter().map(String::as_str).collect();
    match cmd.as_str() {
        // `printf -v NAME` assigns a variable, as `NAME=` would, unchecked.
        "printf" => !args.iter().any(|a| a.starts_with("-v")),
        "grep" | "egrep" | "fgrep" | "cat" | "head" | "tail" | "wc" | "uniq" | "cut" | "tr"
        | "jq" | "diff" | "ls" | "test" | "[" | "true" | "false" | "echo" => true,
        _ if glob => false,
        "bosun" => true,
        "git" => {
            let mut i = 0;
            loop {
                match args.get(i) {
                    Some(&"-C") => i += 2,
                    Some(&"--no-pager") => i += 1,
                    _ => break,
                }
            }
            let rest = args.get(i + 1..).unwrap_or_default();
            let mut options = rest.iter().take_while(|a| **a != "--");
            matches!(
                args.get(i),
                Some(&("show" | "log" | "diff" | "blame" | "status"))
            ) && options.all(|a| !a.starts_with('-') || git_option_ok(a))
        }
        "gh" => matches!(
            args.as_slice(),
            ["issue", "list" | "view" | "status", ..]
                | ["pr", "list" | "view" | "status" | "diff" | "checks", ..]
                | ["search", ..]
                | ["label", "list", ..]
        ),
        _ => false,
    }
}

/// The commands inside `$(...)`, backticks, `( ... )` groups, `sh|bash|zsh -c STR` and
/// `eval STR`. Quotes are not honoured for groups, which only makes the check stricter.
fn nested(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = command.chars().collect();
    let (mut i, mut tick) = (0, None);
    while i < chars.len() {
        match chars[i] {
            '(' => {
                let (mut level, mut j) = (1, i + 1);
                while j < chars.len() && level > 0 {
                    match chars[j] {
                        '(' => level += 1,
                        ')' => level -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let end = if level == 0 { j - 1 } else { j };
                out.push(chars[i + 1..end].iter().collect());
                i = j;
                continue;
            }
            '`' => match tick.take() {
                Some(start) => out.push(chars[start + 1..i].iter().collect()),
                None => tick = Some(i),
            },
            _ => {}
        }
        i += 1;
    }
    if let Some(start) = tick {
        out.push(chars[start + 1..].iter().collect());
    }
    out.extend(word_scripts(command));
    out
}

/// The scripts of `env -S`, `eval` and `sh|bash|zsh|dash|ksh -c` in a command.
fn word_scripts(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let words = shell_words(command);
    for (i, w) in words.iter().enumerate() {
        let name = w.rsplit('/').next().unwrap_or(w);
        if name == "env"
            && let Some(inner) = env_split_string(&words[i + 1..])
        {
            out.push(inner);
        } else if name == "eval" {
            let rest: Vec<&str> = words[i + 1..]
                .iter()
                .map(String::as_str)
                .take_while(|w| *w != SEP)
                .collect();
            out.push(rest.join(" "));
        } else if matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh") {
            let flag = words[i + 1..]
                .iter()
                .take_while(|w| **w != SEP)
                .position(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('c'));
            if let Some(f) = flag
                && let Some(script) = words.get(i + 2 + f)
            {
                out.push(script.clone());
            }
        }
    }
    out
}

/// The command `env -S STR [ARGS]` runs (`-S` splits STR into words): STR and the words after
/// it, read through env's options (`-u NAME`, `-C DIR`, `-i`, assignments).
fn env_split_string(after: &[String]) -> Option<String> {
    let after: Vec<&str> = after
        .iter()
        .map(String::as_str)
        .take_while(|w| *w != SEP)
        .collect();
    let mut k = 0;
    while let Some(&w) = after.get(k) {
        let short = w.starts_with('-') && !w.starts_with("--");
        let value = if w == "--split-string" {
            Some("")
        } else if let Some(v) = w.strip_prefix("--split-string=") {
            Some(v)
        } else if short && !w.starts_with("-u") && !w.starts_with("-C") && w.contains('S') {
            w.split_once('S').map(|(_, v)| v)
        } else {
            None
        };
        if let Some(v) = value {
            let rest = after[k + 1..].join(" ");
            return Some(if v.is_empty() {
                rest
            } else {
                format!("{v} {rest}")
            });
        }
        k += match w {
            "-u" | "--unset" | "-C" | "--chdir" => 2,
            _ if w.starts_with('-') || w.contains('=') => 1,
            _ => return None,
        };
    }
    None
}

const SEP: &str = "\u{0}";

/// Split into words with shell quoting (`'..'`, `".."`, backslash); `;`, `&`, `|` and
/// newlines become `SEP` words.
fn shell_words(s: &str) -> Vec<String> {
    let (mut words, mut cur, mut quote, mut started) = (Vec::new(), String::new(), None, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    started = true;
                }
            }
            (None, c) if c.is_whitespace() || matches!(c, ';' | '&' | '|') => {
                if started {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
                if !c.is_whitespace() || c == '\n' {
                    words.push(SEP.to_string());
                }
            }
            (None, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(cur);
    }
    words
}

/// Words that run the next word as a command (`sudo gh ...`) or open a shell group.
const WRAPPERS: &[&str] = &[
    "env", "command", "sudo", "nohup", "time", "exec", "nice", "{", "!", "then", "do", "else",
    "if", "while", "until", "builtin",
];

/// Commands that run the command after their options: name, options with a separate value,
/// positionals before the command (`timeout 5`), options that change directory.
#[rustfmt::skip]
const RUNNERS: &[(&str, &[&str], usize, &[&str])] = &[
    ("env", &["-u", "--unset", "-C", "--chdir", "-S", "--split-string"], 0, &["-C", "--chdir"]),
    ("sudo", &["-u", "--user", "-g", "--group", "-C", "--close-from", "-D", "--chdir", "-p", "--prompt", "-r", "--role", "-t", "--type", "-T", "--command-timeout", "-U", "--other-user", "-R", "--chroot", "-h", "--host", "-a", "--auth-type", "-c", "--login-class"], 0, &["-D", "--chdir"]),
    ("nice", &["-n", "--adjustment"], 0, &[]), ("command", &[], 0, &[]), ("exec", &["-a"], 0, &[]),
    ("nohup", &[], 0, &[]), ("setsid", &[], 0, &[]), ("time", &["-f", "--format", "-o", "--output"], 0, &[]),
    ("timeout", &["-k", "--kill-after", "-s", "--signal"], 1, &[]),
    ("stdbuf", &["-i", "--input", "-o", "--output", "-e", "--error"], 0, &[]),
    ("ionice", &["-c", "--class", "-n", "--classdata", "-p", "--pid", "-P", "--pgid", "-u", "--uid"], 0, &[]),
    ("xargs", &["-I", "-n", "-L", "-P", "-s", "-d", "-E", "-a", "--max-args", "--max-lines", "--max-procs", "--max-chars", "--delimiter", "--arg-file", "--eof", "--process-slot-var"], 0, &[]),
    ("doas", &["-u", "-C", "-a"], 0, &[]), ("unbuffer", &[], 0, &[]), ("busybox", &[], 0, &[]),
    ("chrt", &["-T", "--sched-runtime", "-P", "--sched-period", "-D", "--sched-deadline"], 1, &[]),
    // `-c` takes no value: the mask positional is the cpu list then.
    ("taskset", &[], 1, &[]),
    ("watch", &["-n", "--interval", "-q", "--equexit"], 0, &[]),
    // `FILE -c CMD`: `-c` is a flag here, so its value is the command.
    ("flock", &["-w", "--timeout", "--wait", "-E", "--conflict-exit-code"], 1, &[]),
    ("strace", &["-a", "-b", "-e", "-E", "-I", "-o", "-O", "-p", "-P", "-s", "-S", "-u", "-U", "-X", "--output", "--attach", "--string-limit", "--user", "--env", "--detach-on", "--columns", "--overhead", "--summary-sort-by", "--trace-path", "--summary-columns", "--const-print-style", "--trace", "--trace-fds", "--signal", "--signals", "--status", "--abbrev", "--verbose", "--raw", "--read", "--write", "--fault", "--inject", "--kvm", "--decode-pids"], 0, &[]),
    ("ltrace", &["-a", "-A", "-D", "-e", "-F", "-l", "-n", "-o", "-p", "-s", "-u", "-w", "-x", "--align", "--indent", "--library", "--output", "--config", "--where"], 0, &[]),
    ("unshare", &["-S", "--setuid", "-G", "--setgid", "-R", "--root", "-w", "--wd", "--propagation", "--setgroups", "--map-user", "--map-group", "--map-users", "--map-groups", "--monotonic", "--boottime"], 0, &["-w", "--wd"]),
    ("chroot", &["--userspec", "--groups"], 1, &[]),
    ("caffeinate", &["-t", "-w"], 0, &[]),
];

/// Runner options that change the root (`unshare -R dir`, `sudo --chroot dir`): the command
/// starts in the new `/`, read as `/` of this filesystem (`chroot` does the same).
const NEW_ROOT: &[&str] = &["-R", "--root", "--chroot"];

/// strace options with an optional value, which getopt takes only after `=`: a flag in
/// `RUNNERS`, and judged again as taking the next word (`--decode-fds all cmd`).
#[rustfmt::skip]
const STRACE_OPTIONAL: &[&str] = &["--quiet", "--silent", "--silence", "--decode-fds", "--timestamps", "--absolute-timestamps", "--syscall-times"];

/// The command after leading runners (`nice -n 5 sudo -u x grep ..`), the directories
/// their options move to (`env -C dir`, `--chdir=dir`, `sudo -D dir`, `unshare -w dir`, `/`
/// after a new root), and whether a runner joins the command into a shell string (`watch`).
fn unwrap_runners<'a>(mut cmd: &'a [&'a str]) -> (&'a [&'a str], Vec<&'a str>, bool) {
    let (mut dirs, mut shell) = (Vec::new(), false);
    while let Some(&(runner, vals, mut pos, cd)) = cmd.first().and_then(|w| {
        let w = w.rsplit('/').next().unwrap_or(w);
        RUNNERS.iter().find(|r| r.0 == w)
    }) {
        let mut i = 1;
        while let Some(&a) = cmd.get(i) {
            i += 1;
            if !a.starts_with('-') && assigned(a).is_none() {
                match pos {
                    0 => {
                        i -= 1;
                        break;
                    }
                    _ => pos -= 1,
                }
                continue;
            }
            let (opt, val) = match a.split_once('=') {
                Some((o, v)) if a.starts_with("--") => (o, Some(v)),
                _ if a.len() > 2 && a.get(..2).is_some_and(|o| vals.contains(&o)) => {
                    (&a[..2], Some(&a[2..]))
                }
                _ => (a, None),
            };
            let val = match val {
                None if vals.contains(&opt) => cmd.get(i).inspect(|_| i += 1).copied(),
                v => v,
            };
            if let Some(v) = val.filter(|_| cd.contains(&opt)) {
                dirs.push(v);
            }
            if val.is_some() && NEW_ROOT.contains(&opt) {
                dirs.push("/");
            }
        }
        // A runner with no command after it is the command (`command env`, `timeout 5 env`).
        if cmd.get(i).is_none_or(|w| redirect(w)) {
            break;
        }
        if runner == "chroot" {
            dirs.push("/");
        }
        // `watch` runs its words through `sh -c` (also judged with `-x`: fail closed).
        shell |= runner == "watch";
        cmd = &cmd[i..];
    }
    (cmd, dirs, shell)
}

/// Mutations and endpoints that merge, tag or move refs through the GitHub API.
const GRAPHQL_WRITES: &[&str] = &[
    "mergePullRequest",
    "enablePullRequestAutoMerge",
    "disablePullRequestAutoMerge",
    "enqueuePullRequest",
    "mergeBranch",
    "createRef",
    "updateRef",
    "deleteRef",
];
const HTTP_WRITES: &[&str] = &[
    "/merge",
    "/git/refs",
    "/git/tags",
    "/releases",
    "auto_merge",
    "automerge",
];

/// Where a git command acts: the `-C` options of earlier `cd`s (`None`: unknown), and how to
/// read the branch there.
#[derive(Clone, Copy)]
struct Target<'a> {
    dir: Option<&'a [String]>,
    /// The git command is the whole input (no other command, nesting, expansion or
    /// assignment), so the config the hook reads is the config git runs with.
    alone: bool,
    branch_at: BranchAt<'a>,
    default_branch: &'a str,
    sink: Option<PushSink<'a>>,
}

/// The segments of a command with the separator that ends each one (`&&`, `||`, `;`, `|`,
/// `&`, a newline, or `""` at the end). Quotes are not parsed; a quoted separator only makes
/// the check stricter.
fn segments_with_sep(command: &str) -> Vec<(&str, &'static str)> {
    let (b, mut out, mut start, mut i) = (command.as_bytes(), Vec::new(), 0, 0);
    while i < b.len() {
        let (sep, len): (&'static str, usize) = match (b[i], b.get(i + 1)) {
            (b'&', Some(b'&')) => ("&&", 2),
            (b'|', Some(b'|')) => ("||", 2),
            (b'&', _) => ("&", 1),
            (b'|', _) => ("|", 1),
            (b';', _) => (";", 1),
            (b'\n', _) => ("\n", 1),
            _ => {
                i += 1;
                continue;
            }
        };
        out.push((command[start..i].trim(), sep));
        i += len;
        start = i;
    }
    out.push((command[start..].trim(), ""));
    out.into_iter().filter(|(s, _)| !s.is_empty()).collect()
}

/// A literal path without its quotes; `None` for anything the shell would expand.
fn literal_path(word: &str) -> Option<String> {
    let w = word.trim_matches(|c| c == '"' || c == '\'');
    let expands = w.is_empty()
        || w.starts_with('~')
        || w == "-"
        || w.contains(['$', '`', '*', '?', '[', '{']);
    (!expands).then(|| w.to_string())
}

/// The directory of `cd ARGS`; `None` for `cd` alone, `cd -`, options or expansions.
fn cd_target(args: &[&str]) -> Option<String> {
    match args {
        ["--", one] => literal_path(one),
        [one] if !one.starts_with('-') => literal_path(one),
        _ => None,
    }
}

/// git subcommands that publish, tag or change a branch: with `xargs` their arguments come
/// from stdin, so the guard cannot judge them.
const XARGS_GIT: &[&str] = &[
    "push",
    "tag",
    "merge",
    "rebase",
    "reset",
    "cherry-pick",
    "commit",
];

fn merge_rule(words: &[&str], segment: &str, target: &Target) -> Option<&'static str> {
    // Leading assignments and wrappers, then runners (`timeout 5`, `xargs -n1`, `env -C dir`),
    // until the command itself.
    let runner = |w: &str| {
        RUNNERS
            .iter()
            .any(|r| r.0 == w.rsplit('/').next().unwrap_or(w))
    };
    // The command name a word runs: `\git`, `"git"`, `g''it` and `/usr/bin/git` run git.
    let name = |w: &str| {
        let even = |q: char| w.matches(q).count().is_multiple_of(2);
        let w = if even('\'') && even('"') {
            w.replace(['\'', '"', '\\'], "")
        } else {
            w.to_string()
        };
        w.rsplit('/').next().unwrap_or(&w).to_string()
    };
    // Bash expands `{a,b}` before the command runs: judge the words it makes (an unquoted empty
    // one is dropped). A word the guard cannot expand is refused where it can steer the command:
    // after a wrapper or runner (whose option values the guard may misread), as the command word
    // when its first word is git, gh, a shell, a wrapper or a runner, or in a git or gh command.
    let (mut grown, mut ex, mut unread) = (false, Vec::new(), Vec::new());
    for w in words {
        match braces(w, 0, 0) {
            Some(Ok(more)) => {
                ex.extend(more.into_iter().filter(|m| !m.is_empty()));
                grown = true;
            }
            Some(Err(())) => {
                unread.push(ex.len());
                ex.push(w.to_string());
            }
            None => ex.push(w.to_string()),
        }
    }
    let lead = |n: &str| WRAPPERS.contains(&n) || runner(n);
    const STEERS: [&str; 8] = ["git", "gh", "sh", "bash", "zsh", "eval", "source", "."];
    // A read-only git subcommand right after `git`: an unread word there writes no ref.
    const RO_GIT: [&str; 8] = [
        "log",
        "show",
        "diff",
        "status",
        "grep",
        "blame",
        "rev-parse",
        "ls-files",
    ];
    for &at in &unread {
        let assign = |w: &String| w.contains('=') && !w.starts_with('-');
        let ro_git = !ex.iter().any(|w| name(w) == "gh")
            && ex.iter().position(|w| name(w) == "git").is_some_and(|p| {
                ex[..p].iter().all(assign)
                    && at > p + 1
                    && ex.get(p + 1).is_some_and(|s| RO_GIT.contains(&s.as_str()))
                    // No hidden option (`--output=`, `grep -O`) can sit in the unread word.
                    && plain_braces(&ex[at])
            });
        let refuse = ex[..at].iter().any(|w| lead(&name(w)))
            || if ex[..at].iter().all(assign) {
                first_word(&ex[at]).is_none_or(|f| lead(&name(&f)) || STEERS.contains(&&*name(&f)))
            } else {
                !ro_git && ex.iter().any(|w| matches!(name(w).as_str(), "git" | "gh"))
            };
        if refuse {
            return Some(UNREAD_BRACE);
        }
    }
    if grown {
        let words: Vec<&str> = ex.iter().map(String::as_str).collect();
        return merge_rule(&words, &ex.join(" "), target);
    }
    let (mut words, mut xargs, mut moved) = (words, false, false);
    loop {
        let skip = words
            .iter()
            .take_while(|w| {
                (w.contains('=') && !w.starts_with('-')) || (WRAPPERS.contains(w) && !runner(w))
            })
            .count();
        let (cmd, dirs, _) = unwrap_runners(&words[skip..]);
        let ran = &words[skip..words.len() - cmd.len()];
        xargs |= ran.iter().any(|w| w.rsplit('/').next() == Some("xargs"));
        moved |= !dirs.is_empty();
        if skip == 0 && ran.is_empty() {
            break;
        }
        words = cmd;
    }
    // A runner that changes directory moves git to a place the guard cannot read: fail closed.
    let target = Target {
        dir: target.dir.filter(|_| !moved),
        ..*target
    };
    if xargs {
        let blind = match words.first().copied() {
            Some("gh") => {
                words.len() < 3 || matches!(words[1], "api" | "release" | "alias" | "repo")
            }
            Some("git") => words.len() < 2 || words[1..].iter().any(|w| XARGS_GIT.contains(w)),
            Some("curl" | "wget" | "http" | "https" | "xh") => true,
            _ => false,
        };
        if blind {
            return Some("`xargs` adds arguments the guard cannot see");
        }
    }
    // `\git`, `"git"`, `g''it` and `/usr/bin/git` all run git.
    // A word with an open quote is part of a longer string (`python -c 'gh ..'`): left as is.
    let first = words.first().map(|w| {
        let even = |q: char| w.matches(q).count() % 2 == 0;
        if even('\'') && even('"') {
            w.replace(['\'', '"', '\\'], "")
        } else {
            w.to_string()
        }
    });
    let first = first.as_deref().map(|w| w.rsplit('/').next().unwrap_or(w));
    if let Some(why) = exec_rule(first, words, segment, &target) {
        return Some(why);
    }
    // `git-fetch`, `/usr/lib/git-core/git-push`: a git-core program runs that subcommand.
    if let Some(sub) = first.and_then(|f| f.strip_prefix("git-")) {
        let args: Vec<&str> = std::iter::once(sub)
            .chain(words[1..].iter().copied())
            .collect();
        return git_rule(&args, &target);
    }
    match first {
        Some("gh") => gh_rule(&words[1..], segment),
        // `GIT_CONFIG_KEY_n`, `GIT_CONFIG_PARAMETERS`: config from the environment.
        Some("git")
            if segment.contains("GIT_CONFIG_")
                && segment.to_ascii_lowercase().contains("alias.") =>
        {
            Some(GIT_ALIAS)
        }
        Some("git")
            if segment.contains("GIT_CONFIG_")
                && segment.to_ascii_lowercase().contains(".fetch") =>
        {
            Some(FETCH_MOVES)
        }
        Some("git") => git_rule(&words[1..], &target),
        Some("curl" | "wget" | "http" | "https" | "xh") => http_rule(segment),
        _ => None,
    }
}

/// A brace word of letters, digits, `{}`, `,` and `.` whose `x..y` ranges join two digits or two
/// letters of one case: it can hold no option (`{!..~}` and `{0..z}` span `-` and `=`).
fn plain_braces(w: &str) -> bool {
    let same = |a: char, b: char| {
        (a.is_ascii_digit() && b.is_ascii_digit())
            || (a.is_ascii_lowercase() && b.is_ascii_lowercase())
            || (a.is_ascii_uppercase() && b.is_ascii_uppercase())
    };
    w.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '{' | '}' | ',' | '.'))
        && w.match_indices("..").all(|(i, _)| {
            match (w[..i].chars().last(), w[i + 2..].chars().next()) {
                (Some(a), Some(b)) => same(a, b),
                _ => false,
            }
        })
}

/// Commands that run another command: `find -exec`, `fd -x`, and `awk system()`.
fn exec_rule(
    first: Option<&str>,
    words: &[&str],
    segment: &str,
    target: &Target,
) -> Option<&'static str> {
    match first {
        Some("find" | "fd" | "fdfind") => {
            let at = words.iter().position(|w| {
                matches!(
                    *w,
                    "-exec"
                        | "-execdir"
                        | "-ok"
                        | "-okdir"
                        | "-x"
                        | "-X"
                        | "--exec"
                        | "--exec-batch"
                )
            })?;
            let end = words[at + 1..]
                .iter()
                .position(|w| matches!(*w, ";" | "+" | "\\;"))
                .map_or(words.len(), |e| at + 1 + e);
            let inner = &words[at + 1..end];
            // `{}` stands for names find adds: judged like the command without them.
            merge_rule(inner, &inner.join(" "), target)
        }
        Some("awk" | "gawk" | "mawk" | "nawk") => {
            calls_system(segment).then_some("`awk` runs commands through system()")
        }
        _ => None,
    }
}

/// Whether the text has `system` followed by optional blanks and `(`: an awk `system()` call.
fn calls_system(s: &str) -> bool {
    s.match_indices("system")
        .any(|(i, m)| s[i + m.len()..].trim_start().starts_with('('))
}

const UNREAD_BRACE: &str = "a `{x..y}` brace sequence the guard cannot see through: \
    write the words out";

/// Bash brace expansion of one word: `None` when it has no unquoted `{a,b}` list or `{x..y}`
/// sequence (`${x}`, `@{u}`, `{}` stay as they are); `Err` for a sequence bash would not expand,
/// more than 64 words or more than 8 lists in one word. The scan starts at `from`, where no
/// quote is open (all before it is already expanded).
fn braces(word: &str, from: usize, depth: usize) -> Option<Result<Vec<String>, ()>> {
    // Each open `{` scans to the end of the word: a long one is not read (fail closed).
    if word.len() > 1024 && word.contains('{') {
        return Some(Err(()));
    }
    let mut at = from;
    while let Some(i) = open_brace(word, at) {
        at = i + 1;
        // An open `{` with a list after it: a quote split it from its `}` (`{a,'b c'}`).
        let Some((end, cuts)) = brace_list(&word[i..]) else {
            if word[i..].contains(',') || word[i..].contains("..") {
                return Some(Err(()));
            }
            continue;
        };
        let alts: Vec<String> = if cuts.is_empty() {
            if !word[i..i + end].contains("..") {
                continue;
            }
            match sequence(&word[i + 1..i + end]) {
                Some(items) => items,
                None => return Some(Err(())),
            }
        } else {
            let starts = [0].into_iter().chain(cuts.iter().map(|c| c + 1));
            let ends = cuts.iter().copied().chain([end]);
            starts
                .zip(ends)
                .map(|(a, b)| word[i + a.max(1)..i + b].to_string())
                .collect()
        };
        if alts.len() > 64 || depth >= 8 {
            return Some(Err(()));
        }
        let (head, tail) = (&word[..i], &word[i + end + 1..]);
        let mut out = Vec::new();
        for alt in alts {
            let one = format!("{head}{alt}{tail}");
            match braces(&one, i, depth + 1) {
                None => out.push(one),
                Some(Ok(more)) => out.extend(more),
                Some(Err(())) => return Some(Err(())),
            }
            if out.len() > 64 {
                return Some(Err(()));
            }
        }
        return Some(Ok(out));
    }
    None
}

/// The offset of the first `{` at or after `from` outside quotes and escapes, not after `$`.
fn open_brace(word: &str, from: usize) -> Option<usize> {
    let (mut quote, mut esc) = (None, false);
    for (i, c) in word[from..].char_indices().map(|(i, c)| (i + from, c)) {
        match (quote, c) {
            _ if std::mem::take(&mut esc) => {}
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => esc = true,
            (Some(_), '"') => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '{') if !word[..i].ends_with('$') => return Some(i),
            _ => {}
        }
    }
    None
}

/// The first word bash makes of `word`, which runs when it is the command word: each list or
/// sequence gives its first item, and a `{` a quote split from its `}` ends at its first `,`.
/// `None` when that item is empty (bash drops an empty word, so the next one runs).
fn first_word(word: &str) -> Option<String> {
    let (mut w, mut at) = (word.to_string(), 0);
    for _ in 0..64 {
        let Some(i) = open_brace(&w, at) else {
            return Some(w);
        };
        at = i + 1;
        let (end, item) = match brace_list(&w[i..]) {
            Some((end, cuts)) if !cuts.is_empty() => (end, w[i + 1..i + cuts[0]].to_string()),
            Some((end, _)) => match sequence(&w[i + 1..i + end]) {
                Some(items) => (end, items[0].clone()),
                None => continue,
            },
            None => {
                let item = w[i + 1..].split([',', ' ']).next().unwrap_or_default();
                return Some(format!("{}{item}", &w[..i])).filter(|f| !f.is_empty());
            }
        };
        (!item.is_empty()).then_some(())?;
        w = format!("{}{item}{}", &w[..i], &w[i + end + 1..]);
        at = i;
    }
    None
}

/// The words of a bash `{x..y[..step]}` sequence (integers or single letters), at most 64.
fn sequence(inner: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = inner.split("..").collect();
    let step = match parts.get(2) {
        Some(s) => s.parse::<i64>().ok()?.unsigned_abs().max(1),
        None => 1,
    };
    let (a, b) = match parts[..] {
        [a, b] | [a, b, _] => (a, b),
        _ => return None,
    };
    let num = |x: &str| x.parse::<i64>().ok();
    let letter = |x: &str| {
        (x.len() == 1 && x.as_bytes()[0].is_ascii_alphabetic()).then(|| i64::from(x.as_bytes()[0]))
    };
    let (lo, hi, text) = match (num(a), num(b), letter(a), letter(b)) {
        (Some(x), Some(y), ..) => (x, y, false),
        (_, _, Some(x), Some(y)) => (x, y, true),
        _ => return None,
    };
    // A counted loop in i128: no bound or step near the i64 limits can overflow or spin.
    let count = lo.abs_diff(hi) / step;
    if count >= 64 {
        return None;
    }
    let pad = [a, b]
        .iter()
        .any(|x| x.trim_start_matches('-').len() > 1 && x.trim_start_matches('-').starts_with('0'));
    let width = if pad { a.len().max(b.len()) } else { 0 };
    let dir: i128 = if lo <= hi { 1 } else { -1 };
    let out = (0..=count)
        .map(|k| {
            let v = i128::from(lo) + dir * i128::from(k) * i128::from(step);
            match u8::try_from(v) {
                Ok(c) if text => char::from(c).to_string(),
                _ => format!("{v:0width$}"),
            }
        })
        .collect();
    Some(out)
}

/// For text starting with `{`: the offset of its matching `}` and of its top-level commas.
fn brace_list(s: &str) -> Option<(usize, Vec<usize>)> {
    let (mut depth, mut quote, mut esc, mut cuts) = (0, None, false, Vec::new());
    for (i, c) in s.char_indices() {
        match (quote, c) {
            _ if std::mem::take(&mut esc) => {}
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => esc = true,
            (Some(_), '"') => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '{') => depth += 1,
            (None, '}') => {
                depth -= 1;
                if depth == 0 {
                    return Some((i, cuts));
                }
            }
            (None, ',') if depth == 1 => cuts.push(i),
            _ => {}
        }
    }
    None
}

/// For a command word the shell builds (`$g`, `"$G"`, `${X:-y}`, `$(..)`, a backtick span): the
/// name (`gh`, `git`) that, put in its place, makes the segment a merge, tag or push. `None` when
/// neither does, as for `$HOME/.cargo/bin/cargo test`.
fn built_name_rule(word: &str, segment: &str, target: &Target) -> Option<&'static str> {
    let start = word.as_ptr() as usize - segment.as_ptr() as usize;
    let rest = &segment[start..];
    // The word ends at the first blank outside quotes, `( )`, `{ }` and backtick spans.
    let (mut depth, mut quote, mut tick) = (0i32, None::<char>, false);
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '`') => tick = !tick,
            (None, '(' | '{') => depth += 1,
            (None, ')' | '}') => depth -= 1,
            (None, c) if c.is_whitespace() && depth <= 0 && !tick => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    ["gh", "git"].into_iter().find(|name| {
        let swapped = format!("{name}{}", &rest[end..]);
        let words: Vec<&str> = swapped.split_whitespace().collect();
        merge_rule(&words, &swapped, target).is_some()
    })
}

/// A shell or interpreter that reads its script from the pipe it is fed: `curl .. | sh`.
fn reads_pipe(words: &[&str]) -> bool {
    let (cmd, _, _) = unwrap_runners(words);
    let name = cmd.first().map_or("", |w| base(w));
    let name = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if ![
        "sh", "bash", "zsh", "dash", "ksh", "fish", "python", "perl", "ruby", "node",
    ]
    .contains(&name)
    {
        return false;
    }
    let args = &cmd[1..];
    args.contains(&"-s")
        || args.iter().all(|a| a.starts_with('-') && *a != "-c")
        || args.contains(&"-")
}

fn destructive(what: &str) -> String {
    format!("destructive-lock: `{what}` can wipe a machine or a database; ask the owner to run it")
}

fn base(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// A client whose arguments, stdin or heredoc are SQL.
fn sql_client(word: &str) -> bool {
    let clients = "psql pgcli mysql mariadb mycli sqlite3 duckdb usql cockroach clickhouse sqlcmd \
        clickhouse-client";
    clients.split_whitespace().any(|c| base(word) == c)
}

/// Destructive-lock (spec §4): the command word of every segment after wrappers and runners,
/// and of every nested command; quoted text, other arguments and heredoc bodies are data.
fn destructive_rule(command: &str, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(too_deep());
    }
    let (code, subs) = split_heredocs(command);
    let inner = subs.into_iter().chain(substitutions(&code, false));
    if let Some(why) = inner
        .chain(word_scripts(&code))
        .find_map(|s| destructive_rule(&s, depth + 1))
    {
        return Some(why);
    }
    let words = shell_words(&code);
    // The fork bomb `:(){ :|:& };:` as words (quoted, it is one word: data).
    if words.concat().contains(":(){:\0:\0}\0:") {
        return Some(destructive("fork bomb"));
    }
    let segments: Vec<&[String]> = words
        .split(|w| w == SEP)
        .filter(|s| !s.is_empty())
        .collect();
    let mut client = None;
    for (i, segment) in segments.iter().enumerate() {
        if disk_redirect(segment) {
            return Some(destructive("redirect into a disk device"));
        }
        let all: Vec<&str> = segment.iter().map(String::as_str).collect();
        let mut cmd = all.as_slice();
        loop {
            let runs = |w: &str| RUNNERS.iter().any(|r| r.0 == base(w));
            let skip = (cmd.iter())
                .take_while(|w| assigned(w).is_some() || WRAPPERS.contains(w) && !runs(w))
                .count();
            let (rest, _, shell) = unwrap_runners(&cmd[skip..]);
            if shell && let Some(why) = destructive_rule(&rest.join(" "), depth + 1) {
                return Some(why);
            }
            if rest.len() == cmd.len() {
                break;
            }
            cmd = rest;
        }
        let (name, args) = cmd.split_first().unwrap_or((&"", &[]));
        if wipes(base(name), args) {
            return Some(destructive(&segment.join(" ")));
        }
        // SQL is what a client reads: its arguments, a pipe into it, its heredoc.
        let remote = ["docker", "podman", "kubectl", "ssh"].contains(&base(name));
        let mut named = cmd.iter().take(if remote { cmd.len() } else { 1 });
        if named.any(|w| sql_client(w)) {
            client.get_or_insert(i);
        }
    }
    let sql = segments[client?.saturating_sub(1)..].concat().join(" ");
    let sql: Vec<&str> = sql.split_whitespace().collect();
    let sql = sql.join(" ").to_ascii_lowercase();
    ["drop database", "drop schema"]
        .into_iter()
        .find(|d| sql.contains(d))
        .map(destructive)
}

/// A redirect (`>`, `>>`, `&>`, `2>|`, glued or not) into a disk device.
fn disk_redirect(segment: &[String]) -> bool {
    segment.iter().enumerate().any(|(i, w)| {
        let w = w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
        let glued = w.trim_start_matches(['>', '|', '&']);
        let next = segment.get(i + 1).map_or("", String::as_str);
        let dev = (if glued.is_empty() { next } else { glued }).strip_prefix("/dev/");
        w.starts_with('>') && dev.is_some_and(|d| DISKS.iter().any(|k| d.starts_with(k)))
    })
}

const DISKS: &[&str] = &["sd", "nvme", "disk", "hd", "vd"];

/// `/`, `~` or `$HOME` (in any spelling) alone or with `/`, `/*` or `/.`.
fn wipe_target(a: &str) -> bool {
    let home = ["~", "$HOME", "${HOME}", "${HOME:?}"]
        .iter()
        .find_map(|h| a.strip_prefix(h));
    ["/", "/*", "/."].contains(&a) || home.is_some_and(|r| ["", "/", "/*", "/."].contains(&r))
}

/// Whether the command `name ARGS` wipes a machine, a disk or an infrastructure.
fn wipes(name: &str, args: &[&str]) -> bool {
    let flag = |long: &str, short: &[char]| {
        let short = |a: &&str| a.starts_with('-') && !a.starts_with("--") && a.contains(short);
        args.iter().any(|a| *a == long || short(a))
    };
    let aimed = args.iter().any(|a| wipe_target(a));
    match name {
        "rm" => {
            args.contains(&"--no-preserve-root")
                || (flag("--recursive", &['r', 'R']) || flag("--force", &['f'])) && aimed
        }
        "chmod" | "chown" | "chgrp" => flag("--recursive", &['R']) && aimed,
        "dd" => (args.iter().filter_map(|a| a.strip_prefix("of=/dev/")))
            .any(|t| !["null", "stdout", "stderr", "tty"].contains(&t) && !t.starts_with("fd/")),
        "shred" | "wipefs" => args.iter().any(|a| a.starts_with("/dev/")),
        "terraform" | "tofu" => match args.iter().find(|a| !a.starts_with('-')) {
            Some(&"destroy") => true,
            Some(&"apply") => args.iter().any(|a| a.trim_start_matches('-') == "destroy"),
            _ => false,
        },
        n => n == "mke2fs" || n.starts_with("mkfs"),
    }
}

/// The command without its heredoc bodies (data, unless a shell or a SQL client on the opening
/// line reads it), and the substitutions in unquoted bodies (they run).
fn split_heredocs(command: &str) -> (String, Vec<String>) {
    let (mut code, mut subs) = (String::new(), Vec::new());
    let mut skip: Option<(String, bool, bool)> = None;
    for line in command.split_inclusive('\n') {
        if let Some((delim, dash, quoted)) = &skip {
            let l = line.trim_end_matches('\n');
            let l = if *dash { l.trim_start_matches('\t') } else { l };
            // Bash ends a heredoc inside `$( )` at `EOF)`: a body line that starts with the
            // delimiter and more is not plain data; fail closed and judge the body as code.
            if l != delim && l.starts_with(delim.as_str()) {
                return (command.to_string(), Vec::new());
            }
            if l == delim {
                skip = None;
                code.push_str(line);
            } else if !quoted {
                subs.extend(substitutions(line, true));
            }
            continue;
        }
        code.push_str(line);
        let Some(at) = heredoc_at(line) else { continue };
        let dash = line[at + 2..].starts_with('-');
        let spec = line[at + 2..].trim_start_matches('-').trim_start();
        let quote = spec.chars().next().filter(|c| matches!(c, '\'' | '"'));
        let delim = match quote {
            Some(q) => spec[1..].find(q).map(|end| spec[1..=end].to_string()),
            None => Some(
                (spec.chars())
                    .take_while(|c| !c.is_whitespace() && !";|&<>()".contains(*c))
                    .filter(|c| *c != '\\')
                    .collect(),
            ),
        };
        let reads = shell_words(line)
            .iter()
            .any(|w| SHELLS.contains(&base(w)) || interp::is_runner(w) || sql_client(w));
        if let Some(delim) = delim.filter(|d| !d.is_empty() && !reads) {
            skip = Some((delim, dash, quote.is_some()));
        }
    }
    (code, subs)
}

/// The `$( )`, backtick and (outside quotes) `( )` spans of a command line, quote-aware; in a
/// heredoc `body` quotes are text and only `$( )` and backticks run.
fn substitutions(text: &str, body: bool) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let (mut out, mut quote, mut i) = (Vec::new(), None, 0);
    while i < c.len() {
        match (quote, c[i]) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => i += 1,
            (None, '\'' | '"') if !body => quote = Some(c[i]),
            (_, '`') => {
                let end = (i + 1..c.len()).find(|&j| c[j] == '`' && c[j - 1] != '\\');
                let end = end.unwrap_or(c.len());
                out.push(c[i + 1..end].iter().collect());
                i = end;
            }
            (q, '(') if (q.is_none() && !body) || (i > 0 && c[i - 1] == '$') => {
                let (mut level, mut j) = (1, i + 1);
                while j < c.len() && level > 0 {
                    level += i32::from(c[j] == '(') - i32::from(c[j] == ')');
                    j += 1;
                }
                let end = if level == 0 { j - 1 } else { j };
                out.push(c[i + 1..end].iter().collect());
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

const DECRYPT_FIX: &str = "secrets stay encrypted; the owner runs `hoist secrets edit`, CI \
    decrypts with `hoist secrets exec` (spec §8)";

fn decrypt_reason(what: &str) -> String {
    format!("decrypting secrets is refused for agents ({what}): {DECRYPT_FIX}")
}

/// An environment dump or a read of the process environment: it can hold tokens and keys, but it
/// is not a decrypt, so it gets its own reason.
fn env_reason(what: &str) -> String {
    format!(
        "dumping {what} is refused for agents (it can hold tokens and keys): read only the \
         variables you need, by name (spec §8)"
    )
}

const HOIST_SECRETS: &[&str] = &["edit", "sync", "grant", "revoke", "rotate", "rotate-key"];
/// Readers: their arguments are data (patterns, text).
#[rustfmt::skip]
const READERS: &[&str] = &["grep", "egrep", "fgrep", "rgrep", "zgrep", "ugrep", "ug", "ugrep+",
    "ug+", "pcregrep", "pcre2grep", "rg", "ag", "ack", "ack-grep", "cat", "head", "tail", "ls", "wc",
    "echo", "printf", "which", "type"];
/// The grep family; rg, ag, ack and rgrep always recurse, rg and ag skip hidden directories.
#[rustfmt::skip]
const SEARCHERS: &[&str] = &["grep", "egrep", "fgrep", "rgrep", "zgrep", "ugrep", "ug", "ugrep+",
    "ug+", "pcregrep", "pcre2grep", "rg", "ag", "ack", "ack-grep"];
/// Searchers whose options and environment run commands: only their pattern is data.
const PAGED: &[&str] = &["ack", "ack-grep", "ugrep", "ug", "ugrep+", "ug+"];
#[rustfmt::skip]
const ENV_READS: &[&str] = &["environ", "process.env", "ENV[", "ENV.", "ENV{", "ENVIRON[", "getenv",
    "Deno.env", "Bun.env", "%ENV", ").env", "$_ENV", "$_SERVER"];
const INTERPS: &[&str] = &[
    "python", "python3", "perl", "ruby", "node", "deno", "bun", "php",
];
const FILE_TOOLS: &[&str] = &[
    "mv", "cp", "install", "chmod", "chown", "ln", "mkdir", "rmdir", "rm", "touch", "tar", "unzip",
    "gzip", "gunzip", "curl", "wget", "stat", "file", "du", "df",
];
/// Package managers, and the subcommands whose arguments are package names (never `run`/`exec`).
const PACKAGERS: &[&str] = &[
    "brew", "apt", "apt-get", "dnf", "yum", "apk", "pacman", "port", "pip", "pip3", "pipx", "npm",
    "pnpm", "yarn", "bun", "cargo",
];
#[rustfmt::skip]
const PKG_SUBS: &[&str] = &["install", "add", "remove", "uninstall", "upgrade", "update", "info",
    "search", "show", "list", "build", "test", "nextest", "check", "clippy", "fmt"];
#[rustfmt::skip]
const GH_DATA: &[&str] = &["t", "title", "b", "body", "notes", "f", "F", "raw-field", "field",
    "search", "q", "jq"];
/// The pagers ack and ugrep may run, and the editors ugrep `--view` may run with no arguments.
const PAGERS_OK: &[&str] = &["less", "more", "most", "cat"];
const VIEWERS_OK: &[&str] = &["vi", "vim", "nvim", "view", "nano", "emacs", "code"];
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "busybox", "python", "python3", "perl", "ruby",
    "node", "deno", "bun", "php", "source", "xargs", "eval",
];

/// The files that `> f`, `>> f` and `tee f` write in these words.
fn write_targets(w: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, x) in w.iter().enumerate() {
        let r = x.trim_start_matches('>');
        #[rustfmt::skip]
        let t = if x.starts_with('>') { if r.is_empty() { w.get(i + 1).map(String::as_str) } else { Some(r) } }
            else if x == "tee" { w[i + 1..].iter().map(String::as_str).find(|a| !a.starts_with('-')) } else { None };
        out.extend(t.map(str::to_string));
    }
    out
}

/// An age secret key line (or its tail) named in a word, in any case.
fn keyline(w: &str) -> bool {
    let w = w.to_lowercase();
    w.contains("age-secret") || w.contains("secret-key-1")
}

/// A word naming `.config/sops` as a whole path segment (`.config/sops-test` is another dir).
fn sops_dir(w: &str) -> bool {
    w.match_indices(".config/sops")
        .any(|(i, m)| w[i + m.len()..].is_empty() || w[i + m.len()..].starts_with('/'))
}

/// A path with `//`, `.` and `dir/..` collapsed, lexically (no fs). `..` stays after `~`, `$X`,
/// a glob or another `..`, and anywhere under `/proc` (its links are symlinks); `/..` is `/`.
fn lexical(path: &str) -> String {
    let (abs, proc) = (path.starts_with('/'), path.starts_with("/proc/"));
    let mut out: Vec<&str> = Vec::new();
    for s in path.split('/') {
        #[rustfmt::skip]
        let plain = |l: &&str| !proc && *l != ".." && !l.starts_with(['~', '$']) && !l.contains(['*', '?', '[']);
        match s {
            "" | "." => {}
            ".." if out.last().is_some_and(plain) => drop(out.pop()),
            ".." if abs && out.is_empty() => {}
            _ => out.push(s),
        }
    }
    let j = out.join("/");
    match (abs, j.is_empty() && !path.is_empty()) {
        (true, _) => format!("/{j}"),
        (false, true) => ".".to_string(),
        _ => j,
    }
}

/// How much of the age key directory a search root contains: 2 for `.config` or an ancestor
/// of `.config/sops` inside it (`~/.config`, `$XDG_CONFIG_HOME`), or a pure `..` chain above
/// an unknown place (`..`, `$PWD/..`) or a glob there (`/*`, `../*`); 1 for home or above it
/// (`~`, `~user`, `/root`, `/home/<user>`, `/home`, `/`, `../bob`, `../home/bob`); 0 for
/// anything else (`~/projects/x`, `../x/src`).
fn key_root(word: &str) -> u8 {
    let n = lexical(word.trim_matches(|c| c == '"' || c == '\''));
    let word = n.as_str();
    let t = word.trim_end_matches('/');
    // A `..` left after `lexical` climbs from an unknown place: what follows the last one is
    // judged as under `/` and as under a home (`..` alone is above it, `../bob` may be a home).
    let segs: Vec<&str> = t.split('/').collect();
    if let Some(i) = segs.iter().rposition(|s| *s == "..") {
        let tail = &segs[i + 1..];
        #[rustfmt::skip]
        let key = tail.iter().any(|s| matches!(*s, ".config" | "sops" | "age") || s.contains(['*', '?', '[']));
        let j = tail.join("/");
        let as_root = key_root(&format!("/{j}")).max(key_root(&format!("~/{j}")));
        return if tail.is_empty() || key {
            2
        } else {
            as_root.max(u8::from(tail.len() == 1))
        };
    }
    let odd = t.contains(['*', '?', '[']);
    // A glob in a segment of home or above it may match the key directory (`/*`, `/r*`).
    if let Some(g) = t.find(['*', '?', '[']) {
        let dir = t[..g]
            .rfind('/')
            .map_or("", |i| if i == 0 { "/" } else { &t[..i] });
        if !dir.is_empty() && key_root(dir) == 1 {
            return 2;
        }
    }
    let under = |r: &str, own: &str| {
        r == own
            || r.strip_prefix(own)
                .is_some_and(|x| x == "/sops" || x.starts_with("/sops/"))
    };
    if t.is_empty() || t == "/home" {
        return u8::from(word.starts_with('/') || word.is_empty());
    }
    if let Some(r) = ["$XDG_CONFIG_HOME", "${XDG_CONFIG_HOME}"]
        .iter()
        .find_map(|h| t.strip_prefix(h))
    {
        return if odd || r.is_empty() || r.starts_with("/sops/") || r == "/sops" {
            2
        } else {
            0
        };
    }
    let rest = ["$HOME", "${HOME}", "/root"]
        .iter()
        .find_map(|h| t.strip_prefix(h))
        .or_else(|| {
            let user = t.strip_prefix('~')?;
            Some(user.find('/').map_or("", |i| &user[i..]))
        })
        .or_else(|| {
            let user = t.strip_prefix("/home/")?;
            Some(user.find('/').map_or("", |i| &user[i..]))
        });
    match rest {
        Some(_) if odd => 2,
        Some("") => 1,
        Some(r) if under(r, "/.config") => 2,
        None if under(t.trim_start_matches("./"), ".config") => 2,
        _ => 0,
    }
}

/// Whether a search of these roots reads the age key: one is under `.config/sops`, or the
/// search recurses into `.config` or an ancestor (home or above only when it enters hidden dirs).
fn key_search(roots: &[&str], recursive: bool, hidden: bool) -> bool {
    let root = roots.iter().map(|a| key_root(a)).max().unwrap_or(0);
    roots.iter().any(|a| sops_dir(a)) || recursive && (root == 2 || root == 1 && hidden)
}

/// A redirect word: `>`, `>>`, `2>`, `&>`, `<`, `2>&1`, `>file`.
fn redirect(a: &str) -> bool {
    a.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&')
        .starts_with(['>', '<'])
}

fn procenv(w: &str) -> bool {
    w.contains("/proc/") && w.contains("env")
}

/// Whether an interpreter or shell command line has code on it (`-c`, `-e`, `--eval`, `eval`, `php -r`).
fn runs_code(n: &str, after: &[&str]) -> bool {
    let dash = |a: &&str, c: char| a.starts_with('-') && !a.starts_with("--") && a.contains(c);
    let interp = INTERPS.contains(&n);
    #[rustfmt::skip]
    let code = |a: &&str| dash(a, 'c') || n == "bun" && (dash(a, 'p') || *a == "--print") || interp && (dash(a, 'e') || *a == "--eval" || *a == "eval" || n == "php" && dash(a, 'r'));
    after.iter().any(code)
}

/// A flag without its dashes and value: `--decrypt=x` and `-decrypt` are `decrypt`.
fn flag(arg: &str) -> Option<&str> {
    let f = arg.strip_prefix('-')?;
    Some(f.trim_start_matches('-').split('=').next().unwrap_or(""))
}

/// A long option given as `full` or a prefix of it at least `min` long (git, getopt and ack
/// take unique prefixes; `min` is the shortest one unique among the tool's options).
fn long_prefix(arg: &str, full: &str, min: usize) -> bool {
    let n = arg.split('=').next().unwrap_or(arg);
    n.len() >= min && full.starts_with(n)
}

/// The values of a long option or its prefixes: `--opt=v`, else the next word.
fn long_values<'a>(args: &[&'a str], full: &str, min: usize) -> Vec<&'a str> {
    #[rustfmt::skip]
    let v = args.iter().enumerate().filter(|(_, a)| long_prefix(a, full, min))
        .filter_map(|(i, a)| a.split_once('=').map(|p| p.1).or_else(|| args.get(i + 1).copied()));
    v.collect()
}

/// Whether the commands ack and ugrep run on their output or files are all known pagers:
/// `--pager` and ugrep's `--view` (also abbreviated), `ACK_PAGER`, `ACK_PAGER_COLOR` and the
/// options in `ACK_OPTIONS` and `PAGER` (set anywhere in the command), and ugrep's view
/// fallbacks `EDITOR` and `VISUAL`; `LESS` may hold only such flags, and `LESSOPEN`,
/// `LESSCLOSE`, `LESSKEY*`, `LESSSECURE`, `BAT_PAGER` and vim's or nvim's init variables may not
/// be set. ugrep's `--pager` and `--view` take a value only with `=`. A pager is a known name, bare or in `/usr/bin` or `/bin`, with plain flag clusters
/// (`-R`, `-`; no `k`, `o` or `O`, which read keys or write files); an editor may only view,
/// with no arguments.
fn pagers_ok(name: &str, words: &[String], args: &[&str]) -> bool {
    let mut opts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut cmds: Vec<(String, bool)> = Vec::new();
    let ugrep = name.starts_with('u');
    // ugrep opens a viewer only from its query TUI: `-Q` (also in a cluster), `--query`, `--view`.
    #[rustfmt::skip]
    let views = ugrep && args.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('Q') || long_prefix(a, "--query", 5) || long_prefix(a, "--view", 4));
    for (k, v) in words
        .iter()
        .filter_map(|w| Some((assigned(w)?, w.split_once('=')?.1)))
    {
        match k {
            "ACK_PAGER" | "ACK_PAGER_COLOR" | "PAGER" => cmds.push((v.to_string(), false)),
            "EDITOR" | "VISUAL" if views => cmds.push((v.to_string(), true)),
            "ACK_OPTIONS" => opts.extend(shell_words(v)),
            "LESS" if shell_words(v).iter().all(|a| pager_flag(a)) => {}
            "LESS" | "LESSOPEN" | "LESSCLOSE" | "LESSKEY" | "LESSKEYIN" | "LESSKEY_SYSTEM"
            | "LESSSECURE" | "BAT_PAGER" => return false,
            // Vim and nvim init commands and files (`init.lua` under `XDG_CONFIG_HOME`).
            "VIMINIT" | "EXINIT" | "MYVIMRC" | "GVIMINIT" | "VIM" | "VIMRUNTIME"
            | "XDG_CONFIG_HOME" | "NVIM_APPNAME"
                if views =>
            {
                return false;
            }
            _ => {}
        }
    }
    let opts: Vec<&str> = opts.iter().map(String::as_str).collect();
    // ugrep's values are optional: a bare `--pager` or `--view` uses the default.
    let eq = |full: &str, min: usize| -> Vec<&str> {
        #[rustfmt::skip]
        let v = opts.iter().filter(|a| long_prefix(a, full, min)).filter_map(|a| Some(a.split_once('=')?.1));
        v.collect()
    };
    let (view, pager) = match ugrep {
        true => (eq("--view", 4), eq("--pager", 5)),
        false => (Vec::new(), long_values(&opts, "--pager", 5)),
    };
    #[rustfmt::skip]
    cmds.extend(pager.into_iter().map(|c| (c.to_string(), false))
        .chain(view.into_iter().map(|c| (c.to_string(), true))));
    cmds.iter().all(|(c, view)| {
        let w = shell_words(c);
        let Some((cmd, rest)) = w.split_first() else {
            return false;
        };
        #[rustfmt::skip]
        let cmd = ["/usr/bin/", "/bin/"].iter().find_map(|d| cmd.strip_prefix(d)).unwrap_or(cmd);
        let pager = PAGERS_OK.contains(&cmd) && rest.iter().all(|a| pager_flag(a));
        let editor = *view && VIEWERS_OK.contains(&cmd) && rest.is_empty();
        !c.contains(['|', ';', '&', '<', '>', '(', ')', '`', '$', '\n']) && (pager || editor)
    })
}

/// A plain pager flag cluster (`-R`, `-` alone), without less's `k`, `o` or `O`.
fn pager_flag(a: &str) -> bool {
    let f = |c: char| c.is_ascii_alphabetic() && !matches!(c, 'k' | 'o' | 'O');
    a == "-" || a.len() > 1 && a.starts_with('-') && a[1..].chars().all(f)
}

/// `sops` may print info or encrypt; `sops FILE` decrypts, so any other argument is refused.
fn sops_ok(args: &[&str]) -> bool {
    const INFO: &[&str] = &["version", "v", "help", "h"];
    #[rustfmt::skip]
    const BAD: &[&str] = &["d", "decrypt", "edit", "exec-env", "exec-file", "set", "rotate", "r",
        "updatekeys", "keyservice", "unset", "publish", "filestatus", "groups"];
    let info = |a: &&str| flag(a).is_some_and(|f| INFO.contains(&f));
    // Flags that take the next word as their value, and boolean ones. The first bare word or
    // unknown flag ends the walk: `-e` after it, or as a value (`--age -e`), is not a marker.
    #[rustfmt::skip]
    const VALUE: &[&str] = &["config", "age", "pgp", "kms", "gcp-kms", "azure-kv", "hc-vault-transit",
        "input-type", "output-type", "unencrypted-suffix", "encrypted-suffix", "unencrypted-regex",
        "encrypted-regex", "output", "indent", "filename-override", "shamir-secret-sharing-threshold"];
    let (mut it, mut enc) = (args.iter(), false);
    while let Some(a) = it.next() {
        let f = flag(a);
        enc = *a == "encrypt" || matches!(f, Some("e" | "encrypt"));
        match f {
            _ if enc => break,
            Some(f)
                if a.contains('=') || matches!(f, "i" | "in-place" | "verbose" | "ignore-mac") => {}
            Some(f) if VALUE.contains(&f) => drop(it.next()),
            _ => break,
        }
    }
    let bad = |a: &&str| BAD.contains(&flag(a).unwrap_or(a));
    args.is_empty() || args.iter().all(info) || (enc && !args.iter().any(bad))
}

/// `age`/`rage` may print info or encrypt; a decrypt flag in any spelling is refused.
fn age_ok(args: &[&str]) -> bool {
    let info = |a: &&str| flag(a).is_some_and(|f| matches!(f, "version" | "h" | "help"));
    #[rustfmt::skip]
    let enc = |a: &&str| flag(a).is_some_and(|f| matches!(f, "r" | "R" | "e" | "p" | "encrypt" | "recipient" | "recipients-file"));
    #[rustfmt::skip]
    let dec = |a: &&str| a.starts_with('-') && (flag(a) == Some("decrypt") || !a.starts_with("--") && a.contains('d'));
    args.is_empty() || args.iter().all(info) || (args.iter().any(enc) && !args.iter().any(dec))
}

#[rustfmt::skip]
fn mentions_secrets(text: &str) -> bool {
    ["sops", "SOPS_", "keys.txt", "hoist secrets"].iter().any(|k| text.contains(k))
        || text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .any(|t| matches!(t, "age" | "rage"))
}

/// Decrypt-lock (spec §4), an allowlist that fails closed: `sops`/`age` only print info or
/// encrypt, `hoist secrets edit|..`, the key directory, `$SOPS_AGE_KEY*`, `/proc/*/environ`,
/// env dumps naming sops, and shells reading stdin or code that mention secrets are refused.
/// Every word of a segment is tried as the command word (except in readers, file tools and
/// install-like packager commands); command strings in options are judged.
fn decrypt_rule(command: &str, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(too_deep());
    }
    let (text, bodies, targets) = strip_data_heredocs(command, depth);
    let (text, wrote) = (text.as_str(), mentions_secrets(&bodies));
    for inner in nested(text) {
        if let Some(why) = decrypt_rule(&inner, depth + 1) {
            return Some(why);
        }
    }
    let low = text.to_lowercase();
    #[rustfmt::skip]
    let key_dir = ["sops/age", "age/keys.txt"].iter().any(|k| text.contains(k)) || text.contains("keys.txt") && (low.contains("sops") || text.contains(".config") || text.contains("find"));
    if key_dir {
        return Some(decrypt_reason("the age key directory"));
    }
    if text.contains("$SOPS_AGE_KEY") || text.contains("${SOPS_AGE_KEY") {
        return Some(decrypt_reason("the age key"));
    }
    // Indirect expansion can name the variable at run time.
    if text.contains("${!")
        && (mentions_secrets(&low) || low.contains("ops_age") || text.contains("${!S"))
    {
        return Some(decrypt_reason("an indirect expansion"));
    }
    let (named, mentions) = (low.contains("sops"), mentions_secrets(text));
    let words = shell_words(text);
    let mut hits = if wrote { targets } else { Vec::new() };
    for segment in words.split(|w| w == SEP) {
        if mentions_secrets(&segment.join(" ")) {
            hits.extend(write_targets(segment));
        }
        let mut rest: Vec<&str> = segment.iter().map(String::as_str).collect();
        while rest.first().is_some_and(|w| assigned(w).is_some()) {
            rest.remove(0);
        }
        let Some(first) = rest.first() else { continue };
        let base = |w: &str| {
            w.rsplit(['/', '#'])
                .next()
                .unwrap_or(w)
                .split('@')
                .next()
                .unwrap_or(w)
                .to_string()
        };
        // Runners (`env`, `nice -n 5`, `timeout 5`, `sudo -u x`, `xargs`) run the command after
        // them: judge it as the command, in the directory `env -C` or `sudo -D` moves to. An
        // option they take that is not known may hide the real command, so after a runner no
        // command exempts its words from being tried as commands.
        let (cmd, cds, shell) = unwrap_runners(&rest);
        let script = shell.then(|| cmd.join(" ")).or_else(|| {
            let s = rest.iter().position(|w| base(w) == "strace")?;
            let p = s + rest[s..].iter().position(|w| STRACE_OPTIONAL.contains(w))?;
            let v = rest.get(p + 1)?;
            let glued = format!("{}={v}", rest[p]);
            Some(
                [&rest[..p], &[glued.as_str()], &rest[p + 2..]]
                    .concat()
                    .join(" "),
            )
        });
        if let Some(why) = script.and_then(|s| decrypt_rule(&s, depth + 1)) {
            return Some(why);
        }
        let wrapped = cmd.len() < rest.len();
        let name = base(cmd.first().unwrap_or(first));
        let args = &cmd[1.min(cmd.len())..];
        #[rustfmt::skip]
        let da = &args[..args.iter().position(|a| redirect(a)).unwrap_or(args.len())];
        let dump = match name.as_str() {
            "printenv" => da.is_empty() || named,
            "compgen" => da.iter().any(|a| matches!(*a, "-e" | "-v")),
            "env" => da
                .iter()
                .all(|a| a.starts_with('-') || assigned(a).is_some()),
            "set" => da.is_empty(),
            "export" | "declare" | "typeset" => {
                da.is_empty()
                    || da.iter().any(|a| matches!(*a, "-p" | "-x"))
                        && (named || da.iter().all(|a| a.starts_with('-')))
            }
            "ps" => da.iter().any(|a| {
                *a == "-E"
                    || !a.starts_with('-')
                        && a.contains('e')
                        && a.chars().all(|c| "aeuxwfl".contains(c))
            }),
            _ => false,
        };
        if dump {
            return Some(env_reason("the environment"));
        }
        // The verb skips git's global options and their values (`-C dir`, `-c k=v`).
        let (mut it, git) = (args.iter(), name == "git");
        let verb = loop {
            match it.next() {
                Some(a)
                    if git
                        && matches!(
                            *a,
                            "-C" | "-c"
                                | "--git-dir"
                                | "--work-tree"
                                | "--namespace"
                                | "--exec-path"
                                | "--config-env"
                        ) =>
                {
                    it.next();
                }
                Some(a) if a.starts_with('-') => {}
                other => break other.copied().unwrap_or(""),
            }
        };
        let long = |name: &str| {
            args.iter().enumerate().find_map(|(i, a)| {
                let v = a.strip_prefix(name).and_then(|r| r.strip_prefix('='));
                v.or_else(|| (*a == name).then(|| args.get(i + 1).copied()).flatten())
            })
        };
        // `bosun main-triage --run CMD` and `rg --pre CMD FILE` run a command.
        let run = match name.as_str() {
            "bosun" => long("--run").map(str::to_string),
            "rg" => long("--pre").map(|c| format!("{c} FILE")),
            #[rustfmt::skip]
            "git" if verb == "grep" => long("--open-files-in-pager").or_else(|| args.iter().enumerate().find_map(|(i, a)| a.strip_prefix("-O").map(|v| if v.is_empty() { args.get(i + 1).copied().unwrap_or("") } else { v }))).map(|c| format!("{c} FILE")),
            _ => None,
        };
        if let Some(why) = run.and_then(|c| decrypt_rule(&c, depth + 1)) {
            return Some(why);
        }
        // A pager or view runs on a file, and a shell there runs the search output as a
        // script: only known pagers pass, without expansions (`$SHELL`, `` `which sh` ``).
        let paged_at = rest.iter().position(|w| PAGED.contains(&base(w).as_str()));
        if paged_at.is_some_and(|p| !pagers_ok(&base(rest[p]), &words, &rest[p + 1..])) {
            return Some(decrypt_reason("an unknown or expanded pager"));
        }
        // `git grep --no-index` searches its paths (or the cwd) like `grep -r`, hidden ones too.
        // `grep.fallbackToNoIndex` (`-c`, `--config-env`, `GIT_CONFIG_KEY_<n>`) falls back to it.
        #[rustfmt::skip]
        let keys = words.iter().filter(|w| assigned(w).is_some_and(|k| k.starts_with("GIT_CONFIG_KEY_"))).filter_map(|w| w.split_once('=').map(|p| p.1));
        #[rustfmt::skip]
        let fallback = args.windows(2).filter(|w| matches!(w[0], "-c" | "--config-env")).map(|w| w[1])
            .chain(args.iter().filter_map(|a| a.strip_prefix("-c").or(a.strip_prefix("--config-env=")))).chain(keys)
            .any(|v| v.to_lowercase().starts_with("grep.fallbacktonoindex"));
        // `-C dir` and `--work-tree dir` move where it searches: join them with each root.
        let at = |b: &str, r: &str| match r.starts_with(['/', '~', '$']) {
            true => r.to_string(),
            false => format!("{b}/{r}"),
        };
        #[rustfmt::skip]
        let (mut dir, mut gi) = (cds.iter().fold(None::<String>, |d, v| Some(d.map_or(v.to_string(), |b| at(&b, v)))), args.iter());
        while let Some(a) = gi.next() {
            #[rustfmt::skip]
            let v = match *a {
                "-C" | "--work-tree" => gi.next().copied(),
                "-c" | "--git-dir" | "--namespace" | "--exec-path" | "--config-env" => { gi.next(); None }
                _ if a.starts_with('-') => a.strip_prefix("--work-tree=").or_else(|| a.strip_prefix("-C").filter(|v| !v.is_empty())),
                _ => break,
            };
            if let Some(v) = v {
                dir = Some(dir.map_or(v.to_string(), |b| at(&b, v)));
            }
        }
        // A key root as the base is outside any repo, so the fallback may apply there whatever
        // config set it (`GIT_CONFIG_PARAMETERS`, a gitconfig file); `../x` is not one here,
        // but `../../root`, `../../home/x` and `../../x` are.
        let key_base = dir.as_deref().map(lexical).is_some_and(|b| {
            let segs: Vec<&str> = b.split('/').collect();
            let top = segs
                .iter()
                .rposition(|s| *s == "..")
                .map(|i| segs.get(i + 1));
            #[rustfmt::skip]
            let home = top.is_none_or(|s| s.is_some_and(|s| matches!(*s, "root" | "home")));
            // Two `..` climb past a repo's parent: `../../user` from `~/repo/sub` is home.
            #[rustfmt::skip]
            let rel = ["$PWD/", "${PWD}/", "\"$PWD\"/", "/proc/self/cwd/"].iter().find_map(|p| b.strip_prefix(p)).unwrap_or(&b);
            let deep = rel.split('/').take_while(|s| *s == "..").count() >= 2;
            key_root(&b) == 2 || key_root(&b) == 1 && (home || deep)
        });
        let no_index = fallback || key_base || args.iter().any(|a| long_prefix(a, "--no-index", 6));
        if git && verb == "grep" && no_index {
            #[rustfmt::skip]
            const VALUE: &[&str] = &["-e", "-f", "-A", "-B", "-C", "-m", "--max-depth", "--threads"];
            // `GIT_ICASE_PATHSPECS`, `GIT_GLOB_PATHSPECS` and their global flags widen every
            // path (`.CONFIG`).
            #[rustfmt::skip]
            let wide = words.iter().any(|w| matches!(assigned(w).unwrap_or(w), "GIT_ICASE_PATHSPECS" | "GIT_GLOB_PATHSPECS" | "--icase-pathspecs" | "--glob-pathspecs"));
            if wide {
                return Some(decrypt_reason("a pathspec env var on a git grep search"));
            }
            // The first bare word is the pattern, unless `-e` or `-f` gave it, also in a short
            // cluster (`-eX`, `-ieX`, `-ifpat`).
            #[rustfmt::skip]
            let mut pat = args.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains(['e', 'f']));
            let (mut roots, mut it, mut paths) = (Vec::new(), it.clone(), false);
            while let Some(a) = it.next() {
                match *a {
                    "--" if !paths => paths = true,
                    _ if !paths && VALUE.contains(a) => drop(it.next()),
                    _ if !paths && a.starts_with('-') => {}
                    // Pathspec magic (`:(exclude)x`, `:!x`, `:/`) searches the whole base; as the
                    // pattern it is judged as a root too, and the next word is a root.
                    _ if a.trim_start_matches(['$', '\'', '"']).starts_with(':') => {
                        pat = true;
                        roots.push(".");
                    }
                    _ if !std::mem::replace(&mut pat, true) => {}
                    _ => roots.push(*a),
                }
            }
            let joined: Vec<String> = match &dir {
                Some(b) if roots.is_empty() => vec![lexical(b)],
                Some(b) => roots.iter().map(|r| lexical(&at(b, r))).collect(),
                None => Vec::new(),
            };
            roots.extend(joined.iter().map(String::as_str));
            if key_search(&roots, true, true) {
                return Some(decrypt_reason("a search of the age key directory"));
            }
        }
        if name == "bosun" && !wrapped {
            continue;
        }
        if READERS.contains(&name.as_str()) {
            let env = |w: &&str| procenv(w);
            #[rustfmt::skip]
            let pos: Vec<&str> = da.iter().copied().filter(|a| !a.starts_with('-')).collect();
            #[rustfmt::skip]
            let pat_flag = args.iter().any(|a| matches!(*a, "-e" | "-f") || a.starts_with("--file"));
            let finds = SEARCHERS.contains(&name.as_str());
            let search = finds && !pat_flag;
            // A search pattern naming the key marker is data, unless it searches home or `.config`.
            let home = |a: &&str| {
                a.starts_with(['~', '/']) || a.contains("$HOME") || a.contains(".config")
            };
            let pat_ok = search && !pos.is_empty() && !pos[1..].iter().any(home);
            if args
                .iter()
                .any(|a| keyline(a) && !(pat_ok && pos.first() == Some(a)))
            {
                return Some(decrypt_reason("an age secret key"));
            }
            // A recursive search whose root holds the age key reads it, whatever the pattern:
            // `.config` and its ancestors always; home (hidden dirs included) for the grep
            // family, and for rg and ag only with --hidden, -u or -. .
            let (rg, always) = (
                matches!(name.as_str(), "rg" | "ag"),
                matches!(name.as_str(), "rg" | "ag" | "ack" | "ack-grep" | "rgrep"),
            );
            // Long options by unique prefix (`--rec`, `--der`, `--dir=rec`, ag `--hi`, `--u`).
            let short = |a: &&str| a.starts_with('-') && !a.starts_with("--");
            // `-d` in a short cluster takes the rest of it, or the next word (`-nd recurse`).
            #[rustfmt::skip]
            let d = args.iter().enumerate().filter(|(_, a)| short(a)).filter_map(|(i, a)| Some((i, a[1..].split_once('d')?)))
                .map(|(i, (_, v))| if v.is_empty() { args.get(i + 1).copied().unwrap_or("") } else { v });
            #[rustfmt::skip]
            let dirs = long_values(args, "--directories", 4).into_iter().chain(d)
                .any(|v| v.len() >= 3 && "recurse".starts_with(v));
            let long =
                |a: &&str, opts: &[(&str, usize)]| opts.iter().any(|(f, m)| long_prefix(a, f, *m));
            #[rustfmt::skip]
            let recursive = always || dirs || args.iter().any(|a| long(a, &[("--recursive", 5), ("--dereference-recursive", 5)]) || short(a) && a.contains(['r', 'R']));
            #[rustfmt::skip]
            let hidden = !rg || args.iter().any(|a| long(a, &[("--hidden", 4), ("--unrestricted", 3)]) || short(a) && a.contains(['u', '.']));
            if finds && key_search(&pos, recursive, hidden) {
                return Some(decrypt_reason("a search of the age key directory"));
            }
            let cfg = |a: &&str| a.contains(".config/") && a.contains(['*', '?', '[', ']']);
            if matches!(name.as_str(), "cat" | "head" | "tail") && args.iter().any(cfg) {
                return Some(decrypt_reason("a glob in .config"));
            }
            if args.iter().any(env)
                && !(search && pos.first().is_some_and(env) && !pos[1..].iter().any(env))
            {
                return Some(env_reason("the process environment"));
            }
            // ack and ugrep take commands in options (`--pager`, `--view`, `--filter`, abbreviated)
            // and from the environment, so only their pattern is data.
            let paged = PAGED.contains(&name.as_str());
            if !paged && !wrapped {
                continue;
            }
            let filters = match paged && name.starts_with('u') {
                true => long_values(args, "--filter", 6),
                false => Vec::new(),
            };
            for c in filters.iter().flat_map(|f| f.split(',')) {
                let c = c.split_once(':').map_or(c, |p| p.1);
                if let Some(why) = decrypt_rule(c, depth + 1) {
                    return Some(why);
                }
            }
        }
        #[rustfmt::skip]
        let sub = args.iter().find(|a| !a.starts_with('-')).is_some_and(|a| PKG_SUBS.contains(a));
        let safe = !wrapped
            && (FILE_TOOLS.contains(&name.as_str()) || PACKAGERS.contains(&name.as_str()) && sub);
        // Data words: message and query option values, search patterns.
        let (msg, vi) = (
            matches!(name.as_str(), "git" | "gh"),
            segment.iter().position(|x| x == verb),
        );
        let (gs, ss) = (
            name == "git" && verb == "grep",
            name == "gh" && verb == "search",
        );
        let api = !wrapped && (name.as_str(), verb) == ("gh", "api");
        let mut data = vec![false; segment.len()];
        let (mut skip, mut pat_done) = (false, false);
        for (i, w) in segment.iter().enumerate() {
            let opt = w.trim_start_matches('-').split('=').next().unwrap_or("");
            let opts: &[&str] = match (name.as_str(), verb) {
                ("gh", _) => GH_DATA,
                ("git", "log") => &["S", "G", "grep"],
                ("git", "grep") => &["e"],
                _ => &["m", "message"],
            };
            let pat = !w.starts_with('-') && vi.is_some_and(|v| i > v) && (ss || gs && !pat_done);
            pat_done |= pat;
            if std::mem::take(&mut skip) || pat {
                data[i] = true;
            } else if msg && w.starts_with('-') && opts.contains(&opt) {
                (data[i], skip) = (true, !w.contains('='));
            }
        }
        // ack and ugrep: the `-e`/`--regexp` values, else the first bare word that is not the
        // value of a long option.
        let paged = PAGED.contains(&name.as_str());
        if paged {
            let (a0, mut seen) = (segment.len() - args.len(), false);
            let e = da
                .iter()
                .any(|a| matches!(*a, "-e" | "--regexp") || a.starts_with("--regexp="));
            for (j, a) in da.iter().enumerate() {
                let prev = if j == 0 { "" } else { da[j - 1] };
                // ugrep's `--pager` and `--view` take no next word.
                #[rustfmt::skip]
                let bare = name.starts_with('u') && (long_prefix(prev, "--pager", 5) || long_prefix(prev, "--view", 4));
                let value = prev.starts_with("--") && !prev.contains('=') && !bare;
                #[rustfmt::skip]
                let pat = if e { matches!(prev, "-e" | "--regexp") || a.starts_with("--regexp=") }
                    else { !seen && !a.starts_with('-') && !value };
                seen |= pat;
                data[a0 + j] |= pat;
            }
        }
        // After a runner the command may be misread: no word is data.
        if wrapped {
            data.fill(false);
        }
        // Options run command strings: a word or option value with whitespace is a command.
        for w in segment
            .iter()
            .zip(&data)
            .filter(|(_, d)| !**d)
            .map(|(w, _)| w)
        {
            let w = w.as_str();
            if procenv(w) {
                return Some(env_reason("the process environment"));
            }
            let t = if assigned(w).is_some() {
                w.split_once('=').map_or(w, |p| p.1)
            } else {
                w
            };
            if keyline(w) {
                return Some(decrypt_reason("an age secret key"));
            }
            if !safe && !(api && !w.starts_with('-')) && t.contains("SOPS_AGE_KEY") {
                return Some(decrypt_reason("the age key"));
            }
            if w.contains(".config/") && w.contains(['*', '?', '[', ']']) {
                return Some(decrypt_reason("a glob in .config"));
            }
            let value = w.split_once('=').map(|(_, v)| v);
            let strs = [
                Some(w.strip_prefix("-O").unwrap_or(w)).filter(|w| w.contains(char::is_whitespace)),
                value.filter(|v| v.contains(char::is_whitespace) || v.starts_with(['|', '!'])),
            ];
            for c in strs.into_iter().flatten() {
                if let Some(why) = decrypt_rule(c.trim_start_matches(['|', '!']), depth + 1) {
                    return Some(why);
                }
            }
        }
        let off = segment.len() - rest.len();
        let tried = if safe || paged && !wrapped {
            0
        } else {
            rest.len()
        };
        for i in 0..tried {
            if data[i + off] {
                continue;
            }
            let (n, after) = (base(rest[i]), &rest[i + 1..]);
            let tgt = |a: &str| {
                hits.iter()
                    .any(|t| t.trim_start_matches("./") == a.trim_start_matches("./"))
            };
            #[rustfmt::skip]
            let refused = match n.as_str() {
                "sops" => !sops_ok(&after[..after.iter().position(|a| redirect(a)).unwrap_or(after.len())]),
                "age" | "rage" => !age_ok(&after[..after.iter().position(|a| redirect(a)).unwrap_or(after.len())]),
                "hoist" => after.iter().position(|a| *a == "secrets").is_some_and(|p| after[p + 1..].iter().take_while(|a| **a != "--" && **a != "exec").any(|a| HOIST_SECRETS.contains(a))),
                "alias" => after.iter().filter_map(|a| a.split_once('=')).any(|(_, v)| matches!(base(v.split(' ').next().unwrap_or("")).as_str(), "sops" | "age" | "rage")),
                "awk" | "gawk" | "mawk" => after.iter().any(|a| a.contains("ENVIRON")),
                "jq" => after.iter().any(|a| a.contains("$ENV") || a.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.'))).any(|t| t == "env" || t.starts_with("env."))),
                n if INTERPS.contains(&n) && after.iter().any(|a| ENV_READS.iter().any(|e| a.contains(e) && (*e != "environ" || runs_code(n, after)))) => true,
                _ if i == 0 && tgt(rest[0]) => true,
                n if (SHELLS.contains(&n) || n == ".") && after.iter().any(|a| tgt(a)) => true,
                n if SHELLS.contains(&n) && mentions => {
                    let interp = INTERPS.contains(&n);
                    let code = runs_code(n, after);
                    let file = after.iter().any(|a| !a.starts_with(['-', '<']));
                    let stdin = after.iter().any(|a| matches!(*a, "-s" | "-") || a.starts_with('<'));
                    if code { interp || after.iter().any(|a| a.contains("{}") || a.contains('$')) } else { !file || stdin }
                }
                _ => false,
            };
            if refused {
                // Code or a filter that reads the environment (and names no secrets tool).
                let env_read = !mentions
                    && (matches!(n.as_str(), "awk" | "gawk" | "mawk" | "jq")
                        || INTERPS.contains(&n.as_str())
                            && after
                                .iter()
                                .any(|a| ENV_READS.iter().any(|e| a.contains(e))));
                if env_read {
                    return Some(env_reason(&format!("the environment (`{n}`)")));
                }
                return Some(decrypt_reason(&format!("`{n}`")));
            }
        }
    }
    None
}

fn heredoc_at(line: &str) -> Option<usize> {
    let (mut quote, b) = (None, line.as_bytes());
    for (i, c) in line.char_indices() {
        let op = c == '<' && b.get(i + 1) == Some(&b'<');
        // In double quotes, `<<` counts after an unclosed `$(`: `"$(cat <<'EOF'`.
        let sub = || line[..i].matches("$(").count() > line[..i].matches(')').count();
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), _) if op && sub() => return (b.get(i + 2) != Some(&b'<')).then_some(i),
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, _) if op => return (b.get(i + 2) != Some(&b'<')).then_some(i),
            _ => {}
        }
    }
    None
}

/// The text without the bodies of quoted-delimiter heredocs (unless their line has a shell),
/// those bodies, and the files the opening lines write (`> f`, `>> f`, `tee f`). When only
/// interpreters (no shell) are on the line, an interpreter owns the heredoc and the line writes
/// no file, the body stays code with its prose literals emptied.
fn strip_data_heredocs(command: &str, depth: usize) -> (String, String, Vec<String>) {
    let (mut out, mut bodies) = (String::new(), String::new());
    let (mut skip, mut targets) = (None::<(String, bool, bool)>, Vec::new());
    for line in command.split_inclusive('\n') {
        if let Some((delim, dash, code)) = &skip {
            let l = line.trim_end_matches('\n');
            let l = if *dash { l.trim_start_matches('\t') } else { l };
            if l != delim && l.starts_with(delim.as_str()) {
                return (command.to_string(), bodies, targets);
            }
            if l == delim {
                skip = None;
                out.push_str(line);
            } else if *code {
                out.push_str(&literals::blank_prose(line, depth));
            } else {
                bodies.push_str(line);
            }
            continue;
        }
        out.push_str(line);
        if let Some(at) = heredoc_at(line) {
            let spec = line[at + 2..].trim_start_matches('-').trim_start();
            let dash = line[at + 2..].starts_with('-');
            let quote = spec.chars().next().filter(|c| matches!(c, '\'' | '"'));
            let words = shell_words(line);
            let names = || words.iter().map(|w| w.rsplit('/').next().unwrap_or(w));
            let reads = names().any(|w| SHELLS.contains(&w));
            // Only interpreters (`python3 -`), no shell or `source`/`xargs`/`eval`; the command
            // of the segment that holds `<<` is one (not `cat <<'EOF' | python3 ..`), and the
            // line writes no file (`python3 - > x.sh <<'EOF'` is a script for later).
            let seg = &line[line[..at].rfind(['|', ';', '&', '(']).map_or(0, |i| i + 1)..at];
            let owner = shell_words(seg).into_iter().find(|w| assigned(w).is_none());
            let owned =
                owner.is_some_and(|w| INTERPS.contains(&w.rsplit('/').next().unwrap_or(&w)));
            let writes = write_targets(&words);
            let interp = owned
                && writes.is_empty()
                && names().all(|w| !SHELLS.contains(&w) || INTERPS.contains(&w));
            if let Some(q) = quote
                && let Some(end) = spec[1..].find(q)
                && (!reads || interp)
            {
                skip = Some((spec[1..=end].to_string(), dash, reads));
                targets.extend(writes);
            }
        }
    }
    (out, bodies, targets)
}

fn http_rule(segment: &str) -> Option<&'static str> {
    if GRAPHQL_WRITES.iter().any(|m| segment.contains(m)) {
        return Some("GraphQL merge or ref mutation");
    }
    let github = segment.contains("api.github.com") || segment.contains("/repos/");
    (github && HTTP_WRITES.iter().any(|w| segment.contains(w)))
        .then_some("GitHub API merge, ref, release or auto-merge write")
}

fn gh_rule(args: &[&str], segment: &str) -> Option<&'static str> {
    // Quotes and backslashes do not hide a word (`gh pr "merge"`, `gh p""r merge`), and a global
    // `-R o/r` or `--repo o/r` before the command does not move it.
    let clean: Vec<String> = args
        .iter()
        .map(|a| a.replace(['\'', '"', '\\'], ""))
        .collect();
    let mut i = 0;
    while i < clean.len() && clean[i].starts_with('-') {
        let a = clean[i].as_str();
        let value_next = matches!(a, "-R" | "--repo");
        i += if value_next { 2 } else { 1 };
    }
    let at = i.min(clean.len());
    // `gh api` is judged on the words as written (quotes shape a GraphQL query).
    if clean.get(at).map(String::as_str) == Some("api") {
        return api_rule(&args[at + 1..], segment);
    }
    let mut args: Vec<&str> = clean[at..].iter().map(String::as_str).collect();
    // `gh pr -R o/r merge 5`, `gh release --repo=o/r create v1`: a flag after the group word does
    // not move the command.
    if !args.is_empty() {
        let mut j = 1;
        while j < args.len() && args[j].starts_with('-') {
            j += if matches!(args[j], "-R" | "--repo") {
                2
            } else {
                1
            };
        }
        args.drain(1..j.min(args.len()));
    }
    match &args[..] {
        ["pr", "merge", ..] => Some("gh pr merge"),
        [
            "release",
            "create" | "edit" | "upload" | "delete" | "delete-asset",
            ..,
        ] => Some("a release write makes or moves a tag"),
        ["alias", "set" | "import", ..] => Some("a gh alias makes commands the guard cannot see"),
        ["workflow", "run", ..] => Some("a workflow run can release or deploy"),
        ["secret" | "variable", "set" | "delete" | "remove", ..] => {
            Some("secrets and variables are the owner's")
        }
        _ => None,
    }
}

/// `gh api ARGS`: the args after `api`.
fn api_rule(rest: &[&str], segment: &str) -> Option<&'static str> {
    // Quotes and backslashes do not hide a path word (`pulls/1/'merge'`).
    let clean: Vec<String> = rest
        .iter()
        .map(|a| a.replace(['\'', '"', '\\'], ""))
        .collect();
    if GRAPHQL_WRITES.iter().any(|m| segment.contains(m)) {
        Some("GraphQL merge or ref mutation")
    } else if graphql_hidden(rest) {
        Some("a GraphQL query the guard cannot see")
    } else if gh_api_read(rest, segment) {
        None
    } else if clean.iter().any(|a| a.contains("/merge")) {
        Some("merge API")
    } else if clean.iter().any(|a| a.contains("/releases")) {
        Some("a release makes a tag")
    } else if clean.iter().any(|a| a.contains("auto_merge")) {
        Some("auto-merge API")
    } else if clean
        .iter()
        .any(|a| a.contains("git/refs") || a.contains("/tags"))
    {
        Some("ref or tag API")
    } else {
        None
    }
}

/// A `gh api` read: every method flag says GET and there is no field or input flag (`-f`, `-F`,
/// `--field`, `--raw-field`, `--input`, which make gh default to POST). Fails closed: a segment
/// with `$`, a backtick, a backslash or unbalanced quotes (cut at a quoted separator) is not a
/// read, nor is a short-flag cluster holding `X`, `f` or `F` other than `-X GET`.
fn gh_api_read(args: &[&str], segment: &str) -> bool {
    let odd = |q: char| segment.matches(q).count() % 2 == 1;
    if segment.contains(['$', '`', '\\']) || odd('\'') || odd('"') {
        return false;
    }
    let args: Vec<String> = args.iter().map(|a| a.replace(['\'', '"'], "")).collect();
    let get = |m: &str| m.eq_ignore_ascii_case("GET");
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if matches!(a, "-X" | "--method") {
            if !args.get(i + 1).is_some_and(|m| get(m)) {
                return false;
            }
            i += 1;
        } else if let Some(m) = a.strip_prefix("--method=").or_else(|| a.strip_prefix("-X")) {
            if !get(m) {
                return false;
            }
        } else if ["--field", "--raw-field", "--input"]
            .iter()
            .any(|f| a.starts_with(f))
            || (a.starts_with('-') && !a.starts_with("--") && a.contains(['X', 'f', 'F']))
        {
            return false;
        }
        i += 1;
    }
    true
}

/// A `gh api graphql` call whose query the guard cannot read: `--input`, a `query=@file`, or a
/// query outside single quotes with `$` or a backtick.
fn graphql_hidden(args: &[&str]) -> bool {
    let unq = |a: &str| a.trim_matches(['\'', '"']).to_string();
    if !args.iter().any(|a| {
        let a = unq(a);
        a == "graphql" || a.ends_with("/graphql")
    }) {
        return false;
    }
    let mut i = 0;
    while let Some(&a) = args.get(i) {
        i += 1;
        if a.starts_with("--input") {
            return true;
        }
        let value = match a {
            "-f" | "-F" | "--field" | "--raw-field" => args.get(i).copied(),
            _ if a.starts_with("--") => a
                .strip_prefix("--field=")
                .or_else(|| a.strip_prefix("--raw-field=")),
            _ => a.strip_prefix("-f").or_else(|| a.strip_prefix("-F")),
        };
        let Some(q) = value.and_then(|v| v.trim_start_matches(['\'', '"']).strip_prefix("query="))
        else {
            continue;
        };
        if q.starts_with('@') || (!q.starts_with('\'') && q.contains(['$', '`', '\\'])) {
            return true;
        }
    }
    false
}

/// A git config key that defines an alias (`alias.x`, any case, quoted or not).
fn alias_key(a: &str) -> bool {
    a.trim_start_matches(['\'', '"'])
        .to_ascii_lowercase()
        .starts_with("alias.")
}

const GIT_ALIAS: &str = "a git alias hides the command it runs";

const PUSH_CONFIG: &str = "git config sets where this push goes (`remote.<r>.push`, `remote.<r>.mirror` or `push.default`); name the refspec, as in `git push -u origin claude/fix-1`";

const PUSH_ALONE: &str =
    "a push without `src:dst` goes where git config says, which another command here may change";

/// `PUSH_ALONE` with the way out for this push: its own branch as `b:b`.
fn alone_fix(words: &[&str]) -> String {
    let after = words.iter().skip_while(|w| **w != "push").skip(1);
    let plain: Vec<&str> = after.copied().filter(|w| !w.starts_with('-')).collect();
    let b = plain
        .get(1)
        .map_or("<branch>", |b| b.trim_matches(['\'', '"']));
    let b = b.split(':').next().unwrap_or(b);
    format!("{PUSH_ALONE}; run the push as its own command, or name both sides: `{b}:{b}`")
}

const FETCH_PLAIN: &str = "a fetch or pull next to other shell commands, whose environment the guard cannot read, or a git command that runs a local command (`ext::`, `fd::`, `-c`, `--upload-pack`, `--exec-path` with a fetch); run a plain git fetch/pull as its own command (`cd` and other `git` commands may join it with `&&`)";

const GLOB_WORD: &str = "a glob (`*`, `?`, `[`) in a command name or a git word, which bash expands against files the guard cannot see; spell the command name and git words literally, or quote them";

const FETCH_MOVES: &str = "a fetch into the default branch or into a ref the guard cannot read; name the branch literally";

const BUILT_SUB: &str = "a git subcommand built by the shell, which the guard cannot see: \
    write the subcommand out, not a variable or substitution";

const UNREAD_PUSH: &str = "a push target the guard cannot read; name the branch literally, as in `git push -u origin claude/fix-1`";

/// A command that is one plain `git` call: no other command, `$`, backtick, group, redirect
/// (but `2>&1`), assignment, `source` or `eval`; a last `| tail -N` or `| head -N` only reads.
fn alone(command: &str) -> bool {
    let c = command.trim().replace("2>&1", "");
    let c = match c.rsplit_once('|') {
        Some((git, tail)) => {
            let w: Vec<&str> = tail.split_whitespace().collect();
            let n = |x: &str| {
                x.trim_start_matches('-')
                    .bytes()
                    .all(|b| b.is_ascii_digit())
            };
            let reads = matches!(w.as_slice(), [t, x] if matches!(*t, "tail" | "head") && n(x))
                || matches!(w.as_slice(), [t, "-n", x] if matches!(*t, "tail" | "head") && n(x));
            if !reads {
                return false;
            }
            git.trim_end().to_string()
        }
        None => c,
    };
    c.starts_with("git ") && !c.contains(['\n', ';', '&', '|', '$', '`', '(', ')', '<', '>'])
}

/// A git config key that sets where a push goes: `remote.<r>.push`, `remote.<r>.mirror`.
/// A key with a backslash is one the guard cannot read: fail closed.
fn push_key(k: &str) -> bool {
    let k = cfg_key(k);
    k.contains('\\')
        || k == "include.path"
        || k == "push.followtags"
        || k.starts_with("includeif.")
        || k.starts_with("remote.") && (k.ends_with(".push") || k.ends_with(".mirror"))
}

/// A config key as git reads it: quotes removed, lowercase.
fn cfg_key(k: &str) -> String {
    k.replace(['\'', '"'], "").to_ascii_lowercase()
}

/// A ref as git reads it: `//` and `/./` collapse and a leading `./` goes (`refs//tags/v1`).
fn norm_ref(r: &str) -> String {
    let mut r = r.trim_start_matches("./").to_string();
    while r.contains("//") || r.contains("/./") {
        r = r.replace("//", "/").replace("/./", "/");
    }
    r
}

/// Whether a push's destination names the default branch (`main`, `refs/heads/main`, and
/// `heads/main`, which git expands to `refs/heads/main`).
fn names_default(dst: &str, default_branch: &str) -> bool {
    let d = norm_ref(dst);
    let d = d.strip_prefix("refs/").unwrap_or(&d);
    d == default_branch || d.strip_prefix("heads/") == Some(default_branch)
}

/// `push.default` values that push only the current branch to its own name (`HEAD` is judged).
fn head_only(v: &str) -> bool {
    matches!(
        v.trim_matches(['\'', '"']).to_ascii_lowercase().as_str(),
        "simple" | "current" | "nothing"
    )
}

/// `git subtree push -P <prefix> <repository> [<commit>:]<ref>`: refused when the ref is
/// missing or any word names the default branch or a tag (quoted messages split into words,
/// so this fails closed).
fn subtree_push(rest: &[&str], default_branch: &str) -> Option<&'static str> {
    if rest.iter().any(|a| a.contains(['$', '`', '\\'])) {
        return Some(UNREAD_PUSH);
    }
    let (mut words, mut it) = (Vec::new(), rest.iter().map(|a| a.trim_matches(['\'', '"'])));
    while let Some(a) = it.next() {
        if matches!(
            a,
            "-P" | "--prefix" | "-m" | "--message" | "-b" | "--branch" | "--onto" | "--annotate"
        ) {
            it.next();
        } else if !a.starts_with('-') {
            words.push(a);
        }
    }
    if words.len() < 2 {
        return Some(UNREAD_PUSH);
    }
    for w in words {
        let dst = w.rsplit(':').next().unwrap_or(w);
        if w.starts_with('+') || dst.contains("refs/tags/") || version_like(dst) {
            return Some("force, tag or delete push");
        }
        if names_default(dst, default_branch) {
            return Some("push to the default branch");
        }
    }
    None
}

/// `git merge --ff-only [origin/<default>|@{u}|@{upstream}]`: catches the local branch up
/// with its remote, never brings in other work.
fn ff_only(rest: &[&str], default_branch: &str) -> bool {
    let refs: Vec<&&str> = rest.iter().filter(|a| **a != "--ff-only").collect();
    rest.len() == refs.len() + 1
        && refs.len() <= 1
        && refs.iter().all(|r| {
            matches!(**r, "@{u}" | "@{upstream}")
                || r.strip_prefix("origin/") == Some(default_branch)
        })
}

/// The value options of `git fetch`; a long one also by a prefix (`--dept 1`), as git takes it.
const FETCH_VALUES: &str = "-s --strategy -X --strategy-option --depth --deepen --shallow-since \
    --shallow-exclude -j --jobs --upload-pack --negotiation-tip --filter -o --server-option \
    --recurse-submodules-default --submodule-prefix --refmap";

/// The flag letters of a short-option cluster (`-pPt`) up to its first value option (`o`, `j`,
/// `s`, `X`), whose value is the rest of the cluster or, when it is the last letter, the next word.
fn short_flags(a: &str) -> Option<&str> {
    let c = a
        .strip_prefix('-')
        .filter(|c| !c.is_empty() && !c.starts_with('-'))?;
    Some(match c.find(['o', 'j', 's', 'X']) {
        Some(i) => &c[..=i],
        None => c,
    })
}

/// The plain words of `git fetch` (the remote, then refspecs) and its `--refmap` values.
fn fetch_words(rest: &[&str]) -> (Vec<String>, Vec<String>) {
    let takes_value = |a: &str| {
        FETCH_VALUES.split_whitespace().any(|v| v == a)
            // `-po origin`: a cluster whose first value option is its last letter takes the next
            // word (`-Xtheirs` holds its value).
            || short_flags(a).is_some_and(|f| f.len() + 1 == a.len() && f.ends_with(['o', 'j', 's', 'X']))
            || (a.len() > 3
                && a.starts_with("--")
                && a != "--recurse-submodules"
                && FETCH_VALUES.split_whitespace().any(|v| v.starts_with(a)))
    };
    let (mut words, mut maps) = (Vec::new(), Vec::new());
    let mut it = rest.iter().map(|a| a.replace(['\'', '"'], ""));
    while let Some(a) = it.next() {
        // A `--refmap` value (also `--refm`, `--refma`) is a refspec too.
        let key = a.split('=').next().unwrap_or(&a);
        if key.len() >= 6 && "--refmap".starts_with(key) {
            match a.split_once('=') {
                Some((_, m)) => maps.push(m.to_string()),
                None => maps.extend(it.next()),
            }
        } else if takes_value(&a) {
            it.next();
        } else if !a.starts_with('-') {
            words.push(a);
        }
    }
    (words, maps)
}

/// A local-repository fetch word: `file:`, a variable, `./`, `../`, `/`, `~` (`./ev:il`), `.x`.
fn local_word(w: &str) -> bool {
    w.starts_with("file:")
        || ["./", "../", "/", "~"].iter().any(|p| w.starts_with(p))
        || w.contains(['$', '`'])
        || (!w.contains(':') && w.starts_with('.'))
}

/// Whether the fetch reads only `origin` or `upstream` (or the default remote; not `--all` or
/// `--multiple` in any prefix): any other name may be a directory (`evil2`).
fn fetch_trusted(rest: &[&str]) -> bool {
    let (words, _) = fetch_words(rest);
    let many = |a: &&str| a.len() >= 4 && ["--all", "--multiple"].iter().any(|o| o.starts_with(a));
    !rest.iter().any(many)
        && !words.iter().any(|w| local_word(w))
        && words
            .first()
            .is_none_or(|w| matches!(w.as_str(), "origin" | "upstream"))
}

/// Why a command is refused when a git fetch or pull is not a plain chain of `git ..`/`cd ..`
/// segments, or a git command runs a local command (spec §4, fetch rules).
fn fetch_not_plain(command: &str) -> Option<&'static str> {
    let text = split_heredocs(command).0.replace("2>&1", "");
    let cut = |c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '{' | '}');
    // `fe""tch` and `\\fetch` run fetch too.
    let base = |w: &str| {
        w.replace(['\'', '"', '\\'], "")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    };
    let bare = |w: &str| matches!(base(w).trim_start_matches("git-"), "fetch" | "pull");
    let git = |w: &str| base(w) == "git" || base(w).starts_with("git-");
    // `$'\\x65xt::'` and `$"..."` quoting can spell any word: refused in any git command.
    if !text.split(cut).any(git) {
        return None;
    }
    if text.contains("$'") || text.contains("$\"") {
        return Some(FETCH_PLAIN);
    }
    // `ext::<cmd>` and `fd::` transports run a command or read a descriptor, in any git command
    // (`remote add x 'ext::sh p'`, then a fetch), however quoted.
    let unquoted = text.replace(['\'', '"', '\\'], "");
    if unquoted.contains("ext::") || unquoted.contains("fd::") {
        return Some(FETCH_PLAIN);
    }
    // `git remote [-v] update` fetches too.
    let toks: Vec<&str> = unquoted.split(cut).filter(|t| !t.is_empty()).collect();
    let remote = toks.iter().position(|t| *t == "remote");
    let update = remote.is_some_and(|p| toks[p..].contains(&"update"));
    if !text.split(cut).any(bare) && !update {
        return None;
    }
    // Any `$` in a fetch or pull (a lone one too) hides a word the guard cannot read.
    if text.contains('$') {
        return Some(FETCH_MOVES);
    }
    let plain = |seg: &str| {
        let w: Vec<&str> = seg.split_whitespace().collect();
        match w.first() {
            // Config (`-c protocol.ext.allow=always`), `--upload-pack` (`-u`, `--upl`) and
            // `--exec-path` run code: words are read unquoted (`'-c'`).
            Some(c) if *c == "git" || base(c).starts_with("git-") => {
                let u: Vec<String> = w.iter().map(|a| a.replace(['\'', '"', '\\'], "")).collect();
                // A git run inside another subcommand (`bisect run`, `submodule foreach`) with a
                // fetch: the guard cannot tell where it reads config.
                let mut sub = 1;
                while u.get(sub).is_some_and(|a| a.starts_with('-')) {
                    let pair = [
                        "-C",
                        "-c",
                        "--git-dir",
                        "--work-tree",
                        "--namespace",
                        "--config-env",
                    ];
                    sub += 1 + usize::from(pair.contains(&u[sub].as_str()));
                }
                let inner = u.iter().skip(sub + 1).position(|a| git(a));
                let fetch = |a: &String| bare(a) || a == "update";
                if inner.is_some_and(|p| u[sub + 1 + p..].iter().any(fetch)) {
                    return false;
                }
                !(u.iter().any(|a| bare(a))
                    && u.iter().any(|a| {
                        a == "-c"
                            || ["--config-env", "--upl", "--ex"]
                                .iter()
                                .any(|p| a.starts_with(p))
                            || short_flags(a).is_some_and(|f| f.contains('u'))
                    }))
            }
            Some(&"cd") => w.len() <= 2,
            Some(&("tail" | "head")) => {
                w.len() == 2 && w[1].trim_start_matches('-').parse::<u32>().is_ok()
            }
            _ => false,
        }
    };
    // A lone `git` segment is judged by `git_rule`, which refuses `\\` in a fetch.
    let segs: Vec<&str> = text
        .split([';', '&', '|', '\n'])
        .filter(|g| !g.trim().is_empty())
        .collect();
    (text.contains(['`', '(', ')', '{', '}', '<', '>'])
        || (segs.len() > 1 && text.contains('\\'))
        || segs.iter().any(|seg| !plain(seg)))
    .then_some(FETCH_PLAIN)
}

/// An unquoted `*`, `?` or `[` in a command word or any git word (`/usr/bin/gi[t]`, `git f?tch`);
/// `[` and `[[` alone are the test builtin.
fn glob_rule(command: &str, top: bool) -> Option<&'static str> {
    let (mut segs, mut word, mut glob, mut quote, mut esc) =
        (vec![Vec::new()], String::new(), false, None, false);
    // Words with whether they hold an unquoted glob and whether any part was quoted.
    let mut quoted = false;
    for c in split_heredocs(command).0.chars().chain([' ']) {
        match (quote, c) {
            _ if esc => (esc, _) = (false, word.push(c)),
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\\') => esc = true,
            (None, '\'' | '"') => (quote, quoted) = (Some(c), true),
            (None, c) if c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')') => {
                if !word.is_empty() || glob || quoted {
                    let w = (std::mem::take(&mut word), std::mem::take(&mut glob));
                    segs.last_mut()?
                        .push((w.0, w.1, std::mem::take(&mut quoted)));
                }
                if !c.is_whitespace() || c == '\n' {
                    segs.push(Vec::new());
                }
            }
            (None, c) => (glob, _) = (glob || matches!(c, '*' | '?' | '['), word.push(c)),
        }
    }
    static VAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\$(?:\{([A-Za-z_]\w*)\}|([A-Za-z_]\w*))$").expect("static regex")
    });
    // Variables the shell sets itself (they persist across calls, like `PWD` after a `cd`).
    const SHELL_VARS: &str = "_ PWD OLDPWD HOME IFS PS1 PS2 PS3 PS4 PROMPT_COMMAND ENV SHELLOPTS \
        BASHOPTS CDPATH PATH SHELL REPLY OPTARG OPTIND RANDOM LINENO SECONDS FUNCNAME GROUPS \
        DIRSTACK PIPESTATUS HOSTNAME HOSTTYPE MACHTYPE OSTYPE PPID UID EUID SHLVL COLUMNS LINES";
    static BRACE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{[^{}]*(,|\.\.)[^{}]*\}").expect("static regex"));
    // A glob whose last component can be `git` (`gi[t]`, `g?t`, `*`; `[..]` is one character).
    static CLASS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[[^\]]*\]").expect("static regex"));
    let names = |w: &str, name: &str| {
        let pat = CLASS.replace_all(w.rsplit('/').next().unwrap_or(w), "?");
        let re = regex::escape(&pat).replace(r"\*", ".*").replace(r"\?", ".");
        Regex::new(&format!("^{re}$")).is_ok_and(|r| r.is_match(name))
    };
    let names_git = |w: &str| names(w, "git");
    segs.iter().find_map(|seg| {
        let git = seg
            .iter()
            .any(|w| w.0.rsplit('/').next() == Some("git") || w.1 && names_git(&w.0));
        // A fetch, pull or update word in a quoted or `=` value of a git command runs a fetch
        // the guard cannot place (`rebase -x 'cd x && git fetch'`, `-c core.editor=..`); a
        // message or search value (`-m`, `--grep`, `--format`) is prose.
        let prose = [
            "-m",
            "--message",
            "--grep",
            "-S",
            "-G",
            "--format",
            "--pretty",
        ];
        let is_prose = |i: usize, w: &str| {
            i > 0 && prose.contains(&seg[i - 1].0.as_str())
                || prose
                    .iter()
                    .any(|f| w.starts_with(f) && (f.len() == 2 || w[f.len()..].starts_with('=')))
        };
        let gitw = |w: &str| {
            w.rsplit('/')
                .next()
                .is_some_and(|b| b == "git" || b.starts_with("git-"))
        };
        // A plain `"$NAME"` passes only as a global `-C`, `--git-dir` or `--work-tree` value (git
        // takes those literally; a positional one can expand to an option), never after a
        // builtin or function that can set a variable (`source`, `eval`, `read`, `f() {..}`).
        let g = seg.iter().position(|w| !w.2 && gitw(&w.0)).unwrap_or(0);
        let mut sub = g + 1;
        while seg.get(sub).is_some_and(|w| w.0.starts_with('-')) {
            let pair = ["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env"];
            sub += 1 + usize::from(pair.contains(&seg[sub].0.as_str()));
        }
        let sets = command
            .split(|c: char| c.is_whitespace() || ";&|()".contains(c))
            .any(|t| {
                "source . eval declare typeset read export printf mapfile readarray local let function"
                    .split(' ')
                    .any(|b| b == t)
            })
            || command.contains("()");
        let push = seg.get(sub).is_some_and(|s| s.0 == "push");
        let place = |i: usize| {
            let prev = i.checked_sub(1).map_or("", |p| seg[p].0.as_str());
            !sets && i < sub && matches!(prev, "-C" | "--git-dir" | "--work-tree")
        };
        let names_fetch = |w: &str, i: usize| {
            let cut = |c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '=');
            // Any `$` or backtick builds words (`${IFS}`, `$(..)`), unless the value is one
            // variable (`"$BRANCH"`) the command does not set: it comes from the session.
            let var = VAR.captures(w).and_then(|c| c.get(1).or(c.get(2)));
            // `git push` refuses any `$` itself ("name the branch literally").
            let session = push || place(i)
                && var.is_some_and(|n| {
                    let n = n.as_str();
                    if n.starts_with("BASH") || SHELL_VARS.split_whitespace().any(|v| v == n) {
                        return false;
                    }
                    let rest = command
                        .replace(&format!("${{{n}}}"), "")
                        .replace(&format!("${n}"), "");
                    !rest.contains(n)
                });
            // Inner quotes and escapes go (`git 'fetch'`, `f"e"tch`); a glob may name one, and a
            // git-core program (`git-fetch`) runs git. A refspec (`a:b`) is no command word.
            let w = w.to_ascii_lowercase().replace(['\'', '"', '\\'], "");
            // Shell code has a space or an operator; a word that is itself a brace expansion
            // (`{a,b c}`) is judged by the brace rule.
            let shell = |c: char| c.is_whitespace() || ";&|(){}<>".contains(c);
            let code = w.contains(shell) && !w.starts_with('{');
            let built = w.contains(['$', '`']) && !session || code && BRACE.is_match(&w);
            built
                || w.split(cut).any(|t| {
                    let base = t.rsplit('/').next().unwrap_or(t);
                    let glob = t.contains(['*', '?', '[']) && !t.contains(':');
                    base.starts_with("git-")
                        || ["fetch", "pull", "update", "git-fetch", "git-pull"]
                            .iter()
                            .any(|n| base == *n || glob && names(t, n))
                })
        };
        if seg.iter().any(|w| !w.2 && gitw(&w.0))
            && seg
                .iter()
                .enumerate()
                .any(|(i, w)| (w.2 || w.0.contains('=')) && !is_prose(i, &w.0) && names_fetch(&w.0, i))
        {
            return Some(FETCH_PLAIN);
        }
        let assign = |w: &&(String, bool, bool)| w.0.contains('=') && !w.0.starts_with('-');
        let cmd = seg
            .iter()
            .find(|w| !assign(w) && !WRAPPERS.contains(&&*w.0));
        let cmd_glob = cmd.is_some_and(|(c, g, _)| *g && c != "[" && c != "[[");
        (cmd_glob && top || git && seg.iter().any(|w| w.1)).then_some(GLOB_WORD)
    })
}

/// Git config a plain fetch or pull follows (spec §4), `key` as `git config` prints it: a
/// refspec that moves the default branch, mirrors or writes tags from an untrusted remote, a
/// command it runs (`uploadpack`, `receivepack`, `core.sshCommand`, an `ext::`/`fd::` URL,
/// `protocol.allow` or `protocol.ext|fd.allow` not `never`), `tagOpt=--tags` on an untrusted
/// remote, or a URL rewrite to `ext::`, `fd::` or a non-network URL.
pub fn config_fetch_risk(key: &str, value: &str, default_branch: &str) -> bool {
    let k = cfg_key(key);
    let remote = k.strip_prefix("remote.").and_then(|r| r.rsplit_once('.'));
    let remote = remote.map_or("", |r| r.0);
    let trusted = matches!(remote, "origin" | "upstream");
    let v = value.to_ascii_lowercase();
    match k.rsplit('.').next().unwrap_or("") {
        "fetch" if k.starts_with("remote.") => {
            fetch_moves(&[remote, value], default_branch)
                || !trusted && norm_ref(value).contains("refs/tags/")
        }
        "uploadpack" | "receivepack" | "sshcommand" => true,
        "url" | "pushurl" => v.contains("ext::") || v.contains("fd::"),
        "tagopt" => !trusted && v.contains("--tags"),
        "insteadof" | "pushinsteadof" => {
            retargets(&k, value) || k.contains("ext::") || k.contains("fd::")
        }
        "allow" => {
            v != "never" && (k == "protocol.allow" || k.contains(".ext.") || k.contains(".fd."))
        }
        _ => false,
    }
}

/// A `remote.<r>.fetch` value that makes a later fetch move the default branch or write a tag.
fn fetch_cfg_moves(v: &str, default_branch: &str) -> bool {
    // Bash brace-expands an assigned value (`+feat:{x,main}`): fail closed.
    if v.contains('{') {
        return true;
    }
    let v = v.replace(['\'', '"'], "");
    let dst = norm_ref(v.rsplit(':').next().unwrap_or(&v));
    fetch_moves(&[".", &v], default_branch)
        || norm_ref(&v).contains("refs/tags/")
        || dst.starts_with("tags/")
}

/// A remote URL on a network host: `scheme://..` (not `file://`) or scp-like `host:path` (no
/// `/` before the `:`). Anything else (`evil2`, `../x`, `/tmp/x`) may be a local repository.
fn network_url(v: &str) -> bool {
    !local_word(v)
        && !v.contains(['{', '`'])
        && (v.contains("://")
            || v.split_once(':')
                .is_some_and(|(h, _)| !h.is_empty() && !h.contains('/')))
}

/// `git remote add|set-url origin|upstream <url>` with a URL that is not on a network host, and
/// `git remote rename <r> origin|upstream`: the trusted names must not point at a local repository.
fn remote_retargets(rest: &[&str]) -> bool {
    // Option values (`-t main`, `--tr main`, `-ft main`) are not skipped: a trusted name
    // anywhere, followed by a URL that is not on a network host, is refused.
    let words: Vec<String> = rest
        .iter()
        .map(|a| a.replace(['\'', '"'], ""))
        .filter(|a| !a.starts_with('-'))
        .collect();
    let trusted = |w: &String| matches!(w.as_str(), "origin" | "upstream");
    match words.first().map(String::as_str) {
        // A push URL does not change where a fetch reads.
        Some("set-url") if rest.iter().any(|a| a.len() >= 4 && "--push".starts_with(a)) => false,
        Some("add" | "set-url") => words
            .iter()
            .position(trusted)
            .is_some_and(|p| words[p + 1..].iter().any(|w| !network_url(w))),
        // `rename origin upstream` keeps a trusted URL; `rename evil upstream` does not.
        Some("rename") => {
            (words.last().is_some_and(trusted) && !words.get(1).is_some_and(trusted))
                || words.iter().any(|w| w.contains('$'))
        }
        _ => false,
    }
}

/// The value of `git remote --mirror` in any prefix (`--mi`), unquoted; `Some("")` when bare.
fn mirror(rest: &[&str]) -> Option<String> {
    rest.iter().find_map(|a| {
        let a = a.replace(['\'', '"'], "");
        let (k, v) = a.split_once('=').unwrap_or((&a, ""));
        (k.len() >= 4 && "--mirror".starts_with(k)).then(|| v.to_string())
    })
}

/// `--tags`/`--prune-tags` in any unique prefix, or `-t`/`-P` in a short cluster: every tag.
fn fetch_tags(rest: &[&str]) -> bool {
    rest.iter().any(|a| {
        let a = a.replace(['\'', '"'], "");
        (a.len() >= 5 && "--tags".starts_with(a.as_str()))
            || (a.len() >= 9 && "--prune-tags".starts_with(a.as_str()))
            || short_flags(&a).is_some_and(|f| f.contains(['t', 'P']))
    })
}

/// Config that points `origin`/`upstream` at a non-network URL (`remote.origin.url=evil2`) or
/// rewrites URLs to one (`url./tmp/x.insteadOf`); `k` as `cfg_key` gives it.
fn retargets(k: &str, v: &str) -> bool {
    let v = v.replace(['\'', '"'], "");
    let base = k.strip_prefix("url.").and_then(|b| {
        b.strip_suffix(".insteadof")
            .or_else(|| b.strip_suffix(".pushinsteadof"))
    });
    base.is_some_and(|b| !network_url(b))
        || (matches!(k, "remote.origin.url" | "remote.upstream.url") && !network_url(&v))
}

/// `git fetch <remote> <src>:<dst>` moves local refs like `update-ref`: refused for `--stdin`, a
/// backslash, a refspec in a variable, a glob destination that can match the default branch, or
/// a default-branch destination unless `origin`/`upstream` sends its own default branch.
fn fetch_moves(rest: &[&str], default_branch: &str) -> bool {
    if rest
        .iter()
        .any(|a| a.contains('\\') || a.starts_with("--stdin"))
    {
        return true;
    }
    let (words, maps) = fetch_words(rest);
    // A path anywhere (an option the guard misreads may hide the remote), or a `/` before the
    // first `:` of the remote word, makes the fetch local.
    let local = words.iter().any(|w| local_word(w))
        || words
            .first()
            .is_some_and(|w| w.split(':').next().unwrap_or(w).contains('/'));
    let trusted = !local && fetch_trusted(rest);
    words.iter().skip(1).chain(&maps).any(|r| {
        let spec = r.trim_start_matches('+');
        let Some((src, dst)) = spec.rsplit_once(':') else {
            return r.contains(['$', '`']);
        };
        // A glob destination that can match the default branch (`refs/heads/*`, `refs/*`, `*`).
        let dst = &norm_ref(dst);
        let glob = dst.split_once('*').is_some_and(|(pre, post)| {
            let full = format!("refs/heads/{default_branch}");
            post.contains('*')
                || [full.as_str(), &full[5..], default_branch].iter().any(|t| {
                    t.len() >= pre.len() + post.len() && t.starts_with(pre) && t.ends_with(post)
                })
        });
        r.contains(['$', '`'])
            || glob
            // `refs/heads/MAIN` is `main` on a case-insensitive file system.
            || (names_default(&dst.to_ascii_lowercase(), &default_branch.to_ascii_lowercase())
                && (!trusted || !names_default(src, default_branch)))
    })
}

/// Whether `git update-ref`, `symbolic-ref`, `branch -f|-M|-C` (or `--force` with `--copy` or
/// `--move`), `checkout -B` or `switch -C` writes the default branch (`update-ref`: any word
/// naming it, as quoted messages split); `branch -f`, `-B` and `-C` may move it to
/// `origin/<default>`, `upstream/<default>` or `refs/remotes/<r>/<default>`.
fn moves_default(sub: &str, rest: &[&str], default_branch: &str) -> bool {
    let words: Vec<String> = rest.iter().map(|a| a.replace(['\'', '"'], "")).collect();
    let short = |a: &str, fl: &[char]| !a.starts_with("--") && a.starts_with('-') && a.contains(fl);
    let named = |w: Option<&String>| w.is_some_and(|w| names_default(w, default_branch));
    let plain = words.iter().filter(|w| !w.starts_with('-'));
    // git takes a long option by a unique prefix: `--forc`, `--cop`, `--mo`.
    let long = |l: &str, min: usize| {
        let key = |w: &String| w.split('=').next().unwrap_or(w).to_string();
        words
            .iter()
            .any(|w| key(w).len() >= min && l.starts_with(&key(w)))
    };
    // A start point that is the default branch's remote-tracking ref only catches it up.
    let tracking = |w: Option<&String>| {
        w.is_some_and(|w| {
            let remote = w
                .strip_prefix("refs/remotes/")
                .and_then(|t| t.split_once('/'));
            remote.is_some_and(|(_, b)| b == default_branch)
                || w.strip_prefix("origin/")
                    .or_else(|| w.strip_prefix("upstream/"))
                    == Some(default_branch)
        })
    };
    match sub {
        "update-ref" => words.iter().any(|w| names_default(w, default_branch)),
        // Its name: the first plain word that is not the `-m` message.
        "symbolic-ref" => named(words.iter().enumerate().find_map(|(i, w)| {
            (!w.starts_with('-') && (i == 0 || words[i - 1] != "-m")).then_some(w)
        })),
        "branch" => {
            let f = long("--force", 6) || words.iter().any(|w| short(w, &['f']));
            let mc = words.iter().any(|w| short(w, &['M', 'C']))
                || (f
                    && (long("--copy", 5)
                        || long("--move", 4)
                        || words.iter().any(|w| short(w, &['c', 'm']))));
            let names: Vec<&String> = plain.collect();
            (f && named(names.first().copied()) && !tracking(names.get(1).copied()))
                || (mc && named(names.last().copied()))
        }
        // `-B <name> [<start>]`, `-Bmain`, `--force-create[=]<name>`.
        "checkout" | "switch" => words.iter().enumerate().any(|(i, w)| {
            let letter = if sub == "checkout" { 'B' } else { 'C' };
            let key = w.split('=').next().unwrap_or(w);
            let stuck = if key.len() > 7 && "--force-create".starts_with(key) {
                w.split_once('=').map(|(_, v)| v)
            } else if short(w, &[letter]) {
                w.split_once(letter).map(|(_, v)| v)
            } else {
                return false;
            };
            let (value, start) = match stuck.filter(|v| !v.is_empty()) {
                Some(v) => (Some(v.to_string()), words.get(i + 1)),
                None => (words.get(i + 1).cloned(), words.get(i + 2)),
            };
            named(value.as_ref()) && !tracking(start)
        }),
        _ => false,
    }
}

/// `v1.2.3`, `1.2` ...: a ref that looks like a version tag.
fn version_like(r: &str) -> bool {
    let t = r.strip_prefix('v').unwrap_or(r);
    // `v1`: whether a local tag of that name exists cannot be read here (core is pure), so a
    // bare `v<digits>` counts too.
    if r.starts_with('v') && !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        return true;
    }
    t.split_once('.').is_some_and(|(a, b)| {
        !a.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit())
            && b.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Whether `text` ends inside a single- or double-quoted string (`\` escapes outside `'`).
fn in_quotes(text: &str) -> bool {
    let (mut q, mut esc) = (None, false);
    for c in text.chars() {
        match (q, c) {
            _ if std::mem::take(&mut esc) => {}
            (Some('\''), '\'') => q = None,
            (Some('\''), _) => {}
            (_, '\\') => esc = true,
            (Some('"'), '"') => q = None,
            (None, '\'' | '"') => q = Some(c),
            _ => {}
        }
    }
    q.is_some()
}

/// The words of a `git push` without redirections (`> f`, `2>/dev/null`, the `2>` that
/// `2>&1` leaves once `&` splits it, `< f`) and the `)` that ends a group.
fn push_words(args: &[String]) -> Vec<&str> {
    let (mut out, mut skip) = (Vec::new(), false);
    for a in args {
        let a = a.trim_end_matches(')');
        if std::mem::take(&mut skip) {
            continue;
        }
        let op = a.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
        // A redirection glued to a word (`main>/dev/null`, `main<x`) ends it.
        if let Some(i) = op.find(['>', '<']).filter(|&i| i > 0) {
            let at = a.len() - op.len() + i;
            out.push(&a[..at]);
            skip = a[at..].trim_start_matches(['>', '<', '&']).is_empty();
        } else if let Some(target) = [">>", ">&", ">", "<"]
            .iter()
            .find_map(|o| op.strip_prefix(o))
        {
            skip = target.is_empty();
        } else if !a.is_empty() {
            out.push(a);
        }
    }
    out
}

/// `git push` flags whose value is the next word.
const PUSH_VALUE_FLAGS: [&str; 5] = ["-o", "--push-option", "--receive-pack", "--exec", "--repo"];

fn git_rule(args: &[&str], target: &Target) -> Option<&'static str> {
    let default_branch = target.default_branch;
    // git's global options: `-C dir`, `--git-dir`, `--work-tree` move the target; `-c k=v`,
    // `--no-pager` and the like do not. A target that cannot be read literally is unknown.
    let (mut i, mut opts, mut known) = (0, Vec::new(), true);
    // `-c remote.<r>.push=..` and the like; `push.default` when set.
    let (mut push_cfg, mut push_default) = (false, None);
    // `-c include.path=f` pulls in a config file a fetch or pull would follow.
    let mut include_cfg = false;
    while i < args.len() && args[i].starts_with('-') {
        let (a, next) = (args[i], args.get(i + 1).copied());
        let config = match a {
            "-c" | "--config-env" => next,
            _ => a.strip_prefix("--config-env="),
        };
        if config.is_some_and(alias_key) {
            return Some(GIT_ALIAS);
        }
        if let Some((k, v)) = config.map(|c| c.split_once('=').unwrap_or((c, ""))) {
            push_cfg |= push_key(k);
            // Bash drops a backslash before git reads the key (`remote.origin.ur\\l`).
            let fk = cfg_key(k).replace('\\', "");
            include_cfg |= fk == "include.path" || fk.starts_with("includeif.");
            if fk.starts_with("remote.")
                && fk.ends_with(".fetch")
                && (a.starts_with("--config-env") || fetch_cfg_moves(v, default_branch))
                || retargets(&fk, if a.starts_with("--config-env") { "" } else { v })
            {
                return Some(FETCH_MOVES);
            }
            if cfg_key(k) == "push.default" {
                // `--config-env` names a variable, not the value.
                let env = a.starts_with("--config-env");
                push_default = Some(if env { "" } else { v });
            }
        }
        let (flag, value, step) = match a.split_once('=') {
            Some((k @ ("--git-dir" | "--work-tree"), v)) => (Some(format!("{k}=")), Some(v), 1),
            _ => match a {
                "-C" => (Some("-C".to_string()), next, 2),
                "--git-dir" | "--work-tree" => (Some(format!("{a}=")), next, 2),
                "-c" | "--config-env" | "--namespace" | "--attr-source" | "--super-prefix" => {
                    (None, None, 2)
                }
                _ => (None, None, 1),
            },
        };
        if let Some(f) = flag {
            match value.and_then(literal_path) {
                Some(v) if f == "-C" => opts.extend([f, v]),
                Some(v) => opts.push(format!("{f}{v}")),
                None => known = false,
            }
        }
        i += step;
    }
    let sub = args.get(i)?.replace(['\'', '"', '\\'], "");
    let (sub, rest) = (sub.as_str(), &args[(i + 1).min(args.len())..]);
    if include_cfg && matches!(sub, "fetch" | "pull") {
        return Some(FETCH_MOVES);
    }
    // `git $g`, `git "${G}"`: the subcommand could be `merge`, `push` or `tag`: fail closed.
    if sub.contains(['$', '`']) {
        return Some(BUILT_SUB);
    }
    let at: Option<Vec<String>> = target
        .dir
        .filter(|_| known)
        .map(|d| d.iter().cloned().chain(opts).collect());
    let on_default = match &at {
        None => true,
        Some(all) => {
            match (target.branch_at)(all) {
                Some(b) => b == default_branch,
                // A moved target git cannot read (a failed `cd`, a missing `-C` dir) may
                // leave git where the shell started: fail closed.
                None => !all.is_empty(),
            }
        }
    };
    match sub {
        "push" | "send-pack" => {
            if push_cfg {
                return Some(PUSH_CONFIG);
            }
            // Quotes do not hide a target; one the shell expands cannot be read: fail closed.
            if rest.iter().any(|a| a.contains(['$', '`', '\\'])) {
                return Some(UNREAD_PUSH);
            }
            let unquoted: Vec<String> = rest.iter().map(|a| a.replace(['\'', '"'], "")).collect();
            let rest = push_words(&unquoted);
            // git takes a long option by any unique prefix (`--mir` is `--mirror`).
            const BULK: [&str; 9] = [
                "--force",
                "--force-with-lease",
                "--force-if-includes",
                "--tags",
                "--follow-tags",
                "--mirror",
                "--delete",
                "--all",
                "--stdin",
            ];
            if rest.iter().any(|a| {
                let long = a.split('=').next().unwrap_or(a);
                long.len() > 2 && long.starts_with("--") && BULK.iter().any(|b| b.starts_with(long))
            }) {
                return Some("force, tag, delete or bulk push");
            }
            if rest.iter().any(|a| {
                matches!(
                    *a,
                    "-f" | "--force" | "--tags" | "--follow-tags" | "--mirror" | "--delete" | "-d"
                ) || a.starts_with("--force-with-lease")
                    || a.starts_with("--force-if-includes")
                    // `-fu`, `-uf`, `-ud`: a short-flag cluster holding `f` or `d`.
                    || (a.starts_with('-') && !a.starts_with("--") && a.contains(['f', 'd']))
            }) {
                return Some("force, tag or delete push");
            }
            // Every branch, or a glob refspec: it can reach the default branch or tags.
            if rest
                .iter()
                .any(|a| matches!(*a, "--all" | "--branches") || a.contains('*'))
            {
                return Some("push of every branch or a glob refspec");
            }
            // The remote, then refspecs; the value of a flag that takes one is neither.
            let mut words = Vec::new();
            let mut it = rest.iter().copied();
            while let Some(a) = it.next() {
                if PUSH_VALUE_FLAGS.contains(&a) {
                    it.next();
                } else if !a.starts_with('-') {
                    words.push(a);
                }
            }
            let refs: Vec<&str> = words.into_iter().skip(1).collect();
            // `git push origin tag rel` and `tags/rel` push the tag `rel`.
            if refs.first() == Some(&"tag")
                || refs.iter().any(|r| {
                    r.split(':')
                        .any(|x| x.trim_start_matches('+').starts_with("tags/"))
                })
            {
                return Some("force, tag or delete push");
            }
            // `send-pack` without refs pushes every matching branch; `--stdin` reads them.
            if sub == "send-pack" && (refs.is_empty() || rest.contains(&"--stdin")) {
                return Some(UNREAD_PUSH);
            }
            if refs.is_empty() && push_default.is_some_and(|d| !head_only(d)) {
                return Some(PUSH_CONFIG);
            }
            if refs
                .iter()
                .any(|r| r.starts_with('+') || r.starts_with(':') || r.contains("refs/tags/"))
                || refs.iter().any(|r| r.split(':').any(version_like))
            {
                return Some("force, tag or delete push");
            }
            let to_default = refs.iter().any(|r| {
                let dst = r.rsplit(':').next().unwrap_or(r);
                names_default(dst, default_branch) || (matches!(dst, "HEAD" | "@") && on_default)
            });
            if to_default || (refs.is_empty() && on_default) {
                return Some("push to the default branch");
            }
            // A refspec without `:dst` (or none) follows `remote.<r>.push` and `push.default`,
            // which an earlier command may change: only alone does the hook read that config.
            let explicit = !refs.is_empty()
                && refs.iter().all(|r| {
                    r.split_once(':')
                        .is_some_and(|(s, d)| !s.is_empty() && !d.is_empty())
                });
            // `pushes` lists it either way; `tool_rules` refuses it first.
            if let Some(sink) = target.sink {
                let srcs = refs.iter().map(|r| r.split(':').next().unwrap_or(r));
                sink(Push {
                    dir: at,
                    srcs: srcs.map(str::to_string).collect(),
                });
            }
            (sub == "push" && !explicit && !target.alone).then_some(PUSH_ALONE)
        }
        "update-ref" | "symbolic-ref" | "branch" | "checkout" | "switch"
            if moves_default(sub, rest, default_branch) =>
        {
            Some("moving the default branch")
        }
        // `--stdin`, a variable, a backtick or an escape in a ref hides it: fail closed. `-m`
        // quoting is not parsed (`-m "a b"c $r`, `-dm x $r` hide the ref from any parser), so ANY
        // non-flag argument with `$` or a backtick denies, a `"$sha"` value included.
        // `symbolic-ref` writes a ref the same way (`refs/tags/x` or `tags/x` as its name).
        "update-ref" | "symbolic-ref" => (rest.iter().any(|a| {
            let q = norm_ref(&a.replace(['\'', '"'], ""));
            a.contains(['`', '\\'])
                || a.starts_with("--stdin")
                || q.contains("refs/tags/")
                || (sub == "symbolic-ref" && q.starts_with("tags/"))
                || (!q.starts_with('-') && q.contains('$'))
        }))
        .then_some("tag"),
        // `git fetch . HEAD:refs/tags/v1` makes a tag too; mirroring the remote's own tags
        // (`'refs/tags/*:refs/tags/*'`, as `--tags`) does not.
        // `git pull <remote> <src>:<dst>` hands its refspecs to fetch.
        "fetch" | "pull"
            if (fetch_tags(rest) && !fetch_trusted(rest))
                // `tag <name>` is git's shorthand for `refs/tags/<name>:refs/tags/<name>`.
                || fetch_words(rest).0.iter().skip(1).any(|w| w.trim_start_matches('+') == "tag")
                || rest.iter().any(|a| {
                let a = norm_ref(&a.replace(['\'', '"', '\\'], ""));
                a.contains("refs/tags/")
                    && (a.trim_start_matches('+') != "refs/tags/*:refs/tags/*" || !fetch_trusted(rest))
                    // `<src>:$T` writes a ref the hook cannot read; `<src>:tags/x` is a DWIM tag.
                    || a.rsplit_once(':').is_some_and(|(_, d)| d.contains(['$', '`']) || d.starts_with("tags/"))
            }) =>
        {
            Some("tag")
        }
        "fetch" | "pull" if fetch_moves(rest, default_branch) => Some(FETCH_MOVES),
        "tag" => {
            let lists = rest.is_empty()
                || rest
                    .iter()
                    .any(|a| matches!(*a, "-l" | "--list" | "-v" | "--verify"))
                || rest.iter().all(|a| {
                    a.starts_with("-n")
                        || a.starts_with("--contains")
                        || a.starts_with("--points-at")
                });
            (!lists).then_some("tag")
        }
        "subtree" if rest.first() == Some(&"push") => subtree_push(&rest[1..], default_branch),
        "config" => {
            let reads = matches!(rest.first(), Some(&("get" | "list")))
                || rest.iter().any(|a| {
                    matches!(*a, "--get" | "--get-all" | "--get-regexp" | "-l" | "--list")
                });
            if reads {
                return None;
            }
            if rest.iter().any(|a| alias_key(a)) {
                return Some(GIT_ALIAS);
            }
            // Writing push targets into config; unsetting them is fine.
            let unsets = rest
                .iter()
                .any(|a| matches!(*a, "--unset" | "--unset-all" | "unset"));
            // A key with a value after it is a write; the key alone reads it.
            let value = |i: usize| rest.get(i + 1).filter(|v| !v.starts_with('-'));
            let writes = rest.iter().enumerate().any(|(i, a)| {
                (push_key(a) && value(i).is_some())
                    || (cfg_key(a) == "push.default" && value(i).is_some_and(|v| !head_only(v)))
            });
            // `remote.<r>.fetch` (also `--add`, `--replace-all`, `set`) that a later fetch follows.
            let fetch = rest.iter().enumerate().any(|(i, a)| {
                let k = cfg_key(a).replace('\\', "");
                k.starts_with("remote.")
                    && k.ends_with(".fetch")
                    && value(i).is_some_and(|v| fetch_cfg_moves(v, default_branch))
                    || value(i).is_some_and(|v| retargets(&k, v))
            });
            // `--rename-section remote.evil remote.upstream` renames a remote like `git remote rename`.
            let plain: Vec<String> = rest
                .iter()
                .filter(|a| !a.starts_with('-') || a.starts_with("--rename-section"))
                .map(|a| cfg_key(a))
                .collect();
            let renames = plain
                .first()
                .is_some_and(|w| matches!(w.as_str(), "rename-section" | "--rename-section"));
            let trusted = |w: Option<&String>| {
                w.is_some_and(|w| matches!(w.as_str(), "remote.origin" | "remote.upstream"))
            };
            if renames && trusted(plain.last()) && !trusted(plain.get(1).filter(|_| plain.len() > 2))
            {
                return Some(FETCH_MOVES);
            }
            if !unsets && fetch {
                return Some(FETCH_MOVES);
            }
            (!unsets && writes).then_some(PUSH_CONFIG)
        }
        // `--mirror=push` (or a bare `--mirror`) makes every push a mirror push.
        // `--mirror=fetch` writes `+refs/*:refs/*`, which a later bare fetch follows.
        // Any unique prefix of `--mirror`, quoted or not (`--mi=fetch`, `'--mirror=fetch'`).
        "remote" if mirror(rest).is_some_and(|v| !v.is_empty() && "fetch".starts_with(&v)) => {
            Some(FETCH_MOVES)
        }
        "remote" if remote_retargets(rest) => Some(FETCH_MOVES),
        "remote" => mirror(rest).is_some().then_some(PUSH_CONFIG),
        "merge" if ff_only(rest, default_branch) => None,
        "merge" | "rebase" | "cherry-pick" | "reset" | "commit" if on_default => {
            Some("changing the default branch")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Timestamp;
    use serde_json::json;

    fn bash(cmd: &str, branch: &str) -> Option<String> {
        tool_rules(
            "Bash",
            &json!({ "command": cmd }),
            &|_| Some(branch.into()),
            "main",
        )
    }

    #[test]
    fn destructive_lock_denies_the_wipe_commands() {
        for cmd in [
            "rm -rf /",
            "rm -rf /*",
            "rm -fr ~",
            "rm -rfv ~/",
            "rm -r -f ~/*",
            "rm --recursive --force /",
            "rm -rf $HOME",
            "rm -rf \"$HOME\"",
            "rm -rf ${HOME}",
            "rm -rf /.",
            "rm -rf --no-preserve-root x",
            "sudo rm -rf /",
            "cd x && rm -rf ~",
            "echo hi; rm -rf /",
            "bash -c 'rm -rf /'",
            "sudo bash -c \"cd / && rm -rf /\"",
            "echo x | xargs rm -rf /",
            "env X=1 rm -rf ~",
            "mkfs.ext4 /dev/sda1",
            "mkfs /dev/sda1",
            "mke2fs /dev/sda1",
            "dd if=x of=/dev/sda bs=1M",
            "shred -n3 /dev/sda",
            "wipefs -a /dev/nvme0n1",
            "cat x > /dev/sda",
            "echo x >/dev/nvme0n1p1",
            "cat x >> /dev/disk2",
            "terraform destroy",
            "terraform apply -destroy",
            "tofu destroy -auto-approve",
            "psql -c 'DROP DATABASE prod'",
            "psql -c \"drop   schema public cascade\"",
            ":(){ :|:& };:",
            "chmod -R 777 /",
            "chown -R me ~",
            "timeout 5 rm -rf /",
            "nice -n 5 rm -rf ~",
            "doas rm -rf /",
            "sudo -u root rm -rf /",
            "command rm -rf /",
            "\\rm -rf /",
            "/bin/rm -rf /",
            "X=1 rm -rf /",
            "(rm -rf /)",
            "echo $(rm -rf /)",
            "echo `rm -rf ~`",
            "git commit -m \"$(rm -rf /)\"",
            "eval 'rm -rf /'",
            "sh -c \"bash -c 'rm -rf ~'\"",
            "bash <<'EOF'\nrm -rf /\nEOF",
            "cat > f <<EOF\n$(rm -rf /)\nEOF",
            "terraform -chdir=infra destroy",
            "echo 'DROP DATABASE x' | psql",
            "mysql -e \"DROP DATABASE x\"",
            "docker exec db psql -c 'drop database x'",
            "psql <<'EOF'\nDROP DATABASE prod;\nEOF",
            "echo x &>/dev/sda",
            "echo hi; :(){ :|:& };:",
            "psql -c \"DROP DATABASE x\"",
            "rm -rf ~/.",
            "rm -rf ${HOME:?}",
            "rm -rf ${HOME:?}/",
            "rm -rf \"${HOME}\"/",
            "rm -rf $HOME/.",
            "rm -rf \"$HOME/\"",
            "sudo -u postgres psql -c 'DROP DATABASE x'",
            "PGPASSWORD=x /usr/bin/psql -h db -c 'drop schema s cascade'",
            "kubectl exec -it db -- psql -c \"DROP DATABASE x\"",
            "clickhouse-client -q 'DROP DATABASE x'",
            "docker compose exec db mysql -e 'drop database x'",
            "echo 'DROP DATABASE x' | sudo -u postgres psql",
            "echo 'DROP DATABASE x' > d.sql && psql -f d.sql",
            "rm -rf ~/*",
            "rm -rf ${HOME}/.",
            "chmod -R 000 ~/.",
            "terraform apply --destroy",
            ":(){:|:&};:",
            ":(){ :|: & };:",
        ] {
            let why = bash(cmd, "feat").unwrap_or_else(|| panic!("allowed: {cmd}"));
            assert!(why.starts_with("destructive-lock"), "{cmd}: {why}");
        }
    }

    #[test]
    fn destructive_lock_allows_the_everyday_commands() {
        for cmd in [
            "rm -rf ./target",
            "rm -rf target /tmp/x",
            "rm file",
            "rm -rf /tmp/x",
            "rm -rf ~/proj/target",
            "dd if=x of=y.img",
            "dd if=/dev/zero of=/dev/null",
            "cat x > /dev/null",
            "terraform plan",
            "chmod -R 755 ./dist",
            "cargo build 2>&1 | tail",
            "mkdir -p out",
            "grep mkfs docs.md",
            "git commit -m \"fix rm -rf / handling\"",
            "git commit -m \"guard: a; rm -rf / is refused\"",
            "git commit -m \"the lock (rm -rf ~) case\"",
            "git commit -m \"DROP DATABASE and drop schema are refused\"",
            "cargo nextest run rm",
            "cargo test destroy mkfs",
            "grep -rn \"dd of=/dev\" src",
            "gh pr create --title t --body \"refuses terraform destroy, mkfs and DROP DATABASE\"",
            "cat > f <<'EOF'\nrm -rf /\nDROP DATABASE x\nmkfs /dev/sda\nEOF",
            "cat > f <<EOF\nrm -rf ~\nterraform destroy\nEOF",
            "bosun lessons add \"never rm -rf ~\"",
            "git log --grep 'rm -rf /'",
            "psql -c 'select 1'",
            "git commit -m ':(){ :|:& };:'",
            "echo \"DROP DATABASE x\"",
            "dd if=x of=/dev/stdout",
            "dd if=x of=/dev/stderr",
            "dd if=x of=/dev/fd/3",
            "dd if=x of=/dev/tty",
            "grep -rn \"DROP DATABASE\" src | grep psql",
            "rg -n psql docs && echo 'drop schema is refused'",
            "git commit -m \"$(cat <<'EOF'\nfix: refuse psql -c 'DROP DATABASE x', rm -rf / and :(){ :|:& };:\n\nCo-Authored-By: C <c@x>\nEOF\n)\"",
            "gh pr create --title t --body \"$(cat <<'EOF'\n- (rm -rf ~) is refused\n- dd of=/dev/sda\nEOF\n)\"",
            "cat > notes.md <<'EOF'\npsql -c 'DROP DATABASE x'\nEOF",
            "cargo nextest run --locked 2>&1 | tail -5",
            "grep -n 'rm -rf /' src/core/guard.rs",
            "ls psql && git diff -- 'drop database'",
            "git log -S 'DROP SCHEMA' -- src/psql",
            "echo \":(){ :|:& };:\" >> notes.md",
            "gh pr create --title \"psql guard\" --body-file - <<'EOF'\nrm -rf ~\nEOF",
        ] {
            assert_eq!(bash(cmd, "feat"), None, "{cmd}");
        }
    }

    #[test]
    fn a_registered_child_may_only_create_one_task_issue() {
        let task = r#"<!-- bosun:task {"item":"M1","task":"T9","epic":5,"touch":["src/a.rs"]} -->"#;
        let sh = |cmd: &str| child_task_create("Bash", &json!({ "command": cmd }));
        let create = |body: &str| format!("gh issue create --title t --body '{body}'");
        assert_eq!(sh(&create(task)).unwrap().epic, 5);
        let mcp = |m: Value| child_task_create("mcp__github__issue_write", &m);
        let ok = mcp(json!({"method":"create","owner":"o","repo":"r","body":task})).unwrap();
        assert_eq!(ok.repo, Some(("o".into(), "r".into())));
        // Not a creation, or not this repo, or not one plain command.
        for cmd in [
            format!("gh issue edit 5 --body '{task}'"),
            format!("gh issue comment 5 --body '{task}'"),
            format!("gh api -X PATCH repos/o/r/issues/5 -f body='{task}'"),
            format!("gh issue create -R evil/other --title t --body '{task}'"),
            format!("gh issue create --repo=evil/other --title t --body '{task}'"),
            format!("bash -c \"{}\"", create(task)),
            format!("{} && {}", create(task), create(task)),
        ] {
            assert!(sh(&cmd).is_err(), "{cmd}");
        }
        assert!(
            mcp(json!({"method":"update","issue_number":5,"owner":"o","repo":"r","body":task}))
                .is_err()
        );
        assert!(
            mcp(json!({"method":"create","issue_number":5,"owner":"o","repo":"r","body":task}))
                .is_err()
        );
        assert!(
            child_task_create("mcp__github__add_issue_comment", &json!({"body": task})).is_err()
        );
        // Prose that names the marker is no decoy for the real one bosun reads, and a variable
        // prefix cannot redirect `gh`.
        let decoy =
            r#"see bosun:task {"epic":5} --> then <!-- bosun:task {"epic":5,"allow":["skip"]} -->"#;
        assert!(sh(&create(decoy)).is_err());
        let decoy2 = r#"x bosun:task {"epic":5,"allow":[]} --> <!-- bosun:task {"epic":5,"capabilities":[]} -->"#;
        assert!(sh(&create(decoy2)).is_err());
        // `$` and backticks can hide a marker; `./gh` and a `cd` prefix change what runs.
        for cmd in [
            create(task).replace("<!--", "$'\\x3c'!--"),
            create(task).replace("src/a.rs", "$HOME/a.rs"),
            create(task).replace("--title t", "--title `id`"),
            create(task).replacen("gh issue", "./gh issue", 1),
            create(task).replacen("gh issue", "/tmp/gh issue", 1),
            format!("cd /tmp/other && {}", create(task)),
            format!("{}; echo hi", create(task)),
        ] {
            assert!(sh(&cmd).is_err(), "{cmd}");
        }
        // Kinds with a lighter gate path are not for a child.
        for kind in ["mainfix", "process-fix", "plan", "nonsense"] {
            let meta = format!(r#"<!-- bosun:task {{"epic":5,"kind":"{kind}"}} -->"#);
            assert!(sh(&create(&meta)).is_err(), "{kind}");
        }
        for kind in ["feature", "bookkeeping", "flake"] {
            let meta = format!(r#"<!-- bosun:task {{"epic":5,"kind":"{kind}"}} -->"#);
            assert!(sh(&create(&meta)).is_ok(), "{kind}");
        }
        for cmd in [
            create(task).replacen("gh issue", "GH_REPO=evil/x gh issue", 1),
            create(task).replacen("gh issue", "GH_HOST=evil.example gh issue", 1),
            create(task).replacen("gh issue", "command gh issue", 1),
        ] {
            assert!(sh(&cmd).is_err(), "{cmd}");
        }
        // The marker: no allow or capabilities, an epic, one marker, nothing else, valid JSON.
        for body in [
            r#"<!-- bosun:task {"epic":5,"allow":["skip"]} -->"#,
            r#"<!-- bosun:task {"epic":5,"capabilities":[]} -->"#,
            r#"<!-- bosun:task {"item":"M1"} -->"#,
            r#"<!-- bosun:task {"epic":5} --> <!-- bosun:task {"epic":6} -->"#,
            r#"<!-- bosun:task {"epic":5} --> <!-- bosun:epic 3 -->"#,
            "<!-- bosun:task not json -->",
            "<!-- bosun:task {\"epic\":5}",
        ] {
            assert!(sh(&create(body)).is_err(), "{body}");
        }
    }

    #[test]
    fn prereqs_lock_judges_only_issue_body_writes() {
        let body = "<!-- bosun:prereq H1 sum=ab state=open -->";
        let deny = |tool: &str, input: Value| prereqs_lock(tool, &input).is_some();
        let sh = |cmd: &str| deny("Bash", json!({ "command": cmd }));
        assert!(deny("mcp__github__issue_write", json!({ "body": body })));
        assert!(!deny(
            "mcp__github__issue_write",
            json!({ "body": "plain" })
        ));
        assert!(!deny(
            "mcp__github__create_pull_request",
            json!({ "body": body })
        ));
        assert!(!deny("mcp__github__issue_read", json!({ "body": body })));
        assert!(sh(&format!("gh issue edit 5 --body \"{body}\"")));
        assert!(sh(
            "gh api -X PATCH repos/o/r/issues/5 -f body='x <!-- bosun:prereq H1 --> y'"
        ));
        assert!(sh(&format!(
            "curl -X PATCH -d '{body}' https://api.github.com/x"
        )));
        assert!(sh("gh issue edit 5 --body '<!-- bosun:task {\"n\":1} -->'"));
        assert!(sh("gh issue comment 5 -b '<!-- bosun:epic 3 -->'"));
        assert!(deny(
            "mcp__github__add_issue_comment",
            json!({ "body": "<!-- bosun:epic 3 -->" })
        ));
        assert!(!sh("gh issue view 5"));
        assert!(!sh("gh issue list"));
        assert!(!sh("gh api repos/o/r/issues/5"));
        assert!(!sh("git commit -m \"fix bosun:prereq parsing\""));
        assert!(!sh("bosun prereqs plans/x.md --post"));
        // Documented best-effort gap (design F4): the body comes from a file, so no marker here.
        assert!(!sh("gh issue edit 5 --body-file f"));
        // Nested shells, attached short flags, curl --json, hub.
        assert!(sh(
            "bash -c 'gh issue edit 5 --body \"<!-- bosun:task {} -->\"'"
        ));
        assert!(sh("echo $(gh issue comment 5 -b '<!-- bosun:epic 3 -->')"));
        assert!(sh("gh issue comment 5 -b'<!-- bosun:task {\"n\":1} -->'"));
        assert!(sh("gh issue comment 5 -b\"<!-- bosun:task {} -->\""));
        assert!(sh(
            "curl --json '<!-- bosun:prereqs p.md -->' https://api.github.com/x"
        ));
        assert!(sh(
            "hub api -X PATCH repos/o/r/issues/5 -f body='<!-- bosun:task {} -->'"
        ));
        // Prose, `bosun:task-pr` and read-only calls are not refused.
        assert!(!sh(
            "gh issue comment 5 --body 'I fixed how bosun:task metadata is read'"
        ));
        assert!(!sh(
            "gh pr comment 5 -b 'Closes #5 <!-- bosun:task-pr 5 -->'"
        ));
        assert!(!sh("gh api -X GET search/issues -f q='<!-- bosun:task'"));
        assert!(!sh(
            "gh api --method GET search/issues -f q='<!-- bosun:task'"
        ));
        // gh and curl use the last method flag: any write method among them is a write.
        for cmd in [
            "gh api -X GET --method PATCH repos/o/r/issues/5 -f body='<!-- bosun:task {} -->'",
            "gh api -XGET -XPATCH repos/o/r/issues/5 -f body='<!-- bosun:task {} -->'",
            "gh api --method=GET -X PATCH repos/o/r/issues/5 -f body='<!-- bosun:task {} -->'",
            "curl -X GET -X PATCH -H 'x: <!-- bosun:task {} -->' https://api.github.com/x",
        ] {
            assert!(sh(cmd), "{cmd}");
        }
        assert!(!sh(
            "gh api -X GET --method GET search/issues -f q='<!-- bosun:task'"
        ));
        assert!(!sh(
            "gh api graphql -f query='{ search(query: \"<!-- bosun:task\") { issueCount } }'"
        ));
        assert!(sh(
            "gh api graphql -f query='mutation { x(body: \"<!-- bosun:task {} -->\") }'"
        ));
        assert!(!deny(
            "mcp__github__add_issue_comment",
            json!({ "body": "I fixed how bosun:task metadata is read" })
        ));
    }

    #[test]
    fn merge_lock_refuses_merges_tags_and_force() {
        for cmd in [
            "gh pr merge 12 --squash",
            "gh api -X PUT repos/o/r/pulls/12/merge",
            "gh release create v1.0.0",
            "git push --force origin feature",
            "git push -f",
            "git push --force-with-lease origin x",
            "git push origin +feature",
            "git push origin feature:main",
            "git push origin main",
            "git push --tags",
            "git push origin refs/tags/v1",
            "git -C repo tag v1.0.0",
            "git tag -a v1 -m x",
            "cargo test && git push -f origin x",
            "GIT_TRACE=1 git push --force",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        assert!(bash("git merge feature", "main").is_some());
        assert!(bash("git push", "main").is_some());
        assert!(bash("git push origin HEAD", "main").is_some());
        assert!(bash("git commit -m x", "main").is_some());
    }

    /// The hook's directory is on `main`; `/wt` (and anything below `-C /wt`) is a worktree on
    /// `feature`. Returns the options the rule asked about, and its verdict.
    fn in_worktree(cmd: &str) -> (Vec<Vec<String>>, Option<String>) {
        let asked = std::cell::RefCell::new(Vec::new());
        let branch_at = |opts: &[String]| {
            asked.borrow_mut().push(opts.to_vec());
            let wt = opts
                .iter()
                .any(|o| o == "/wt" || o.ends_with("=/wt/.git") || o.ends_with("=/wt"));
            Some(if wt { "feature" } else { "main" }.to_string())
        };
        let why = tool_rules("Bash", &json!({ "command": cmd }), &branch_at, "main");
        (asked.into_inner(), why)
    }

    #[test]
    fn merge_lock_reads_the_branch_where_git_acts() {
        for cmd in [
            "cd /wt && git commit -m x",
            "cd \"/wt\"; git add -A; git commit -m x",
            "git -C /wt commit -m x",
            "git --git-dir=/wt/.git --work-tree=/wt commit -m x",
            "git --git-dir /wt/.git commit -m x",
            "cd /wt && git push -u origin feature:feature",
        ] {
            let (asked, why) = in_worktree(cmd);
            assert_eq!(why, None, "{cmd}: asked {asked:?}");
        }
        let (asked, _) = in_worktree("cd /wt && git -C sub commit -m x");
        assert_eq!(asked, vec![vec!["-C", "/wt", "-C", "sub"]]);
        // The hook's own directory is on main.
        assert!(in_worktree("git commit -m x").1.is_some());
        assert!(in_worktree("git -C /other commit -m x").1.is_some());
        // A moved target git cannot read (a failed `cd`) leaves git in the start dir.
        let none = |_: &[String]| None;
        let cmd = json!({"command": "cd /missing; git commit -m x"});
        assert!(tool_rules("Bash", &cmd, &none, "main").is_some());
        let cmd = json!({"command": "git commit -m x"});
        assert_eq!(tool_rules("Bash", &cmd, &none, "main"), None);
    }

    #[test]
    fn merge_lock_fails_closed_on_a_target_it_cannot_read() {
        for cmd in [
            "cd $WT && git commit -m x",
            "cd ~/wt && git commit -m x",
            "cd && git commit -m x",
            "cd - && git commit -m x",
            "cd /wt | true; git commit -m x",
            "cd /wt || git commit -m x",
            "git -C \"$WT\" commit -m x",
            "git -C `pwd` commit -m x",
            "git --git-dir=$D commit -m x",
            "git -C",
        ] {
            let (asked, why) = in_worktree(cmd);
            if cmd != "git -C" {
                assert!(why.is_some(), "{cmd}");
                assert!(asked.is_empty(), "{cmd}: asked {asked:?}");
            }
        }
        // A `cd` the shell may skip, or runs in a subshell, does not move git.
        for cmd in [
            "false && cd /wt; git commit -m x",
            "true || cd /wt; git commit -m x",
            "echo | cd /wt; git commit -m x",
            "(\ncd /wt\n)\ngit commit -m x",
            "cd /wt && GIT_DIR=/main/.git git commit -m x",
            "export GIT_WORK_TREE=/main; cd /wt && git commit -m x",
            "cd /wt && git -c core.worktree=/main commit -m x",
        ] {
            assert!(in_worktree(cmd).1.is_some(), "{cmd}");
        }
        // An `&&` chain that reaches git keeps the move.
        assert_eq!(in_worktree("make && cd /wt && git commit -m x").1, None);
        // Reading commands stay allowed wherever they run.
        assert_eq!(in_worktree("cd $WT && git status").1, None);
        assert_eq!(in_worktree("git -C $X log").1, None);
    }

    #[test]
    fn merge_lock_sees_nested_commands_and_api_writes() {
        for cmd in [
            "bosun status $(gh pr merge 1)",
            "bosun status `gh pr merge 1`",
            "sh -c \"gh pr merge 1\"",
            "bash -c 'gh pr merge 1'",
            "bash -lc 'echo hi; gh pr merge 1'",
            "zsh -c \"bash -c 'gh pr merge 1'\"",
            "(gh pr merge 1)",
            "echo $(echo $(gh pr merge 1))",
            "eval \"gh pr merge 1\"",
            "{ gh pr merge 1; }",
            "sudo gh pr merge 1",
            "env FOO=1 git push --force",
            "gh api graphql -f query='mutation{mergePullRequest(input:{pullRequestId:\"x\"}){clientMutationId}}'",
            "gh api graphql -f query='mutation { enablePullRequestAutoMerge(input:{}) { clientMutationId } }'",
            "gh api graphql -f query='mutation{createRef(input:{}){clientMutationId}}'",
            "gh api graphql -f query='mutation{updateRef(input:{}){clientMutationId}}'",
            "curl -X PUT https://api.github.com/repos/o/r/pulls/1/merge",
            "curl -s -X POST -H 'Authorization: token x' https://api.github.com/repos/o/r/git/refs -d '{}'",
            "curl https://api.github.com/repos/o/r/releases -d '{\"tag_name\":\"v1\"}'",
            "curl https://api.github.com/graphql -d '{\"query\":\"mutation{mergePullRequest(input:{})}\"}'",
            "curl https://api.github.com/repos/o/r/pulls/1 -d '{\"auto_merge\":true}'",
            "gh api -X PUT repos/o/r/pulls/1/ccr/auto_merge -f merge_method=squash",
            "curl -X PUT https://api.github.com/repos/o/r/pulls/1/ccr/auto_merge -d '{\"merge_method\":\"squash\"}'",
            "wget --method=PUT https://api.github.com/repos/o/r/pulls/1/merge",
            "http PUT api.github.com/repos/o/r/pulls/1/merge",
            "https PUT api.github.com/repos/o/r/git/refs ref=refs/tags/v1",
            "git push origin v1.0.0",
            "git push origin 1.2.3",
            "git push origin HEAD:v2.0",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        // Past the depth limit the guard refuses rather than guessing.
        assert!(bash("((((((((((gh pr view 1))))))))))", "feature").is_some());
    }

    /// F9: a `gh api` read (no method but GET, no field or input flag) may name `auto_merge`.
    #[test]
    fn merge_lock_allows_gh_api_reads_that_name_auto_merge() {
        for cmd in [
            "gh api repos/o/r/pulls/12 --jq '.auto_merge'",
            "gh api repos/o/r/pulls/12 --jq '{state: .state, am: .auto_merge}'",
            "gh api -X GET repos/o/r/pulls/12 --jq .auto_merge",
            "gh api --method get repos/o/r/pulls/12/ccr/auto_merge",
            "gh api repos/o/r/pulls/12/merge",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// F9: a `gh api` write (a write method, or a field or input flag, which default to POST)
    /// on the merge or auto-merge endpoint stays refused.
    #[test]
    fn merge_lock_refuses_gh_api_merge_writes() {
        for cmd in [
            "gh api -X PUT repos/o/r/pulls/12/merge",
            "gh api -XPUT repos/o/r/pulls/12/ccr/auto_merge",
            "gh api --method=PUT repos/o/r/pulls/12/merge",
            "gh api -X put repos/o/r/pulls/12/ccr/auto_merge",
            "gh api -X GET -X PUT repos/o/r/pulls/12/merge",
            "gh api repos/o/r/pulls/12/merge -f merge_method=squash",
            "gh api repos/o/r/pulls/12/ccr/auto_merge -F merge_method=squash",
            "gh api repos/o/r/pulls/12/ccr/auto_merge -fmerge_method=squash",
            "gh api repos/o/r/pulls/12/ccr/auto_merge --field merge_method=squash",
            "gh api repos/o/r/pulls/12/ccr/auto_merge --raw-field=merge_method=squash",
            "gh api repos/o/r/pulls/12/merge --input body.json",
            "gh api -X GET repos/o/r/pulls/12/merge -f merge_method=squash",
            "gh api repos/o/r/pulls/12/ccr/auto_merge --jq '.a | .b' -X PUT",
            "gh api -iX PUT repos/o/r/pulls/12/merge",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
    }

    /// F9: real commands nest `$( )` and jq parens past three levels; each inner command is
    /// still judged.
    #[test]
    fn merge_lock_judges_deep_substitutions() {
        for cmd in [
            "echo $(echo $(echo $(echo $(echo hi))))",
            "bash -c \"echo $(gh pr list --json number --jq 'map(select(.number > 0))')\"",
            "cd /x && N=$(gh pr list --json number --jq \"[.[] | select(.title | \
             test(\\\"$(git log -1 --format=%s)\\\"))] | .[0].number\") && echo $N",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
        for cmd in [
            "echo $(gh pr merge 1 --squash)",
            "echo $(echo $(git push --force origin x))",
            "echo $(echo $(echo $(echo $(echo $(gh pr merge 1)))))",
            "x=$(bash -c \"echo $(gh api -X PUT repos/o/r/pulls/1/merge)\")",
            "echo \"$(echo `gh release create v1`)\"",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        let deep = format!(
            "echo {}gh pr view 1{}",
            "$(echo ".repeat(12),
            ")".repeat(12)
        );
        let why = bash(&deep, "feature").unwrap_or_default();
        assert!(why.contains("nested deeper than"), "{why}");
        assert!(!why.contains("decrypting"), "{why}");
    }

    /// F9: `git merge --ff-only REF` on the default branch moves only the local branch.
    #[test]
    fn merge_lock_allows_a_fast_forward_of_the_local_branch() {
        for cmd in [
            "git merge --ff-only origin/main",
            "git fetch origin && git merge --ff-only origin/main",
            "git merge --ff-only",
            "git merge --ff-only @{u}",
            "git merge --ff-only @{upstream}",
        ] {
            assert_eq!(bash(cmd, "main"), None, "{cmd}");
        }
        for cmd in [
            "git merge origin/main",
            "git merge --no-ff origin/feature",
            "git merge --ff-only --no-ff origin/feature",
            "git merge --ff-only origin/a origin/b",
            "git merge --ff-only -m x origin/feature",
            "git merge --ff-only origin/feature",
            "git merge --ff-only feature",
            "git merge --ff-only upstream/main",
            "git merge --ff-only origin/main~1",
            "git merge --ff-only FETCH_HEAD",
            "git merge --ff-only origin/main && git push origin main",
            "git merge --ff-only origin/main && git push --force",
            "git push --force origin feature",
            "gh pr merge 12 --squash",
        ] {
            assert!(bash(cmd, "main").is_some(), "{cmd}");
        }
    }

    #[test]
    fn normal_work_is_allowed() {
        for cmd in [
            "git push -u origin feature",
            "git push origin HEAD",
            "git merge origin/main",
            "git commit -m 'fix: x'",
            "git tag",
            "git tag -l 'v*'",
            "gh pr view 12",
            "gh api repos/o/r/pulls/12",
            "cargo nextest run",
            "bosun release 4 --to ready",
            "bosun status $(git rev-parse HEAD)",
            "bash -c 'cargo test'",
            "(cd web && npm test)",
            "gh api graphql -f query='query{viewer{login}}'",
            "curl -s https://api.github.com/repos/o/r/pulls/1",
            "curl https://example.com/merge",
            "git push origin release/1.2",
            "git push -u origin claude/fix-1",
            "git merge origin/main",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// Refused on `feature`, then allowed on `feature`.
    fn refused_then_allowed(refused: &[&str], allowed: &[&str]) {
        for cmd in refused {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        for cmd in allowed {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// Review of #351: quotes do not hide a push target, and a target the guard cannot read
    /// (`$VAR`, `$( )`, a backslash) is refused.
    #[test]
    fn merge_lock_unquotes_push_targets() {
        refused_then_allowed(
            &[
                "git push origin 'main'",
                "git push origin \"main\"",
                "git push origin \"HEAD:main\"",
                "git push origin 'feature:refs/heads/main'",
                "git push origin \"+refs/heads/x:refs/heads/main\"",
                "git push origin '+feature'",
                "git push origin ':feature'",
                "git push origin 'v1.2.3'",
                "git push origin \"refs/tags/v1\"",
                "git push origin $B",
                "git push origin \"$B\"",
                "git push origin ${B}:main",
                "git push origin $(git branch --show-current)",
                "git push origin `git branch --show-current`",
                "git push origin ma\\in",
            ],
            &[
                "git push origin 'feature'",
                "git push -u origin \"claude/fix-1\"",
                "git push origin 'HEAD:feature'",
            ],
        );
        assert!(bash("git push origin 'HEAD'", "main").is_some());
        // The refusal says how to fix it in one step.
        let why = bash("git push -u origin \"$BRANCH\"", "feature").unwrap_or_default();
        assert!(why.contains("name the branch literally"), "{why}");
    }

    /// Review of #351: every `gh release` write, not only `create`.
    #[test]
    fn merge_lock_refuses_every_gh_release_write() {
        refused_then_allowed(
            &[
                "gh release edit v1.0.0 --draft=false",
                "gh release upload v1.0.0 bosun.tar.gz",
                "gh release delete v1.0.0 -y",
                "gh release delete-asset v1.0.0 bosun.tar.gz",
            ],
            &[
                "gh release view v1.0.0",
                "gh release list",
                "gh release download v1.0.0",
            ],
        );
    }

    /// Review of #351: GraphQL merge and ref mutations, and a query the guard cannot see.
    #[test]
    fn merge_lock_refuses_graphql_writes_and_hidden_queries() {
        refused_then_allowed(
            &[
                "gh api graphql -f query='mutation{mergeBranch(input:{}){clientMutationId}}'",
                "gh api graphql -f query='mutation{deleteRef(input:{refId:\"x\"}){clientMutationId}}'",
                "gh api graphql -f query='mutation{createRef(input:{}){ref{name}}}'",
                "gh api graphql -f query='mutation{mergePullRequest(input:{}){clientMutationId}}'",
                "gh api graphql -f query='mutation{enablePullRequestAutoMerge(input:{}){clientMutationId}}'",
                "gh api graphql -f query='mutation{disablePullRequestAutoMerge(input:{}){clientMutationId}}'",
                "gh api graphql -f query='mutation{enqueuePullRequest(input:{}){clientMutationId}}'",
                "gh api graphql -F query=@q.graphql",
                "gh api graphql -f query=@q.graphql",
                "gh api graphql -Fquery=@q.graphql",
                "gh api graphql --field query=@q.graphql",
                "gh api graphql --raw-field=query=@q.graphql",
                "gh api graphql --input q.json",
                "gh api graphql -f query=\"$Q\"",
                "gh api graphql -f query=$(cat q.graphql)",
            ],
            &[
                "gh api graphql -f query='query{viewer{login}}'",
                "gh api graphql -f query='query($o:String!){repository(owner:$o,name:\"r\"){id}}' -f o=x",
                "gh api graphql -F number=12 -f query='query{viewer{login}}'",
            ],
        );
    }

    /// Review of #351: pushes that carry tags.
    #[test]
    fn merge_lock_refuses_tag_carrying_pushes() {
        refused_then_allowed(
            &[
                "git push --follow-tags",
                "git push --follow-tags origin feature",
                "git push --tags origin",
                "git push origin feature --tags",
            ],
            &[
                "git push origin feature",
                "git push --no-follow-tags origin feature",
            ],
        );
    }

    /// Review of #351: a git alias hides the command it runs.
    #[test]
    fn merge_lock_refuses_git_aliases() {
        refused_then_allowed(
            &[
                "git -c alias.p='push origin main' p",
                "git -c 'alias.p=!git push --force' p",
                "git -c Alias.P=push p origin main",
                "git --config-env=alias.p=P p",
                "git config alias.p 'push origin main'",
                "git config --global alias.p '!git push -f'",
                "git config --add alias.p push",
                "git config set alias.p push",
                "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.p GIT_CONFIG_VALUE_0=push git p",
            ],
            &[
                "git -c core.pager=cat log",
                "git config --get alias.p",
                "git config --get-regexp alias",
                "git config get alias.p",
                "git config --list",
                "git config user.name x",
            ],
        );
    }

    /// Review of #351: `env -S` splits a string into a command; `builtin` and `command` run
    /// the command after them.
    #[test]
    fn merge_lock_unwraps_env_split_string_builtin_and_command() {
        refused_then_allowed(
            &[
                "env -S 'git push origin main'",
                "env -S 'git push' origin main",
                "env -u X -S 'gh pr merge 12'",
                "env -S'git push --force'",
                "env --split-string='git push --force'",
                "/usr/bin/env -S \"gh release create v1\"",
                "builtin command git push origin main",
                "builtin exec git push --force",
                "builtin eval 'git push origin main'",
                "command git push origin main",
                "command -p gh pr merge 12",
            ],
            &[
                "env -S 'cargo test'",
                "env -S 'git push origin feature:feature'",
                "command -v git",
                "builtin echo merge",
            ],
        );
    }

    /// Review of #351: workflow runs, secrets and variables.
    #[test]
    fn merge_lock_refuses_workflow_runs_secret_and_variable_writes() {
        refused_then_allowed(
            &[
                "gh workflow run release.yml",
                "gh secret set TOKEN < t",
                "gh secret delete TOKEN",
                "gh variable set X --body 1",
                "gh variable delete X",
            ],
            &[
                "gh workflow list",
                "gh workflow view ci.yml",
                "gh secret list",
                "gh variable list",
                "gh variable get X",
            ],
        );
    }

    /// Gaps: release writes through `gh api`, `xargs` around gh or git writes (stdin adds
    /// arguments the guard cannot see), `gh alias` (new commands that bypass the guard), and
    /// pushes of every ref or of a glob.
    #[test]
    fn merge_lock_refuses_release_xargs_alias_and_bulk_push_gaps() {
        for cmd in [
            "gh api repos/o/r/releases -f tag_name=v1.0.0",
            "gh api -X POST repos/o/r/releases",
            "gh api --method PATCH repos/o/r/releases/1 -F draft=false",
            "gh api repos/o/r/releases --input rel.json",
            "echo 12 | xargs gh api -X PUT repos/o/r/pulls/12/merge",
            "echo repos/o/r/pulls/12/merge | xargs gh api -X PUT",
            "echo -X PUT | xargs gh api repos/o/r/pulls/12/merge",
            "echo main | xargs git push origin",
            "echo v1 | xargs -I{} git tag {}",
            "echo 12 | xargs -n1 gh pr merge",
            "echo x | xargs -0 git merge",
            "timeout 5 git push origin main",
            "nice -n 5 gh pr merge 12",
            "gh alias set pm 'pr merge'",
            "gh alias set --shell x 'git push origin main'",
            "gh alias import aliases.yml",
            "git push origin --all",
            "git push --all origin",
            "git push --branches origin",
            "git push --mirror origin",
            "git push origin 'refs/heads/*'",
            "git push origin refs/heads/*:refs/heads/*",
            "git push origin 'refs/tags/*'",
            "git push origin '*:*'",
            "git push origin \"*\"",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        for cmd in [
            "gh api repos/o/r/releases",
            "gh api repos/o/r/releases/latest --jq .tag_name",
            "gh release view v1.0.0",
            "gh release list",
            "gh alias list",
            "git ls-files | xargs wc -l",
            "echo 1 | xargs gh pr view",
            "git branch --all",
            "git push -u origin feature",
            "timeout 60 cargo test",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    #[test]
    fn tag_mirror_fetch_and_update_ref_values() {
        for cmd in [
            "git fetch origin 'refs/tags/*:refs/tags/*'",
            "git fetch origin \"+refs/tags/*:refs/tags/*\"",
            "git update-ref refs/heads/x abc123",
            "git update-ref -m 'a b' refs/heads/x abc123",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
        for cmd in [
            "git fetch . HEAD:refs/tags/v1",
            "git fetch origin main:refs/t\\ags/v1",
            "git fetch origin refs/tags/v1:refs/tags/v1",
            "git fetch origin 'refs/tags/*:refs/tags/*' HEAD:refs/tags/v1",
            "git fetch origin refs/heads/*:refs/tags/*",
            "git fetch origin main:tags/v1",
            "git fetch origin +HEAD:tags/v1",
            "git fetch origin HEAD:$T",
            "git fetch . \"HEAD:$T\"",
            "git -c push.followTags=true push origin feature",
            "git config push.followTags true",
            "git update-ref $ref abc",
            "git update-ref -m 'a b' $r HEAD",
            "git update-ref -m \"a b\" $r HEAD",
            "git update-ref -m \"a b\"c $r HEAD",
            "git update-ref -m a' 'b $r HEAD",
            "git update-ref -m\"a b\" $r HEAD",
            "git update-ref -dm x $r",
            "git update-ref refs/heads/x \"$sha\"",
            "git update-ref -m \"$msg\" refs/heads/x abc",
            "git update-ref \"$ref\" abc",
            "git update-ref -d $ref",
            "git update-ref refs/tags/v1 abc",
            "git update-ref refs/heads/x refs/tags/v1",
            "git update-ref --stdin",
            "git update-ref refs/heads/x `git rev-parse HEAD`",
            "git update-ref refs/he\\ads/x abc",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
    }

    /// Review of the merge-lock: `symbolic-ref` tags, brace refspecs, local fetches and a
    /// subcommand the shell builds.
    #[test]
    fn merge_lock_refuses_symbolic_ref_braces_local_fetch_and_built_subcommands() {
        refused_then_allowed(
            &[
                "git symbolic-ref refs/tags/x refs/heads/main",
                "git symbolic-ref -m why refs/tags/x refs/heads/main",
                "git symbolic-ref 'refs/tags/x' refs/heads/feature",
                "git symbolic-ref tags/x refs/heads/main",
                "git symbolic-ref $r refs/heads/main",
                "git symbolic-ref refs/t\\ags/x refs/heads/main",
                "git push origin refs/{tags,x}/v1",
                "git push origin {main,feat}",
                "git push origin feat:{main,x}",
                "git push origin refs/{heads,tags}/{a,b}",
                "git update-ref refs/{tags,heads}/v1 HEAD",
                "git symbolic-ref refs/{tags,x}/v1 refs/heads/main",
                "git fetch . refs/{tags,x}/v1:refs/{tags,x}/v1",
                "git fetch . $SPEC",
                "git fetch . \"$SPEC\"",
                "git fetch . HEAD:refs/tags/v1",
                "git fetch . feat:main",
                "git fetch . +feat:refs/heads/main",
                "git fetch ../other feat:main",
                "git fetch /tmp/repo feat:heads/main",
                "git fetch --depth 1 . feat:main",
                "g=merge; git $g x",
                "git \"$g\" x",
                "git ${G} origin main",
                "git -C wt $g x",
                "git $(echo push) origin main",
                "git push origin v{1..3}",
                "git push origin x{1..65}",
            ],
            &[
                "git symbolic-ref HEAD",
                "git symbolic-ref --short HEAD",
                "git symbolic-ref -q HEAD",
                "git symbolic-ref HEAD refs/heads/feature",
                "git push origin {feat,feat}:{feat,feat}",
                "git push -u origin feature:feature",
                "git log --format='{a,b}' -1",
                "git log '{a,b}'",
                "gh api repos/{owner}/{repo}/pulls",
                "find . -name x -exec grep -l y {} +",
                "git fetch origin",
                "git fetch origin main",
                "git fetch origin main:refs/remotes/origin/main",
                "git fetch --all --prune",
                "git fetch . feat:other",
                "for i in {1..3}; do echo $i; done",
                "git -C \"$dir\" status",
                "git status",
            ],
        );
    }

    /// Second review: empty brace words, `{x..y}` behind a quoted name, fetch and ref moves
    /// of the default branch, git's value-taking global options. Each refusal names its reason.
    #[test]
    fn merge_lock_judges_brace_words_fetches_and_moves_of_the_default_branch() {
        const MAIN: &str = "push to the default branch";
        const FETCH: &str = "a fetch into the default branch";
        const MOVE: &str = "moving the default branch";
        let many = format!("git push origin {}", "{a,b}".repeat(20));
        let cases = [
            ("{git,} push origin HEAD:main", MAIN),
            ("git {,} push origin HEAD:main", MAIN),
            ("{sudo,} git push origin HEAD:main", MAIN),
            ("git push origin {,}", MAIN),
            ("{,git} merge x", "changing the default branch"),
            ("{gh,} pr merge 1", "pr merge"),
            ("\\git push origin HEAD:ma{i..i}n", MAIN),
            ("\\git push origin v{1..1}", "tag"),
            ("\"git\" push origin v{1..3}", "tag"),
            ("sudo git push origin v{1..3}", "tag"),
            ("\\gh api repos/o/r/pulls/{7..7}/merge -X PUT", "merge API"),
            (many.as_str(), UNREAD_BRACE),
            ("git fetch . feat:ma\\in", FETCH),
            ("git fetch \\. feat:main", FETCH),
            ("git fetch --filter blob:none . feat:main", FETCH),
            (
                "git fetch --recurse-submodules-default yes . x:refs/heads/main",
                FETCH,
            ),
            ("git fetch . 'refs/heads/*:refs/heads/*'", FETCH),
            ("git fetch origin 'refs/heads/*:refs/heads/*'", FETCH),
            ("git fetch origin feat:main", FETCH),
            ("git fetch /tmp/other main:main", FETCH),
            ("git fetch origin $SPEC", FETCH),
            ("git update-ref refs/heads/main HEAD~1", MOVE),
            ("git update-ref -d refs/heads/main", MOVE),
            ("git symbolic-ref refs/heads/main refs/heads/x", MOVE),
            ("git branch -f main HEAD~1", MOVE),
            ("git branch --force main x", MOVE),
            ("git branch -M x main", MOVE),
            ("git checkout -B main origin/x", MOVE),
            ("git switch -C main x", MOVE),
            ("git --namespace x push origin HEAD:main", MAIN),
            ("git --attr-source x merge y", "changing the default branch"),
        ];
        for (cmd, why) in cases {
            let got = bash(cmd, "main").unwrap_or_default();
            assert!(got.contains(why), "{cmd}: {got:?}");
        }
        for cmd in [
            "git reset --hard HEAD@{1}",
            "git stash apply stash@{0}",
            "git rebase @{u}",
            "git log @{u}..",
            "git diff @{u}..HEAD",
            "gh api repos/{owner}/{repo}",
            "git log --format='{a,b}' -1",
            "git push origin {feat,}",
            "git fetch origin main:main",
            "git fetch origin '+refs/heads/*:refs/remotes/origin/*'",
            "git fetch origin 'refs/tags/*:refs/tags/*'",
            "git fetch --filter blob:none origin",
            "git symbolic-ref HEAD refs/heads/main",
            "git branch -f feat main",
            "git switch -C fix main",
            "git checkout -B fix origin/main",
            "git --namespace x status",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// Third review: `{x..y}` in the command name, braces before runners, `branch` copy and
    /// move flags, stuck values, and globs that cannot reach the default branch.
    /// Third review, round 2: sequence bounds near `i64` limits, braces a quote split leaves
    /// open, and long words with many open braces.
    #[test]
    fn brace_sequences_at_the_i64_limits_and_open_braces_fail_closed() {
        let start = std::time::Instant::now();
        let max = i64::MAX;
        assert_eq!(sequence(&format!("1..2..{max}")).unwrap(), ["1"]);
        assert_eq!(
            sequence(&format!("{max}..{max}")).unwrap(),
            [max.to_string()]
        );
        assert_eq!(sequence(&format!("1..2..{}", i64::MIN)).unwrap(), ["1"]);
        assert_eq!(sequence(&format!("n..n..{max}")).unwrap(), ["n"]);
        assert_eq!(sequence(&format!("{}..{max}", i64::MIN)), None);
        assert_eq!(
            sequence(&format!("{max}..{}..{max}", i64::MIN))
                .unwrap()
                .len(),
            3
        );
        assert_eq!(sequence("1..5..2").unwrap(), ["1", "3", "5"]);
        assert_eq!(sequence("c..a").unwrap(), ["c", "b", "a"]);
        assert_eq!(sequence("01..03").unwrap(), ["01", "02", "03"]);
        let long = format!("git push origin {}", "{".repeat(20_000));
        assert!(bash(&long, "feature").is_some());
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        for cmd in [
            "git push origin {HEAD:main,'--repo=x y'}",
            "gh pr merge {1,'2 x'}",
        ] {
            let got = bash(cmd, "feature").unwrap_or_default();
            assert!(got.contains(UNREAD_BRACE), "{cmd}: {got:?}");
        }
        for cmd in ["echo {a,'b c'}", "git log --format='{a,b'"] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// Third review, round 3: an unexpandable brace word is refused only where it can be or
    /// steer the command word, so quoted jq and JSON text pass and runner values do not hide git.
    #[test]
    fn unexpandable_brace_words_are_judged_by_where_they_sit() {
        let big = format!("{{git,push,origin,HEAD:main{}}}", ",-v".repeat(61));
        for cmd in [
            big.clone(),
            format!("timeout 60s {big}"),
            format!("sudo -u root {big}"),
            format!("env -u HOME {big}"),
            format!("\\sudo {big}"),
            format!("\"env\" {big}"),
            format!("nice -n 5 {big}"),
        ] {
            let got = bash(&cmd, "main").unwrap_or_default();
            assert!(got.contains(UNREAD_BRACE), "{cmd}: {got:?}");
        }
        for cmd in [
            "gh pr list --json number,title --jq '.[] | {number, title}'",
            "gh api repos/o/r/pulls | jq '.[] | {number, title, state}'",
            "cat > x.json <<'EOF'\n{\"t\":\"a b\",\"ok\":1}\nEOF",
            "echo '{\"t\":\"a b\",\"ok\":1}' | jq .",
            "{a..z}{a..z} push origin HEAD:main",
        ] {
            assert_eq!(bash(cmd, "main"), None, "{cmd}");
        }
        let got = bash("git fetch origin \"$BRANCH\"", "main").unwrap_or_default();
        assert!(got.contains("name the branch literally"), "{got}");
    }

    /// Owner review: `//` and `/./` in a ref, abbreviated long options, `--refmap` and a
    /// `remote.<r>.fetch` set on the command line.
    #[test]
    fn merge_lock_normalises_refs_abbreviations_refmaps_and_fetch_config() {
        for cmd in [
            "git symbolic-ref refs//tags/v1 refs/heads/feat",
            "git symbolic-ref refs/./tags/v1 refs/heads/feat",
            "git update-ref refs//tags/v1 HEAD",
            "git update-ref ./refs/tags/v1 HEAD",
            "git update-ref refs//heads/main HEAD",
            "git symbolic-ref refs/./heads//main refs/heads/x",
            "git fetch . feat:refs//heads/main",
            "git fetch . HEAD:refs//tags/v1",
            "git branch --forc main feat",
            "git branch --cop --forc x main",
            "git branch --mo --force x main",
            "git switch --force-c main feat",
            "git checkout --force-c=main feat",
            "git switch --force-cr main feat",
            "git fetch --refmap=+refs/heads/feat:refs/heads/main origin feat",
            "git fetch --refmap +refs/heads/feat:refs/heads/main origin feat",
            "git fetch --refmap='refs/heads/*:refs/heads/*' origin",
            "git -c remote.origin.fetch=+refs/heads/feat:refs/heads/main fetch origin",
            "git -c remote.origin.fetch=+refs/tags/x:refs/tags/x fetch origin",
            "git --config-env=remote.origin.fetch=F fetch origin",
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=x:main git fetch origin",
            // Later review: any `-c` on a fetch or pull is refused (was allowed).
            "git -c remote.origin.fetch=+refs/heads/*:refs/remotes/origin/* fetch origin",
            "git -c core.x=1 fetch origin",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        for cmd in [
            "git symbolic-ref HEAD refs/heads/feat",
            "git branch --format=x",
            "git switch --force feat",
            "git fetch --refmap= origin",
            "git fetch --refmap='+refs/heads/*:refs/remotes/origin/*' origin",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// Owner review: abbreviated `--refmap` and value options must not hide a local remote.
    #[test]
    fn merge_lock_reads_abbreviated_fetch_options() {
        for cmd in [
            "git fetch --refm=+refs/heads/feat:refs/heads/main origin feat",
            "git fetch --refma +refs/heads/feat:refs/heads/main origin feat",
            "git fetch --dept 1 /tmp/evil +main:main",
            "git fetch --jo 1 ../evil main:main",
            "git fetch --upload-pa x . main:main",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        for cmd in [
            "git fetch --dept 1 origin main:main",
            "git fetch --refm= origin",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    /// Known misses closed: fetch config from an earlier segment or `git config`, a local
    /// path with `:`, an unknown remote, and long brace sequences in read-only git commands.
    #[test]
    fn merge_lock_judges_fetch_config_remotes_and_read_only_sequences() {
        for cmd in [
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=feat:main; git fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=+x:refs/tags/v1\ngit fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=x:main && bash -c 'git fetch origin'",
            "export GIT_CONFIG_PARAMETERS=\"'remote.origin.fetch=+feat:main'\"; git fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=$V; git fetch origin",
            "git config remote.origin.fetch +feat:main && git fetch origin",
            "git config --add remote.origin.fetch +feat:refs/heads/main",
            "git config --replace-all remote.origin.fetch 'x:refs/tags/v1'",
            "git config set remote.origin.fetch +feat:main",
            "git config remote.origin.fetch x:tags/v1",
            "git fetch ./ev:il +main:main",
            "git fetch ../a:b main:main",
            "git fetch ~/a:b main:main",
            "git fetch /tmp/a:b main:main",
            "git fetch evil2 +main:main",
            "git fetch evil2 main:refs/heads/main",
            "git fetch https://example.invalid/x.git main:main",
            "git fetch evil2 'refs/tags/*:refs/tags/*'",
            "git fetch --dry evil2 main:main",
            // Review round 1: repeated values, the split-quoted form, mirror remotes, retargets.
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=+refs/heads/*:refs/remotes/origin/* GIT_CONFIG_VALUE_0=+feat:main; git fetch origin",
            "export GIT_CONFIG_PARAMETERS=\"'remote.origin.fetch'='+feat:main'\"; git fetch origin",
            "git remote add --mirror=fetch evil /tmp/evil && git fetch evil",
            "git remote add upstream /tmp/evil && git fetch upstream +main:main",
            "git remote set-url origin /tmp/evil && git fetch origin +main:main",
            "git remote rename evil upstream",
            "git -c remote.origin.url=/tmp/evil fetch origin +main:main",
            "git config remote.origin.url ../evil",
            "git config url./tmp/evil.insteadOf https://github.com/ && git fetch origin +main:main",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.url GIT_CONFIG_VALUE_0=/tmp/evil; git fetch origin +main:main",
            // Review round 2: bare directories, option values, first params entry, braces, pull.
            "git remote set-url origin evil2 && git fetch origin +main:main",
            "git remote add upstream evil2",
            "git config remote.origin.url evil2",
            "git -c remote.origin.url=evil2 fetch origin +main:main",
            "git config url.evil2/.insteadOf https://github.com/",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.url GIT_CONFIG_VALUE_0=evil2; git fetch origin +main:main",
            "git remote add -t main upstream /tmp/evil",
            "git remote add --track main upstream /tmp/evil",
            "git remote add -m main upstream /tmp/evil",
            "git remote add --master main upstream /tmp/evil",
            "export GIT_CONFIG_PARAMETERS=\"'remote.origin.url=/tmp/x'\"; git fetch origin +main:main",
            "export GIT_CONFIG_PARAMETERS=\"'remote.origin.url'='/tmp/x'\"; git fetch origin +main:main",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=+feat:{x,main}; git fetch",
            "git pull evil2 +main:main",
            "git pull ./ev:il +main:main",
            "git pull /tmp/x +main:main",
            "git pull . HEAD:refs/tags/v1",
            "git diff {a..z}{a..z}--output=.git/refs/heads/main",
            "git grep {a..z}{a..z}-O",
            // Review round 3: short clusters, abbreviations, quoted mirror, key escapes, --tags.
            "git fetch -po origin evil2 +main:main",
            "git pull -qo origin evil2 +main:main",
            "git fetch -po origin evil2 'refs/tags/*:refs/tags/*'",
            "git remote add --tr main upstream /tmp/evil",
            "git remote add --mas main upstream /tmp/evil",
            "git remote add -ft main upstream /tmp/evil",
            "git remote add -fm main upstream /tmp/evil",
            "git remote add '--mirror=fetch' evil https://x && git fetch evil",
            "git remote add --mi=fetch evil https://x",
            "git remote add --mirr=fetch evil https://x",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.{x,fetch} GIT_CONFIG_VALUE_0=+feat:main; git fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetc\\h GIT_CONFIG_VALUE_0=+feat:main; git fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.ur\\l GIT_CONFIG_VALUE_0=/tmp/evil; git fetch origin +main:main",
            "git -c remote.origin.ur\\l=/tmp/x fetch origin +main:main",
            "git -c remote.origin.fetc\\h=+feat:main fetch origin",
            "git fetch --tags -f evil2",
            "git fetch --tags --force /tmp/evil",
            "git fetch -t evil2",
            "git grep {!..~}{!..~}",
            "git log {!..~}",
            "git log {0..z}",
            // Review round 4: every remote, prune-tags, letter-by-letter clusters.
            "git fetch --all --tags",
            "git fetch --all --tags --force",
            "git pull --all --tags",
            "git fetch --multiple origin evil2 --tags",
            "git fetch -pP evil2",
            "git fetch --prune --prune-tags --force evil2",
            "git fetch -oxs evil2 origin +main:main",
            "git pull -Xtheirs evil2 origin +main:main",
            // Review round 5: `tag <name>` shorthand, config section renames.
            "git fetch evil2 tag v1.0",
            "git fetch origin tag v1.0",
            "git pull evil2 tag v1.0",
            "git fetch evil2 +tag v1.0",
            "git config --rename-section remote.evil remote.upstream",
            "git config rename-section remote.evil remote.origin",
            // Round 6: config files and indirect names before a fetch or pull.
            "GIT_CONFIG_GLOBAL=/tmp/g git fetch origin",
            "export GIT_CONFIG_GLOBAL=/tmp/g; git fetch origin",
            "GIT_CONFIG_SYSTEM=/tmp/g git pull origin main",
            "export GIT_CONFIG_SYSTEM=/tmp/g\ngit fetch origin",
            "git -c include.path=/tmp/g fetch origin",
            "git -c includeIf.onbranch:feature.path=/tmp/g pull",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=include.path GIT_CONFIG_VALUE_0=/tmp/g; git fetch origin",
            "export GIT_CONFIG_PARAMETERS=\"'include.path=/tmp/g'\"; git fetch origin",
            "K=GIT_CONFIG_KEY_0; export $K=remote.origin.fetch GIT_CONFIG_COUNT=1 GIT_CONFIG_VALUE_0=+feat:main; git fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_VALUE_0=+feat:main; git fetch origin",
            "export $K=x; git pull origin main",
            // Delta review: any config name or `$` before `=` next to a fetch or pull.
            "GIT_CONFIG_GLOBAL+=/tmp/g git fetch origin",
            "export GIT_CONFIG_GLOBAL+=/tmp/g; git fetch origin",
            "read GIT_CONFIG_GLOBAL <<< /tmp/g; export GIT_CONFIG_GLOBAL; git fetch origin",
            "printf -v GIT_CONFIG_GLOBAL /tmp/g; export GIT_CONFIG_GLOBAL; git pull",
            "HOME=/tmp/h git fetch origin",
            "XDG_CONFIG_HOME=/tmp/x git fetch origin",
            "K=GIT_CONFIG_KEY_0 V=GIT_CONFIG_VALUE_0 C=GIT_CONFIG_COUNT; export \"$K\"=remote.origin.fetch \"$V\"=+feat:main \"$C\"=1; git fetch origin",
            "env \"$K\"=remote.origin.fetch \"$V\"=+feat:main \"$C\"=1 git fetch origin",
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.x GIT_CONFIG_VALUE_0=1; git fetch origin",
            "export GIT_CONFIG_NOSYSTEM=1; git fetch origin",
            "export $(cat f); git fetch origin",
            "env $(cat f) git fetch origin",
            "source f.sh; git fetch origin",
            ". f.sh && git pull",
            "export `echo Zm9v | base64 -d`; git fetch origin",
            // Allowlist: fetch or pull only in plain `git ..` / `cd ..` chains.
            "export FOO=bar; git fetch",
            "eval \"$(cat f.sh)\"; git fetch origin",
            "while read l; do export \"$l\"; done < f; git fetch origin",
            "{ . f.sh; git fetch origin; }",
            "if true; then . f.sh; fi; git fetch origin",
            "builtin source f.sh; git fetch origin",
            "\\source f.sh; git fetch origin",
            "BASH_ENV=f.sh bash -c \"git fetch origin\"",
            "s=source; $s f; git fetch origin",
            "export $v; git pull",
            "bash -c 'git fetch origin'",
            "git fetch origin < /dev/null",
            "git -c core.x=1 pull origin main",
            "git log \" -m 'a\" ; HOME=/tmp/h git fetch origin ; git log \"'\"",
            "git log \" --title 'a\" ; GIT_CONFIG_GLOBAL=/tmp/g git pull ; git log \"'\"",
            "git -c protocol.ext.allow=always fetch 'ext::sh -c git% update-ref% refs/heads/main% refs/heads/feat'",
            "git -c 'url.ext::sh -c x%#.insteadOf=https://' -c protocol.ext.allow=always fetch origin",
            "git fetch 'ext::sh -c x'",
            "git fetch fd::3",
            "git -c protocol.ext.allow=always fe\"\"tch 'ext::sh -c x'",
            // Unquoted words, upload-pack and command transports in any git command.
            "git fetch --upload-pack='git update-ref refs/heads/ma''in abc #' .",
            "git pull --upload-pack='x #' .",
            "git fetch -u 'x #' .",
            "git fetch --upl='x #' .",
            "git fetch --upload-pack 'x #' .",
            "git --exec-path=/tmp/d fetch origin",
            "git '-c' protocol.ext.allow=always fetch 'e'\"xt::sh /tmp/p.sh\"",
            "git '--config-env=protocol.ext.allow=E' fetch origin",
            "git fetch 'fd'::3",
            "git remote add x 'ext::sh /tmp/p.sh'",
            "git remote set-url origin ext::sh",
            "git config remote.x.url 'fd::3'",
            "git clone 'ext::sh /tmp/p.sh' d",
            "git push 'ext::sh x' feat:feat",
            // ANSI-C and locale quoting: any `$` near a fetch, `$'`/`$"` in any git command.
            "git fetch $'--upload-pack=git update-ref refs/heads/main abc #' .",
            "git fetch $'-u' 'x #' .",
            "git fetch $'-qu' 'x #' .",
            "git fetch --$'upload-pack'=x .",
            "git fetch $\"--upload-pack\"=x .",
            "git pull $'--upload-pack=x #' .",
            "git remote add x $'\\x65xt::sh p.sh'",
            "git remote add x $'\\145xt::sh p.sh'",
            "git fetch origin $BRANCH",
            // Globs: bash expands them against files the guard cannot see.
            "/usr/bin/gi[t] fetch '--upload-pack=sh p.sh #' .",
            "/usr/bin/gi[t] fetch . +side:main",
            "/usr/bin/g?t remote add x 'ext::sh p'",
            "git f?tch '--upload-pack=sh p #' .",
            "git fetch *upload-pack* .",
            "git fetch . +side:mai[n]",
            "g*t push origin HEAD:main",
            // A glob that can name git, behind any wrapper or nesting.
            "env -i /usr/bin/gi[t] fetch . +side:main",
            "env -u X /usr/bin/gi[t] fetch . +side:main",
            "nice -n 5 /usr/bin/gi[t] fetch . +side:main",
            "sudo -u root /usr/bin/gi[t] fetch . +side:main",
            "command -p /usr/bin/gi[t] fetch . +side:main",
            "exec -a x /usr/bin/gi[t] fetch . +side:main",
            "timeout 5 /usr/bin/gi[t] fetch . +side:main",
            "stdbuf -o0 /usr/bin/gi[t] fetch . +side:main",
            "setsid -w /usr/bin/gi[t] fetch . +side:main",
            "xargs /usr/bin/gi[t] fetch . <<< '+side:main'",
            "bash -c '/usr/bin/gi[t] fetch . +side:main'",
            "eval '/usr/bin/gi[t] fetch . +side:main'",
            "timeout 5 g?t push origin HEAD:main",
            "nice *t push",
            "echo x | xargs *",
            // git-core programs are git; `$` in nested shell text near a fetch.
            "git-fetch . +feat:main",
            "/usr/lib/git-core/git-fetch . +feat:main",
            "git-pull . feat:main",
            "git fetch . feat:MAIN",
            "git fetch . feat:refs/heads/Main",
            // A git run inside another git subcommand skips the config walk.
            "git bisect run git -C t2 fetch",
            "git submodule foreach git -C ../t2 fetch",
            "git submodule foreach 'git pull'",
            "git -C x submodule foreach git remote update",
            "git-submodule foreach git fetch",
            // Command-running subcommands with a fetch word anywhere, quoted too.
            "git bisect run sh -c \"cd t2; git fetch\"",
            "git submodule foreach 'cd ../t2 && git fetch'",
            "git difftool --extcmd='cd t2 && git fetch'",
            "git filter-branch --index-filter 'cd t2 && git fetch'",
            "git rebase -x 'git pull' main",
            // A fetch word in a quoted or `=` value of any git command.
            "git rebase --exe 'cd t2 && git fetch' main",
            "git rebase --ex 'cd t2 && git fetch' main",
            "git rebase -x'cd t2 && git fetch' main",
            "git rebase -ix 'cd t2 && git fetch' main",
            "git submodule --quiet foreach 'cd ../t2 && git fetch'",
            "git -c sequence.editor='cd t2 && git fetch #' rebase -i main",
            "git -c core.editor='cd t2 && git pull #' commit",
            "git instaweb --browser='cd t2 && git fetch'",
            "git web--browse --browser='cd t2 && git fetch' x",
            "git -c browser.x.cmd='cd t2 && git fetch' help -w log",
            "git send-email --to-cmd='cd t2 && git remote update' x.patch",
            "git rebase -x 'git fe\"\"tch' main",
            "git rebase -x \"cd t2 && git 'fetch'\" main",
            "git rebase -x 'cd t2 && git f\"e\"tch' main",
            "git rebase -x 'git fetc?' main",
            "git rebase -x 'git f*h' main",
            // Expansions and git-core programs inside a value.
            "git rebase -x 'cd t2 && git {fetch,}' main",
            "git rebase -x 'git -C t2 $(echo fe)tch' main",
            "git rebase -x 'git -C t2 `echo fe`tch' main",
            "git rebase -x 'cd t2 && /usr/lib/git-core/git-fetch' main",
            "git rebase -x 'cd t2 && ./git-core/git-fe*' main",
            "git rebase -x 'cd t2 && ./x/*' main",
            // `${IFS}` and other `$` or backtick in a value, without a space.
            "git rebase -x 'cd${IFS}t2;git${IFS}fe$@tch' main",
            "git rebase -x 'git${IFS}{fetch,}' main",
            "git rebase -x '$(echo${IFS}fe)tch' main",
            "git rebase -x '`echo${IFS}fe`tch' main",
            "git rebase -x 'f${x}etch' main",
            "git rebase -x 'cd;git;{fetch,}' main",
            "X=$(printf 'git f%stch' e); git rebase -x \"$X\" main",
            "export X=$(printf x); git rebase -x '$X' main",
            // Special or shell-kept variables, and values git runs as code.
            "set -- 'cd${IFS}t2;git${IFS}fe$@tch'; git rebase -x \"$1\" main",
            ": 'x'; git rebase -x \"$_\" main",
            "git rebase -x \"$PWD\" main",
            "git push origin \"$OLDPWD\"",
            "git rebase -x \"$CMD\" main",
            "git rebase --exec=\"$CMD\" main",
            "git -c core.editor=\"$ED\" commit",
            "git bisect run \"$CMD\"",
            "git submodule foreach \"$CMD\"",
            "git fetch --upload-pack=\"$UP\" origin",
            // A plain variable passes only as a -C/--git-dir/--work-tree value or a positional
            // ref or path of a read or ref command, and never after a builtin that sets one.
            "git clone -u \"$CMD\" t2 t3",
            "source ./v; git push origin \"$B\"",
            "declare -x C\\MD=x; git push origin \"$CMD\"",
            "eval 'C''MD=x'; git push origin \"$CMD\"",
            ". ./v; git config core.pager \"$CMD\"",
            "git config core.editor \"$CMD\"",
            "git config diff.x.command \"$CMD\"",
            "git checkout -b \"$B\"",
            "function --exec=./p.sh { git rebase \"$FUNCNAME\" main; }; --exec=./p.sh",
            "f() { git log \"$X\"; }; f",
            "git switch \"$BRANCH\"",
            "git diff \"$BASE\" HEAD",
            "git add \"$FILE\"",
            "git-push origin HEAD:main",
            "/usr/lib/git-core/git-update-ref refs/heads/main HEAD",
            "git-fetch --upload-pack='x #' .",
            "bash -c 'git fetch $R'",
            "sh -c \"git pull origin $B\"",
            // A bare `*` component can name git (was allowed).
            "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.fetch GIT_CONFIG_VALUE_0=+refs/heads/*:refs/remotes/origin/*; git status",
            // Unquoted, bash may expand it against planted files (was allowed).
            "git fetch origin +refs/tags/*:refs/tags/*",
            "git remote rename evil upstream",
            "git push origin {1..100}",
            "git update-ref refs/heads/x {1..100}",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        for cmd in [
            "git fetch origin main:main",
            "git fetch upstream main:main",
            "git fetch origin 'refs/tags/*:refs/tags/*'",
            "git fetch --dept 1 origin main:main",
            "git fetch --depth=1 origin main:main",
            "git fetch evil2 feat:feat",
            "git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'",
            "git config --get remote.origin.fetch",
            "git log -n {1..100}",
            "git remote add upstream https://example.invalid/x.git",
            "git remote set-url origin https://example.invalid/x.git",
            "git remote add backup /tmp/backup",
            "git config remote.origin.url https://example.invalid/x.git",
            "git remote set-url origin git@example.invalid:x/y.git",
            "git remote add -t main upstream https://example.invalid/x.git",
            "git pull origin main",
            "git pull --rebase origin feature",
            "git fetch --tags",
            "git fetch --tags origin",
            "git fetch -p origin",
            "git remote set-url --push origin /tmp/x",
            "git log {a..z}",
            "git log {1..100..2}",
            "git pull -Xtheirs origin feature",
            "git remote rename origin upstream",
            "git config --rename-section remote.origin remote.upstream",
            "git fetch origin feature",
            "git remote set-url --pu origin /tmp/x",
            "GIT_CONFIG_GLOBAL=/tmp/g git status",
            "git fetch",
            "git pull --rebase",
            "git config user.name x",
            "git config --get remote.origin.url",
            "git fetch . feat:feat",
            "git fetch && git rebase origin/main",
            "cd x && git pull",
            "git fetch origin && git merge --ff-only origin/main",
            "git pull --rebase 2>&1 | tail -5",
            "git -C x fetch origin",
            "git pull -Xours origin feature",
            "git fetch --update-head-ok origin",
            "git commit -m 'why? fix *all* [x]'",
            "git fetch origin 'refs/heads/*:refs/remotes/origin/*'",
            "[ -d x ] && git status",
            "ls *.rs",
            "rm -f *.log",
            "git-fetch origin",
            "git log --grep fetch",
            "git rebase -x 'cargo test' main",
            "git log --format='%H {a,b} $(x)'",
            "git --git-dir \"$GD\" status",
            "git commit -m \"$(cat msg.txt)\"",
            "git rebase -x 'cargo test --workspace' main",
            "git commit --message='fix pull'",
            "git log --format='%s update' -S fetch",
            "git commit -q -F - <<'EOF'\nfix: don't fetch when 'git pull' runs\n* list item\nEOF",
            "git remote -v",
            "git -C t2 remote -v update",
            "git commit -m 'fix fetch and pull'",
            "git --work-tree=../t2 fetch",
            "bash -c 'echo $HOME'",
            "git log --oneline -- docs/git-push.md",
            "timeout 5 ls *.rs",
            "git commit -m 'note GIT_CONFIG_GLOBAL and HOME' && git fetch origin",
            "git fetch --all",
            "git fetch -pP origin",
            "git fetch --prune origin",
            "git pull",
            "git show {1..100}",
            "git diff {1..100}",
            "git rev-parse {1..100}",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    #[test]
    fn merge_lock_expands_sequences_and_braces_before_runners() {
        const MAIN: &str = "push to the default branch";
        const MOVE: &str = "moving the default branch";
        let cases = [
            ("{g..g}it push origin HEAD:main", MAIN),
            ("g{i..i}t push origin HEAD:main", MAIN),
            ("{g..g}h pr merge 1", "pr merge"),
            ("{g..g}{i..i}{t..t} push origin HEAD:main", MAIN),
            ("{git,{a..z}{a..z}} push origin HEAD:main", UNREAD_BRACE),
            ("sudo {a..z}{a..z} push origin HEAD:main", UNREAD_BRACE),
            ("sudo {-E,} git push origin HEAD:main", MAIN),
            ("command {-p,} git push origin HEAD:main", MAIN),
            ("nice {-n,} 5 git push origin HEAD:main", MAIN),
            ("git branch -cf x main", MOVE),
            ("git branch -c -f x main", MOVE),
            ("git branch --copy --force x main", MOVE),
            ("git branch -mf x main", MOVE),
            ("git branch --move --force x main", MOVE),
            ("git checkout -Bmain x", MOVE),
            ("git switch -Cmain x", MOVE),
            ("git branch -f main", MOVE),
            ("git branch -f main feat/x", MOVE),
            (
                "git fetch origin 'refs/heads/*:refs/heads/*'",
                "a fetch into",
            ),
            ("git fetch origin '+refs/*:refs/*'", "a fetch into"),
            ("git fetch origin 'x/*:*'", "a fetch into"),
        ];
        for (cmd, why) in cases {
            let got = bash(cmd, "main").unwrap_or_default();
            assert!(got.contains(why), "{cmd}: {got:?}");
        }
        for cmd in [
            "for i in {1..3}; do echo $i; done",
            "echo {1..100}",
            "git branch -f main origin/main",
            "git checkout -B main origin/main",
            "git switch -C main refs/remotes/origin/main",
            "git fetch origin 'refs/pull/*/head:refs/pull/*/head'",
            "git fetch origin 'refs/notes/*:refs/notes/*'",
            "git fetch origin 'refs/tags/*:refs/tags/*'",
            "git fetch origin '+refs/heads/*:refs/remotes/origin/*'",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
    }

    #[test]
    fn merge_lock_refuses_pushes_driven_by_config_send_pack_and_subtree() {
        for cmd in [
            "git send-pack origin main",
            "git send-pack --thin git@github.com:o/r.git HEAD:refs/heads/main",
            "git send-pack origin",
            "git send-pack --stdin origin",
            "git send-pack --all origin",
            "git send-pack --force origin feature",
            "git subtree push --prefix=lib origin main",
            "git subtree push -P lib origin refs/heads/main",
            "git subtree push -P lib origin HEAD:main",
            "git subtree push -P lib origin",
            "git subtree push -P lib origin v1.2.3",
            "git subtree push -P lib origin $B",
            "git -c remote.origin.push=HEAD:main push",
            "git -c remote.origin.push=refs/heads/x:refs/heads/x push origin",
            "git -c Remote.Origin.Push=x push origin x",
            "git --config-env=remote.origin.push=P push",
            "git -c remote.origin.mirror=true push origin",
            "git -c push.default=matching push",
            "git -c push.default=upstream push origin",
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.push GIT_CONFIG_VALUE_0=HEAD:main git push",
            "GIT_CONFIG_PARAMETERS=\"'push.default=matching'\" git push",
            "git config remote.origin.push HEAD:main && git push",
            "git config remote.origin.push refs/heads/x:refs/heads/x",
            "git config --add remote.origin.push HEAD",
            "git config set remote.origin.push HEAD:main",
            "git config remote.origin.mirror true",
            "git config push.default matching",
            "git config --global push.default upstream",
            "git remote add --mirror=push backup https://x/y.git",
            "git -c 'push.default'=matching push",
            "git -c 'remote.origin.push'=HEAD:main push",
            "git config \"remote.origin.push\" HEAD:main && git push",
            "git config remote.origin.pu''sh HEAD:main",
            "git config push.\"default\" matching",
            "export GIT_CONFIG_PARAMETERS=\"'remote.origin.push=HEAD:main'\"; git push",
            "GIT_CONFIG_GLOBAL=/tmp/g git push",
            "HOME=/tmp/h git push origin feature",
            "git -c include.path=/tmp/g push origin feature",
            "git push origin HEAD:heads/main",
            "git send-pack origin feature:heads/main",
            "git send-pack --std origin feature",
            "git push --mir origin",
            "git push --al origin",
            "git push --del origin feature",
            "printf '[remote \"origin\"]\\n\\tpush = HEAD:refs/heads/main\\n' >> .git/config && git push origin",
            "echo x | tee -a .git/config && git push origin",
            "sed -i 's/a/b/' .git/config && git push",
            "cp /tmp/c .git/config && git push origin feature",
            "cd .git && echo x >> config && cd .. && git push",
            "python3 -c 'open(\".git/config\",\"a\")' && git push",
            "export GIT_CONFIG_GLOBAL=/tmp/g; git push origin feature",
            "env XDG_CONFIG_HOME=/tmp/x git push origin feature",
            "git -c includeIf.onbranch:feature.path=/tmp/g push origin feature",
            "git config include.path /tmp/g && git push",
            "git push --force-w origin feature",
            "git push --force-i origin feature",
            "f=.gi; echo x >> ${f}t/config && git push origin feature",
            "echo x >> .g?t/c?nfig && git push origin feature",
            ". envfile && git push origin feature",
            "source envfile; git push",
            "eval \"$(cat envfile)\"; git push origin feature",
            "cargo test && git push -u origin feature",
            "git push -u origin feature 2>/dev/null",
            "git push -u origin feature | sh",
            "git push -u origin feature | tail -3 | sh",
            "bash -c 'git push origin feature'",
            "\\git push origin feature:main",
            "\"git\" push origin feature:main",
            "g''it push origin feature:main",
            "git \"push\" origin feature:main",
            "git 'pu''sh' origin feature:main",
            "/usr/bin/git push origin feature:main",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        for cmd in [
            "git send-pack origin feature",
            "git send-pack origin HEAD:refs/heads/feature",
            "git subtree push --prefix=lib origin feature",
            "git subtree push -P lib -m 'split lib' origin lib-split",
            "git subtree split -P lib -b main-lib",
            "git -c push.default=current push",
            "git -c push.default=simple push origin",
            "git -c user.name=x push -u origin feature",
            "git config --get remote.origin.push",
            "git config --get-regexp 'remote.*push'",
            "git config --unset-all remote.origin.push",
            "git config push.default current",
            "git config user.name x",
            "git remote add upstream https://x/y.git",
            "git config remote.origin.push",
            "git config --type=bool remote.origin.mirror",
            "GIT_CONFIG_NOSYSTEM=1 git commit -m 'remote. handling for .push'",
            "git push origin feature:heads/feature",
            "git push --set-upstream origin feature",
            "CARGO_HOME=/tmp/ch cargo build && git push -u origin feature:feature",
            "export RUSTUP_HOME=/x; git push -u origin feature:feature",
            "git commit -m 'fix home= parsing' && git push -u origin feature:feature",
            "git commit -m 'document include.path' && git push origin feature:feature",
            "git log --grep=HOME= && git push -u origin feature:feature",
            "git commit -m 'see .git/config' && git push -u origin feature:feature",
            "cat .git/config && git push origin feature:feature",
            "timeout 60 git push origin feature:feature",
            "git push origin @",
            "git push -u origin feature 2>&1 | tail -3",
            "git push origin feature | head -n 5",
            "/usr/bin/git push origin feature:feature",
        ] {
            assert_eq!(bash(cmd, "feature"), None, "{cmd}");
        }
        assert!(bash("git send-pack origin HEAD", "main").is_some());
        assert!(bash("git push origin @", "main").is_some());
        let why = bash("true && git push -u origin claude/x", "feature").unwrap();
        assert!(why.contains("`claude/x:claude/x`"), "{why}");
    }

    /// The guard cannot parse these unquoted heredoc bodies exactly (a `)` inside a quote, a
    /// comment, a nested heredoc or a `case` pattern; a delimiter joined by a backslash-newline),
    /// so each is refused although bash runs the label write.
    #[test]
    fn label_lock_refuses_unquoted_heredoc_bodies_it_cannot_parse() {
        let g = "gh issue edit 1 --add-label bosun:state/merged";
        for body in [
            format!("$(true # )\n{g}\n)"),
            format!("$(echo \")\"; {g})"),
            format!("$(echo ')'; {g})"),
            format!("$(cat <<X\n)\nX\n{g})"),
            format!("$(true;# )\n{g})"),
            format!("$(case x in a) {g};; esac)"),
            format!("`echo \\`{g}\\``"),
            format!("EO\\\nF\n{g}"),
        ] {
            let cmd = format!("cat <<EOF\n{body}\nEOF");
            assert!(bash(&cmd, "x").is_some(), "{cmd}");
        }
    }

    #[test]
    fn label_lock_treats_prose_flag_heredocs_as_data() {
        for cmd in [
            "git commit -m \"$(cat <<'EOF'\nfix bosun:state/review label\nEOF\n)\"",
            "git commit -m \"$(cat <<'EOF'\nfix bosun:state/review `gh issue view 1`\nEOF\n)\"",
            "git commit -m \"$(cat <<-'EOF'\n\tfix bosun:state/review\n\tEOF\n)\"",
            "git commit --message=\"$(cat <<'EOF'\nbosun:state/review\nEOF\n)\"",
            "gh pr create --body='about bosun:state/x'",
            "gh pr create --title t --body \"$(cat <<'EOF'\nfix bosun:state/review\n\nmore\nEOF\n)\"",
            "gh issue comment 3 --body \"$(cat <<'EOF'\nbosun:state/review\nEOF\n)\"",
            "git commit -m 'docs: bosun:state/review'",
            "gh pr create --body 'about bosun:state/review' --title \"t\"",
        ] {
            assert_eq!(bash(cmd, "x"), None, "{cmd}");
        }
        for cmd in [
            "gh pr edit 3 --add-label \"$(cat <<'EOF'\nbosun:state/x\nEOF\n)\"",
            "gh pr create --body \"$(cat <<'EOF'\nx\nEOF\n)\" --label bosun:state/x",
            "gh issue edit 3 --add-label bosun:state/x",
            "git commit -m \"$(cat <<'EOF'\nx\nEOF\n)\" && gh issue edit 3 --add-label bosun:state/x",
            "git commit -m \"$(cat <<EOF\nbosun:state/x $USER\nEOF\n)\"",
            "git commit -m \"$(cat <<'EOF'\nbosun:state/x\nEOF\n) $(gh label create bosun:state/y)\"",
            "git commit -m \"$(cat <<'EOF'\nbosun:state/x\nEOF\n; gh label create bosun:state/y)\"",
            "git commit -m \"$(sh <<'EOF'\ngh label create bosun:state/y\nEOF\n)\"",
            "git commit -m \"bosun:state/x $(gh label create bosun:state/y)\"",
            "git commit -m 'x' --add-label 'bosun:state/y'",
            "FOO=1 git -C sub commit -m 'bosun:state/x'",
            "gh -R o/r issue comment 3 --body 'bosun:state/x'",
            "git -c core.editor=sh commit -e -m 'gh issue edit 5 --add-label bosun:state/x'",
            "GIT_EDITOR=sh git commit -e -m 'gh issue edit 5 --add-label bosun:state/x'",
            "git -c core.hooksPath=/tmp/h commit -m 'bosun:state/x'",
            "git --config-env=core.editor=ED commit -m 'bosun:state/x'",
            "git commit -e -m 'bosun:state/x'",
            "git commit -t tmpl -m 'bosun:state/x'",
            "git commit -F f -m 'bosun:state/x'",
            "git commit --cleanup=x -m 'bosun:state/x'",
            "git commit --no-verify -m 'bosun:state/x'",
            "gh pr create -F f -t 'bosun:state/x'",
            "git commit -m \"$(cat <<'EOF'\nx\nEOF)\"; gh issue edit 3 --add-label bosun:state/x\nEOF\n)\"",
            "git commit -m 'bosun:state/x' && gh issue edit 3 --add-label \"$(git log -1 --format=%s)\"",
            "git commit -m 'bosun:state/x' && gh api repos/o/r/issues/3/labels -f labels[]=\"$(git log -1 --format=%s)\"",
            "git -c alias.y='!sh' y tag -m 'bosun:state/x'",
            "gh pr create --body \"$(cat <<'EOF'\nbosun:state/x\nEOF\n)\" --add-label foo",
            "git commit -m 'bosun:state/x'; gh label create bosun:state/y",
            "echo hi # ; git commit -m '\ngh issue edit 3 --add-label bosun:state/x #'",
            "cat <<'X'\ngit commit -m '\nX\ngh issue edit 3 --add-label bosun:state/x #'",
            "echo x # git commit -m \"$(cat <<'EOF'\nbosun:state/x\nEOF\n)\"\ngh issue edit 3 --add-label bosun:state/x",
            "echo $'\\'' ; git commit -m '\ngh issue edit 3 --add-label bosun:state/x #'",
            "G=gh; git commit -m 'bosun:state/x'; $G issue create -l y",
            "G=gh\ngit commit -m 'bosun:state/x'\n$G issue create -l y",
            "git commit -m 'bosun:state/x' # x",
            "git commit -m 'bosun:state/x' && gh issue edit 3 --add-label y",
            "git commit -m 'bosun:state/x' > x",
            "git commit -m 'bosun:state/x' && gh issue create -t x -b y -l \"$(git log -1 --format=%s)\"",
            "git commit -m 'bosun:state/x' && gh issue create -t x -b y --label=\"$(git log -1 --format=%s)\"",
            "git commit -m \"$(cat <<'EOF'\nx\nEOF)\"; rm bosun:state/y\nEOF\n)\"",
        ] {
            assert!(bash(cmd, "x").is_some(), "{cmd}");
        }
    }

    #[test]
    fn label_lock_treats_unquoted_heredoc_prose_as_data() {
        for cmd in [
            "cat >> docs/x.md <<EOF\nThe bosun:state/ready label (see `bosun status`) is set by bosun.\nEOF",
            "cat >> docs/x.md <<EOF\nbosun:state/ready costs $ 5; a | b\nEOF",
            "cat >> docs/x.md <<EOF\nbosun:state/ready: $(bosun status)\nEOF",
            "cat <<-EOF > docs/x.md\n\tbosun:state/ready \\`gh issue edit 1 --add-label bosun:state/x\\`\n\tEOF",
        ] {
            assert_eq!(bash(cmd, "x"), None, "{cmd}");
        }
        for cmd in [
            "cat <<EOF\nbosun:state/ready `gh issue edit 1 --add-label bosun:state/merged`\nEOF",
            "cat <<EOF\nprose $(gh issue edit 1 --add-label bosun:state/merged) prose\nEOF",
            "cat <<EOF\nbosun:state/x ${y:-$(gh issue edit 1 --add-label bosun:state/merged)}\nEOF",
            "cat <<EOF\nbosun:state/x $USER\nEOF",
            "cat <<EOF\nbosun:state/x `unterminated\nEOF",
            "cat <<EOF\nbosun:state/x $(gh label create bosun:state/x\nEOF",
        ] {
            assert!(bash(cmd, "x").is_some(), "{cmd}");
        }
    }

    #[test]
    fn read_only_github_tools_skip_the_label_lock() {
        let call = |tool: &str, input: Value| tool_rules(tool, &input, &|_| None, "main");
        let ready = json!({"labels": ["bosun:state/ready"], "state": "OPEN"});
        for tool in [
            "mcp__github__list_issues",
            "mcp__github__search_issues",
            "mcp__github__get_label",
            "mcp__github__issue_read",
            "mcp__github__pull_request_read",
        ] {
            assert_eq!(call(tool, ready.clone()), None, "{tool}");
        }
        // The list is pinned: every name is skipped, an unknown one is judged like a write.
        for name in READ_ONLY_GITHUB {
            let tool = format!("mcp__github__{name}");
            assert_eq!(call(&tool, ready.clone()), None, "{tool}");
        }
        let expected = [
            "actions_get",
            "actions_list",
            "get_check_run",
            "get_commit",
            "get_copilot_job_status",
            "get_file_contents",
            "get_job_logs",
            "get_label",
            "get_latest_release",
            "get_me",
            "get_release_by_tag",
            "get_tag",
            "get_team_members",
            "get_teams",
            "issue_read",
            "list_branches",
            "list_commits",
            "list_issue_fields",
            "list_issue_types",
            "list_issues",
            "list_pull_requests",
            "list_releases",
            "list_repository_collaborators",
            "list_tags",
            "pull_request_read",
            "search_code",
            "search_commits",
            "search_issues",
            "search_pull_requests",
            "search_repositories",
            "search_users",
        ];
        let mut have = READ_ONLY_GITHUB.to_vec();
        have.sort_unstable();
        assert_eq!(
            have, expected,
            "the read-only list changed: review the new tool first"
        );
        for tool in [
            "mcp__github__get_new_thing",
            "mcp__github__list_x",
            "mcp__github__search_x",
            "mcp__github__x_read",
            "mcp__github__issue_write",
            "mcp__github__update_pull_request",
            "mcp__github__create_label",
            "mcp__github__actions_run_trigger",
        ] {
            assert!(call(tool, ready.clone()).is_some(), "{tool}");
        }
        assert!(call("mcp__github__merge_pull_request", json!({})).is_some());
    }

    #[test]
    fn label_lock_judges_writes_not_mentions() {
        for cmd in [
            "grep -rn 'bosun:state/' src",
            "git show HEAD:x | grep bosun:state/review",
            "cat > docs/x.md <<'EOF'\nThe bosun:state/review label ...\nEOF",
            "gh issue list --label bosun:state/review",
            "gh pr view 3 --json labels | grep bosun:state/",
            "gh search issues bosun:state/review",
            "gh label list --search bosun:state/",
            "grep -r bosun:state/ docs",
            "bosun claim 3 # bosun:state/claimed",
        ] {
            assert_eq!(bash(cmd, "x"), None, "{cmd}");
        }
        for cmd in [
            "gh issue edit 3 --add-label bosun:state/merged",
            "gh pr edit 3 --remove-label bosun:state/review",
            "gh api repos/o/r/issues/3/labels -f labels[]=bosun:state/merged",
            "curl -X POST https://api.github.com/repos/o/r/issues/3/labels -d '{\"labels\":[\"bosun:state/merged\"]}'",
            "L=bosun:state/merged; gh issue edit 1 --add-label \"$L\"",
            "echo bosun:state/x | xargs gh issue edit 1 --add-label",
            "echo bosun:state/x | xargs -n1 gh issue edit 1 --add-label",
            "FOO=1 sudo gh issue edit 1 --add-label bosun:state/x",
            "bash -c \"gh issue edit 1 --add-label bosun:state/merged\"",
            "$(gh issue edit 1 --add-label bosun:state/merged)",
            "python3 -c \"import x; x.f('bosun:state/merged')\"",
            "node -e \"f('bosun:state/merged')\"",
            "gh label create bosun:state/x",
            "gh issue create --label bosun:state/ready",
            "gh api graphql -f query='mutation{addLabelsToLabelable(input:{labelIds:[\"bosun:state/x\"]})}'",
        ] {
            assert!(bash(cmd, "x").is_some(), "{cmd}");
        }
    }

    /// The label-lock fails closed: a command that names a state label passes only when every
    /// command in it, at every nesting level, is on the read-only allowlist. MUST rows are the
    /// F13 review probes (every one the old rule refused that can write a label) and our own.
    /// Known misses, also missed by the old rule and accepted: a label split by quotes or
    /// escapes (`"bosun:"state/x`, `bosun:sta""te/x`, `bosun:state\/x`, `$'bosun:state\x2fx'`),
    /// a label built at run time (`printf 'bosun:%s/x' state`), and a label written to a file
    /// in one call and read in another (`echo bosun:state/x > l.txt`, later `$(cat l.txt)`).
    #[test]
    fn label_lock_allows_only_read_only_commands() {
        const MUST: &[&str] = &[
            "gh issue edit 1 --add-label bosun:state/merged",
            "gh pr edit 1 --remove-label bosun:state/merged",
            "gh issue create -t x --label bosun:state/merged",
            "gh pr create --label bosun:state/merged",
            "gh label create bosun:state/merged",
            "gh label edit bosun:state/merged --name y",
            "gh label delete bosun:state/merged --yes",
            "gh label clone o/r # bosun:state/merged",
            "gh api repos/o/r/issues/1/labels -f labels[]=bosun:state/merged",
            "gh api -X DELETE repos/o/r/issues/1/labels/bosun:state/merged",
            "gh api graphql -f query='mutation{addLabelsToLabelable(input:{labelIds:[\"bosun:state/merged\"]})}'",
            "gh api graphql -f query='mutation{removeLabelsFromLabelable(input:{labelIds:[\"bosun:state/merged\"]})}'",
            "gh api --method POST repos/o/r/issues/1 -f labels[]=bosun:state/merged",
            "gh api -X PATCH repos/o/r/issues/1 -f 'labels[]=bosun:state/merged'",
            "gh -R o/r issue edit 1 --add-label bosun:state/merged",
            "GH_TOKEN=x gh issue edit 1 --add-label bosun:state/merged",
            "env gh issue edit 1 --add-label bosun:state/merged",
            "sudo gh issue edit 1 --add-label bosun:state/merged",
            "command gh issue edit 1 --add-label bosun:state/merged",
            "echo bosun:state/merged | xargs gh issue edit 1 --add-label",
            "nohup gh issue edit 1 --add-label bosun:state/merged",
            "time gh issue edit 1 --add-label bosun:state/merged",
            "exec gh issue edit 1 --add-label bosun:state/merged",
            "/usr/bin/gh issue edit 1 --add-label bosun:state/merged",
            "./gh issue edit 1 --add-label bosun:state/merged",
            "\"gh\" issue edit 1 --add-label bosun:state/merged",
            "'gh' issue edit 1 --add-label bosun:state/merged",
            "\\gh issue edit 1 --add-label bosun:state/merged",
            "g\\h issue edit 1 --add-label bosun:state/merged",
            "g''h issue edit 1 --add-label bosun:state/merged",
            "timeout 30 gh issue edit 1 --add-label bosun:state/merged",
            "nice -n 5 gh issue edit 1 --add-label bosun:state/merged",
            "stdbuf -o0 gh issue edit 1 --add-label bosun:state/merged",
            "watch -n1 gh issue edit 1 --add-label bosun:state/merged",
            "find . -maxdepth 0 -exec gh issue edit 1 --add-label bosun:state/merged \\;",
            "awk 'BEGIN{system(\"gh issue edit 1 --add-label bosun:state/merged\")}'",
            "parallel gh issue edit 1 --add-label ::: bosun:state/merged",
            "script -qc 'gh issue edit 1 --add-label bosun:state/merged' /dev/null",
            "ssh localhost gh issue edit 1 --add-label bosun:state/merged",
            "git -c alias.x='!gh issue edit 1 --add-label bosun:state/merged' x",
            "make -f /dev/stdin <<< 'a:; gh issue edit 1 --add-label bosun:state/merged'",
            "npx gh-cli issue edit 1 --add-label bosun:state/merged",
            "php -r 'system(\"gh issue edit 1 --add-label bosun:state/merged\");'",
            "lua -e 'os.execute(\"gh issue edit 1 --add-label bosun:state/merged\")'",
            "pwsh -c 'gh issue edit 1 --add-label bosun:state/merged'",
            "fish -c 'gh issue edit 1 --add-label bosun:state/merged'",
            "busybox sh -c 'gh issue edit 1 --add-label bosun:state/merged'",
            "curl -X POST -H 'Authorization: token x' https://api.github.com/repos/o/r/issues/1/labels -d '[\"bosun:state/merged\"]'",
            "wget --post-data='[\"bosun:state/merged\"]' https://api.github.com/repos/o/r/issues/1/labels",
            "http POST api.github.com/repos/o/r/issues/1/labels labels:='[\"bosun:state/merged\"]'",
            "xh POST api.github.com/repos/o/r/issues/1/labels labels:='[\"bosun:state/merged\"]'",
            "curl -X PATCH https://api.github.com/repos/o/r/issues/1 -d '{\"labels\":[\"bosun:state/merged\"]}'",
            "python3 -c \"import os; os.system('gh issue edit 1 --add-label bosun:state/merged')\"",
            "node -e \"require('child_process').execSync('gh issue edit 1 --add-label bosun:state/merged')\"",
            "ruby -e 'system(\"gh issue edit 1 --add-label bosun:state/merged\")'",
            "perl -e 'system(\"gh issue edit 1 --add-label bosun:state/merged\")'",
            "deno eval 'new Deno.Command(\"gh\",{args:[\"issue\",\"edit\",\"1\",\"--add-label\",\"bosun:state/merged\"]}).outputSync()'",
            "bun -e 'Bun.spawnSync([\"gh\",\"issue\",\"edit\",\"1\",\"--add-label\",\"bosun:state/merged\"])'",
            "bash -c 'gh issue edit 1 --add-label bosun:state/merged'",
            "sh -c 'gh issue edit 1 --add-label bosun:state/merged'",
            "eval 'gh issue edit 1 --add-label bosun:state/merged'",
            "echo $(gh issue edit 1 --add-label bosun:state/merged)",
            "echo `gh issue edit 1 --add-label bosun:state/merged`",
            "(gh issue edit 1 --add-label bosun:state/merged)",
            "{ gh issue edit 1 --add-label bosun:state/merged; }",
            "if true; then gh issue edit 1 --add-label bosun:state/merged; fi",
            "for l in bosun:state/merged; do gh issue edit 1 --add-label $l; done",
            "while true; do gh issue edit 1 --add-label bosun:state/merged; break; done",
            "f(){ gh issue edit 1 --add-label bosun:state/merged; }; f",
            "f(){ gh issue edit 1 --add-label \"$1\"; }; f bosun:state/merged",
            "echo bosun:state/merged | gh issue edit 1 --add-label \"$(cat)\"",
            "true && gh issue edit 1 --add-label bosun:state/merged",
            "false || gh issue edit 1 --add-label bosun:state/merged",
            "true; gh issue edit 1 --add-label bosun:state/merged",
            "true\ngh issue edit 1 --add-label bosun:state/merged",
            "bash <<EOF\ngh issue edit 1 --add-label bosun:state/merged\nEOF",
            "bash <<< 'gh issue edit 1 --add-label bosun:state/merged'",
            "echo 'gh issue edit 1 --add-label bosun:state/merged' | bash",
            "echo 'gh issue edit 1 --add-label bosun:state/merged' | sh",
            "L=bosun:state/merged; gh issue edit 1 --add-label \"$L\"",
            "export L=bosun:state/merged; bash -c 'gh issue edit 1 --add-label \"$L\"'",
            "L=bosun:state/merged; bash -c \"gh issue edit 1 --add-label $L\"",
            "L=bosun:state/merged; eval \"gh issue edit 1 --add-label $L\"",
            "L=bosun:state/merged; echo $(gh issue edit 1 --add-label $L)",
            "L=bosun:state/merged; (gh issue edit 1 --add-label $L)",
            "L=bosun:state/merged bash -c 'gh issue edit 1 --add-label $L'",
            "L=bosun:state/merged; xargs gh issue edit 1 --add-label <<< $L",
            "L=bosun:state/merged; find . -maxdepth 0 -exec gh issue edit 1 --add-label $L \\;",
            "L=bosun:state/merged; timeout 9 gh issue edit 1 --add-label $L",
            "L=bosun:state/merged; \"gh\" issue edit 1 --add-label $L",
            "echo bosun:state/merged > /tmp/l; gh issue edit 1 --add-label $(cat /tmp/l)",
            "echo bosun:state/merged > /tmp/l\ngh issue edit 1 --add-label \"$(< /tmp/l)\"",
            "gh issue edit 1 --add-label $'bosun:state/merged'",
            // Our own probes: quoting desyncs, options that run programs, hidden commands.
            "echo \"`printf '\"'`\"; gh issue edit 1 --add-label bosun:state/merged",
            "echo \"$(printf ')\"')\"; gh issue edit 1 --add-label bosun:state/merged",
            "echo '`' `gh issue edit 1 --add-label bosun:state/merged`",
            "echo $'a\\'b'; gh issue edit 1 --add-label bosun:state/merged",
            "echo x # it's\ngh issue edit 1 --add-label bosun:state/merged",
            "echo '<<EOF'\ngh issue edit 1 --add-label bosun:state/merged\nEOF",
            "cat <<EOF\n$(gh issue edit 1 --add-label bosun:state/merged)\nEOF",
            "cat <<EOF\n`gh issue edit 1 --add-label bosun:state/merged`\nEOF",
            "cat <<'EOF'\nbosun:state/merged\nEOF\ngh issue edit 1 --add-label bosun:state/merged",
            "cat <<'EOF'\nbosun:state/merged\nE0F\ngh issue edit 1 --add-label bosun:state/merged",
            "grep () ( gh issue edit 1 --add-label \"$1\" ); grep bosun:state/merged",
            "grep () { gh issue edit 1 --add-label \"$1\"; }; grep bosun:state/merged",
            "./grep bosun:state/merged",
            "/tmp/x/cat bosun:state/merged",
            "PATH=/tmp/x; grep bosun:state/merged",
            "GIT_EXTERNAL_DIFF=./x git diff # bosun:state/merged",
            "PAGER=./x git log # bosun:state/merged",
            "git -c core.pager=./x log # bosun:state/merged",
            "git --exec-path=/tmp log # bosun:state/merged",
            "git grep -O./x bosun:state/merged",
            "git grep --open-files-in-pager=./x bosun:state/merged",
            "git diff --ext-diff # bosun:state/merged",
            "rg --pre ./x bosun:state/merged",
            "sort --compress-program=./x f # bosun:state/merged",
            "echo bosun:state/merged > /dev/tcp/127.0.0.1/80",
            "gh --repo o/r issue list # bosun:state/merged; gh issue close 1",
            "gh issue list --label bosun:state/review && gh issue close 1",
            "grep bosun:state/x f | xargs gh issue edit 1 --add-label",
            "cat <(gh issue edit 1 --add-label bosun:state/merged)",
            "grep x >(gh issue edit 1 --add-label bosun:state/merged)",
            "[[ x ]] && gh issue edit 1 --add-label bosun:state/merged",
            "case x in x) gh issue edit 1 --add-label bosun:state/merged;; esac",
            "! gh issue edit 1 --add-label bosun:state/merged",
            "coproc gh issue edit 1 --add-label bosun:state/merged",
            "alias g=gh; g issue edit 1 --add-label bosun:state/merged",
            "source x.sh # bosun:state/merged",
            ". x.sh # bosun:state/merged",
            "less '+!gh issue edit 1 --add-label bosun:state/merged' f",
            "bosun status $(gh issue edit 1 --add-label bosun:state/merged)",
            "ech\\\no x; gh issue edit 1 --add-label bosun:state/merged",
            "echo ${x:-$(gh issue edit 1 --add-label bosun:state/merged)}",
            // Second review: allowlisted commands that run programs, and expansions that do.
            "sort -S 1K --com=sh f # bosun:state/merged",
            "git grep -iOsh bosun:state/merged",
            "git grep --open-files-in-pa=sh bosun:state/merged",
            "X=-Osh; git grep $X bosun:state/merged",
            "X=--pre=sh; rg $X bosun:state/merged",
            "echo > ./--pre=sh; rg x * # bosun:state/merged",
            "x='a[$(gh issue edit 1 --add-label bosun:state/merged)]'; echo $[x]",
            "x='$(gh issue edit 1 --add-label bosun:state/merged)'; cat <<E\n${x@P}\nE",
            "x='$(gh issue edit 1 --add-label bosun:state/merged)'; cat <<E\n$x\nE",
            "git log --ext x # bosun:state/merged",
            "git log --textconv -p # bosun:state/merged",
            "git show {--ext-diff,HEAD} # bosun:state/merged",
            "git log * # bosun:state/merged",
            "git -C x log --output-directory=y # bosun:state/merged",
            "x=$(gh issue edit 1 --add-label bosun:state/merged)",
            "x=`gh issue edit 1 --add-label bosun:state/merged`",
            "grep \"$X\" f # bosun:state/merged",
            // Third review: git config written and used in the same command.
            "printf '[core]\\n\\tfsmonitor = sh' > ../h1/.gitconfig; HOME=../h1; git status # bosun:state/x",
            "printf '[core]\\n\\tfsmonitor = sh' >> .git/config; git status # bosun:state/x",
            "echo x > ~/.config/git/config; XDG_CONFIG_HOME=../h; git status # bosun:state/x",
            "printf -v PATH /tmp/x; grep bosun:state/x f",
            "echo '[core]' >>../h1/gitconfig # bosun:state/x",
        ];
        let mut wrong: Vec<String> = MUST
            .iter()
            .filter(|c| bash(c, "x").is_none())
            .map(|c| format!("allowed: {c:?}"))
            .collect();
        for cmd in [
            "grep -rn 'bosun:state/' src",
            "grep -r bosun:state/ docs",
            "git show HEAD:docs/spec.md | grep bosun:state/review",
            "git log --oneline -5 -- docs/spec.md | grep -c bosun:state/",
            "grep -l bosun:state/ docs/*.md",
            "git --no-pager -C sub log --oneline | grep -c bosun:state/",
            "cat > docs/x.md <<'EOF'\nThe bosun:state/review label is set by bosun pr.\nEOF",
            "cat > docs/x.md <<-\"EOF\"\n\tIt's bosun:state/review (see `bosun pr`); a | b; $(x)\n\tEOF\ngit diff --stat",
            "cat <<EOF > docs/x.md\nIt's bosun:state/review (see spec)\nEOF",
            "gh issue list --label bosun:state/review",
            "gh pr view 3",
            "gh pr view 3 --json labels | grep bosun:state/",
            "gh pr view 3 --json labels | jq -r '.labels[] | select(.name == \"bosun:state/review\") | .name'",
            "gh search issues label:bosun:state/review",
            "gh label list --search bosun:state/ 2>&1 | head -5",
            "bosun claim 3",
            "bosun claim 3 # bosun:state/claimed",
            "L=bosun:state/review; gh issue list --label bosun:state/review",
            "test -f x && echo 'bosun:state/review' || printf '%s\\n' none",
            "grep -c bosun:state/ docs/spec.md; wc -l docs/spec.md",
        ] {
            if let Some(why) = bash(cmd, "x") {
                wrong.push(format!("refused: {cmd:?}: {why}"));
            }
        }
        assert!(wrong.is_empty(), "{}:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// The label-lock parser never panics: a seeded xorshift glues tokens (quotes, heredocs,
    /// substitutions, separators, multibyte and zero-width chars) into a few thousand commands.
    #[test]
    fn the_bash_rules_never_panic() {
        const TOKENS: &[&str] = &[
            "bosun:state/x",
            "'",
            "\"",
            "`",
            "\\",
            "$(",
            ")",
            "(",
            "${",
            "}",
            "<<",
            "<<-",
            "'EOF'",
            "\"EOF\"",
            "EOF",
            "\n",
            "\r",
            ";",
            "&",
            "|",
            "#",
            ">&",
            "&>",
            ">|",
            "<",
            ">",
            "<<<",
            "$'",
            "é",
            "✓",
            "🦀",
            "\u{200d}",
            "👩\u{200d}💻",
            " ",
            " ",
            "\t",
            "gh",
            "issue",
            "edit",
            "list",
            "git",
            "-C",
            "log",
            "grep",
            "cat",
            "bosun",
            "cd",
            "x=1",
            "-",
            "\tEOF",
        ];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5000 {
            let len = (next() % 24) as usize;
            let cmd: String = (0..len)
                .map(|_| TOKENS[(next() % TOKENS.len() as u64) as usize])
                .collect();
            let caught = std::panic::catch_unwind(|| bash(&cmd, "x"));
            assert!(caught.is_ok(), "panicked on {cmd:?}");
        }
    }

    #[test]
    fn label_lock() {
        assert!(bash("gh issue edit 4 --add-label bosun:state/merged", "x").is_some());
        assert_eq!(bash("bosun claim 4 # bosun:state/claimed", "x"), None);
        let mcp = json!({"issue_number": 4, "labels": ["bosun:state/queued"]});
        assert!(tool_rules("mcp__github__issue_write", &mcp, &|_| None, "main").is_some());
        let other = json!({"issue_number": 4, "labels": ["bug"]});
        assert_eq!(
            tool_rules("mcp__github__issue_write", &other, &|_| None, "main"),
            None
        );
        // Prose that names a state label is not a label edit.
        let pr = json!({"title": "x", "body": "moves tasks to bosun:state/ready after merge"});
        assert_eq!(
            tool_rules("mcp__github__create_pull_request", &pr, &|_| None, "main"),
            None
        );
        // Label fields at any depth, and label tools on their whole input, are still refused.
        let nested = json!({"issue": {"Labels": [{"name": "bosun:state/merged"}]}});
        assert!(tool_rules("mcp__github__issue_write", &nested, &|_| None, "main").is_some());
        let tool = json!({"name": "bosun:state/merged"});
        assert!(tool_rules("mcp__github__update_label", &tool, &|_| None, "main").is_some());
        for t in [
            "mcp__github__merge_pull_request",
            "mcp__github__enable_pr_auto_merge",
        ] {
            let why = tool_rules(t, &json!({}), &|_| None, "main").unwrap();
            assert!(why.contains("D13"));
            assert!(why.contains("bosun pr --pr <n> --auto-merge"), "{why}");
        }
    }

    #[test]
    fn edit_lock_treats_leases_of_my_per_task_ids_as_mine() {
        let lease = |session: &str| Lease {
            issue: 19,
            session: session.into(),
            origin: "claimed".into(),
            expires: Timestamp(i64::MAX),
            touch: vec!["crates/**".into()],
            comment_id: 1,
        };
        // The hook sees the plain session `S`; its lease was taken under `S/19`.
        assert_eq!(edit_lock("crates/a.rs", true, &[lease("S/19")], "S"), None);
        // `S/19` and `S/20` still conflict; so do a stranger and a prefix lookalike.
        assert!(edit_lock("crates/a.rs", true, &[lease("S/19")], "S/20").is_some());
        assert!(edit_lock("crates/a.rs", true, &[lease("S2/19")], "S").is_some());
        assert!(edit_lock("crates/a.rs", true, &[lease("T/19")], "S").is_some());
    }

    #[test]
    fn edit_lock_uses_leases_of_other_sessions() {
        let lease = Lease {
            issue: 7,
            session: "other".into(),
            origin: "spawned".into(),
            expires: Timestamp(i64::MAX),
            touch: vec!["src/core/**".into()],
            comment_id: 1,
        };
        let leases = [lease];
        let why = edit_lock("src/core/wave.rs", true, &leases, "me").unwrap();
        assert!(
            why.contains("#7") && why.contains("bosun wait --issue 7"),
            "{why}"
        );
        assert_eq!(edit_lock("docs/x.md", true, &leases, "me"), None);
        // Brackets and braces are literal in an edited path, never a way past the lock.
        let app = Lease {
            touch: vec!["app/**".into()],
            ..leases[0].clone()
        };
        for p in ["app/[id]/page.tsx", "app/{team}/x.tsx", "app/(group)/a.tsx"] {
            assert!(
                edit_lock(p, true, std::slice::from_ref(&app), "me").is_some(),
                "{p}"
            );
        }
        assert_eq!(edit_lock("web/[id]/p.tsx", true, &[app], "me"), None);
        assert_eq!(edit_lock("src/core/wave.rs", true, &leases, "other"), None);
        assert_eq!(
            edited_path("Write", &json!({"file_path": "a.rs"})),
            Some("a.rs".to_string())
        );
        assert_eq!(edited_path("Read", &json!({"file_path": "a.rs"})), None);
    }

    #[rustfmt::skip]
    const DECRYPT_CASES: &[(&str, bool)] = &[
        ("sops -d secrets/prod.yaml", true), ("sops secrets/prod.sops.yaml", true),
        ("EDITOR=cat sops f", true), ("sops -decrypt f", true), ("sops -d=true f", true),
        ("sops -decrypt=true f", true), ("sops --decrypt=yes f", true),
        ("sops exec-env f cmd", true), ("sops rotate -i f", true), ("sops set f '[\"a\"]' 1", true),
        ("sops -e --decrypt f", true), ("bash -c 'sops -d secrets/prod.yaml'", true),
        ("sh -c \"age -d -i k f\"", true), ("\"sops\" -d x", true),
        ("/usr/bin/sops --decrypt x", true), ("env FOO=1 sops exec-env f cmd", true),
        ("sudo -u root sops -d f", true), ("x=$(sops -d f)", true), ("echo `sops -d f`", true),
        ("sops -d f | jq .", true), ("timeout 10 sops -d f", true), ("stdbuf -o0 sops -d f", true),
        ("watch sops -d f", true), ("doas sops -d f", true),
        ("find . -name '*.sops.yaml' -exec sops -d {} \\;", true),
        ("cat <<'EOF' | sh\nsops -d f\nEOF", true), ("cat <<'EOF' | bash -s\nsops -d f\nEOF", true),
        ("echo 'sops -d f' | sh", true), ("bash <<< 'sops -d f'", true),
        ("bash <<'EOF'\nsops -d f\nEOF", true), (": \"<<'X'\"\nsops -d f\nX", true),
        ("age -di k f", true), ("age --decrypt=1 f", true), ("age f", true),
        ("hoist secrets edit prod", true), ("hoist --root x secrets sync", true),
        ("cat ~/.config/sops/age/keys.txt", true), ("cat ~/.config/sops/age/*", true),
        ("cd ~/.config/sops; cat age/keys.txt", true), ("cat /proc/self/environ", true),
        ("cat /proc/1/environ | tr '\\0' '\\n' | grep SOPS", true), ("echo $SOPS_AGE_KEY", true),
        ("echo \"${SOPS_AGE_KEY}\"", true), ("cat $SOPS_AGE_KEY_FILE", true),
        ("printenv SOPS_AGE_KEY", true), ("env | grep SOPS", true), ("export -p | grep SOPS", true),
        ("bash -c \"bash -c \\\"sh -c 'sops -d f'\\\"\"", true), ("sops --version", false),
        ("sops --help", false), ("sops -e -i f.yaml", false), ("sops encrypt f.yaml", false),
        ("command -v sops", false), ("age -r age1abc -o f.age f", false), ("age --version", false),
        ("age-keygen -o $TMP/k.txt", false), ("hoist secrets check", false),
        ("hoist secrets exec --help", false),
        ("git commit -m 'docs: explain SOPS_AGE_KEY_FILE'", false),
        ("grep -rn SOPS_AGE_KEY_FILE src/", false), ("grep -rn 'sops -d' docs/", false),
        ("SOPS_AGE_KEY_FILE=tests/fixtures/k.txt cargo test", false),
        ("export SOPS_AGE_RECIPIENTS=age1abc", false), ("cargo test sops_decrypt_refused", false),
        ("set -e; ls", false), ("git config diff.sopsdiffer.textconv \"sops -d\"", true),
        ("git -c diff.s.textconv='sops -d' diff", true),
        ("git -c core.pager='sops -d f' log", true),
        ("LESSOPEN='|sops -d %s' less f.sops.yaml", true), ("man -P 'sops -d f' ls", true),
        ("gh alias set x '!sops -d f'", true), ("watch -n1 'sops -d f'", true),
        ("ssh host 'sops -d f'", true),
        ("python3 -c \"import subprocess;subprocess.run(['sops','-d','f'])\"", true),
        ("node -e \"require('child_process').execSync('sops -d f')\"", true),
        ("perl -e 'system(\"sops -d f\")'", true),
        ("git commit -m \"docs: sops -d is refused\"", false),
        ("gh pr create --title \"x\" --body \"refuses sops -d\"", false),
        ("git commit -m \"$(cat <<'EOF'\nsops/age $SOPS_AGE_KEY /proc/self/environ keys.txt\nEOF\n)\"", false),
        ("gh pr create --body \"$(cat <<'EOF'\nsops/age $SOPS_AGE_KEY /proc/self/environ keys.txt\nEOF\n)\"", false),
        ("sops --config .sops.yaml -e f.yaml", false), ("brew install sops age", false),
        ("apt-get install -y age sops", false), ("mv sops /usr/local/bin/sops", false),
        ("install -m 755 sops /usr/local/bin/sops", false),
        ("tar xzf age.tar.gz && mv age/age /usr/local/bin/", false),
        ("cargo nextest run sops decrypt", false), ("grep -rn '/proc/self/environ' src", false),
        ("npx sops -d f", true), ("nix run nixpkgs#sops -- -d f", true),
        ("flock /tmp/l sops -d f", true), ("strace -f sops -d f", true),
        ("git bisect run sops -d f", true), ("coproc sops -d f", true),
        ("case x in *) sops -d f;; esac", true),
        ("if false; then :; elif sops -d f; then :; fi", true),
        ("bosun main-triage --run \"sops -d f\"", true),
        ("bosun main-triage --run='sops -d f'", true),
        ("echo sops | xargs -I{} sh -c '{} -d f'", true), ("rg --pre sops x src", true),
        ("rg --pre=sops x src", true), ("grep x /proc/self/environ", true),
        ("grep -e x /proc/self/environ", true), ("rg pat src /proc/1/environ", true),
        ("gh api -f body=\"x sops -d y\" repos/o/r/issues", false),
        ("gh issue list -q \".[] | sops -d\"", false), ("bosun claim 3", false),
        ("rg --pre-glob '*.gz' x src", false), ("npm install sops age", false),
        ("cargo test sops", false), ("curl -o sops https://x/sops", false),
        ("env | grep -i sops", true), ("printenv | grep -i sops_age", true),
        ("cat ~/.config/*/age/keys.txt", true), ("cat ~/.config/s*/age/keys.txt", true),
        ("cat ~/.config/so\"ps/age/keys.txt\"", true), ("sops --age -e f", true),
        ("sops --age encrypt f", true), ("sops --unencrypted-suffix -e f", true),
        ("sops --age=age1abc -e f", false), ("sops --verbose -e f.yaml", false),
        ("awk 'BEGIN{print ENVIRON[\"SOPS_AGE_KEY\"]}'", true), ("jq -n 'env.SOPS_AGE_KEY'", true),
        ("jq -n '$ENV.SOPS_AGE_KEY'", true), ("jq -n 'env.HOME'", true),
        ("printenv | grep -i age", true), ("env | sort", true), ("set | grep -i age", true),
        ("declare -x | grep age", true), ("compgen -e | head", true), ("env > /tmp/e", true),
        ("export -p > /tmp/e", true), ("python3 -c \"import os;print(os.environ)\"", true),
        ("node -e \"console.log(process.env)\"", true), ("ruby -e 'p ENV[\"A\"]'", true),
        ("perl -e 'print $ENV{A}'", true), ("python3 -c \"import os;os.getenv('A')\"", true),
        ("p=S\"OPS_AGE_KEY\"; echo ${!p}", true), ("cat /proc/self/env*", true),
        ("tar cf x.tar ~/.config/s*/a", true), ("find ~ -name keys.txt", true),
        ("find / -name keys.txt -print", true), ("cat keys.txt ~/.config/x", true),
        ("FOO=SOPS_AGE_KEY_FILE true", true), ("hoist secrets --env prod edit", true),
        ("hoist secrets --env=prod edit", true), ("hoist secrets --repo x rotate-key", true),
        ("alias s=sops", true), ("alias d='age -d'", true), ("alias r=rage", true),
        ("go run github.com/getsops/sops/v3/cmd/sops@v3.8.1 -d f", true),
        ("cat > /tmp/x.sh <<'EOF'\nsops -d f\nEOF\nbash /tmp/x.sh", true),
        ("cat > /tmp/x <<'EOF'\nsops -d f\nEOF\n. /tmp/x", true),
        ("cat > x.py <<'EOF'\nsops -d f\nEOF\npython3 x.py", true),
        ("cat > x <<'EOF'\nsops -d f\nEOF\nsource x", true), ("hoist secrets --env prod check", false),
        ("alias ll='ls -l'", false), ("cat > d.md <<'EOF'\nsops -d f\nEOF\ncargo test", false),
        ("cargo test", false), ("cargo nextest run", false), ("grep -rn age src/", false),
        ("rg sops", false), ("git log --grep sops", false), ("ls ~/.config", false),
        ("npm run build", false), ("pnpm test -- age", false), ("python3 scripts/check.py", false),
        ("node --test", false), ("printenv HOME", false), ("export PATH=$PATH:/x", false),
        ("printenv", true), ("env", true), ("env -0", true), ("set", true), ("export -p", true),
        ("declare -p", true), ("declare -x", true), ("typeset -p", true), ("typeset -x", true),
        ("compgen -e", true), ("compgen -v", true), ("ps e", true), ("ps eww", true), ("ps -E", true),
        ("ps axe", true), ("echo ${!SOPS*}", true), ("v=SOPS_AGE_KEY; echo ${!v}", true),
        ("for i in \"${!pkgs[@]}\"; do echo \"package $i\"; done", false),
        ("echo \"${!prefix@}\" # usage", false), ("git grep SOPS_AGE_KEY", false),
        ("git log -S SOPS_AGE_KEY --oneline", false), ("git log --grep 'SOPS_AGE_KEY' --oneline", false),
        ("gh search code SOPS_AGE_KEY", false), ("sed -n s/SOPS_AGE_KEY//p f", true),
        ("gh api repos/o/r/environments/prod/secrets/SOPS_AGE_KEY", false),
        ("cat > x.sh <<'EOF'\nsops -d f\nEOF\nchmod +x x.sh && ./x.sh", true),
        ("cat > x.sh <<'EOF'\nsops -d f\nEOF\nbash x.sh", true),
        ("cat <<'EOF' > x.py\nsops -d f\nEOF\npython3 x.py", true),
        ("tee x.sh <<'EOF'\nsops -d f\nEOF\nsh x.sh", true),
        ("cat > docs/s.md <<'EOF'\nmention sops -d here\nEOF\nbash scripts/check.sh", false),
        ("cat > notes.md <<'EOF'\nsops\nEOF\ngit ls-files | xargs wc -l", false),
        ("hoist secrets --manifest secrets.toml edit", true), ("hoist secrets --keys A,B edit", true),
        ("hoist --env prod secrets sync", true), ("hoist secrets exec --env prod -- cargo test", false),
        ("python3 -c \"from os import environ\"", true), ("deno eval \"Deno.env.get('A')\"", true),
        ("node -e \"require('process').env\"", true), ("php -r 'echo getenv(\"A\");'", true),
        ("php -r 'echo $_SERVER[\"A\"];'", true), ("jq '.env' f.json", false), ("make ENV=prod", false),
        ("docker compose --env-file .env up", false), ("ps aux", false), ("ps -e", false),
        ("python3 -c 'import os; print(os.getcwd())'", false),
        ("node -e 'console.log(process.argv)'", false), ("cargo build --package age", false),
        ("git grep --open-files-in-pager='sops -d' x", true), ("git grep -O'sops -d' x", true),
        ("git grep -O sops x", true), ("git grep --open-files-in-pager=sops x", true),
        ("python3 -c \"from os import environ as e; print(e)\"", true),
        ("declare -p SOPS_AGE_KEY", true), ("git grep -n sops docs/", false),
        ("gh search code sops", false), ("git log --grep sops --oneline", false),
        ("python3 -m pytest tests/test_environ.py", false), ("declare -p arr", false),
        ("typeset -p x", false),
        ("git -C . grep -O'sops -d' x", true), ("git -c core.quotepath=off grep -O'sops -d' x", true),
        ("git --git-dir=.git grep --open-files-in-pager=sops x", true),
        ("git -C . grep SOPS_AGE_KEY", false), ("git --no-pager grep -n sops docs/", false),
        ("git -C . log --grep sops --oneline", false),
        ("grep -r AGE-SECRET-KEY ~/.config", true), ("grep -r SECRET-KEY-1 ~", true),
        ("rg -uu AGE-SECRET ~", true), ("cat ~/.config/sop[s]/age/key[s].txt", true),
        ("printf 'sops -d f\\n' > /tmp/x.sh; bash /tmp/x.sh", true),
        ("echo 'sops -d f' > x.sh; . x.sh", true),
        ("echo 'sops -d f' > x.sh && chmod +x x.sh && ./x.sh", true),
        ("perl -e 'print %ENV'", true), ("node -e 'Bun.env'", true), ("jq -n env", true),
        ("jq -n 'env'", true), ("git commit -m \"docs: AGE-SECRET-KEY lines are R-secrets\"", false),
        ("git commit -m \"$(cat <<'EOF'\nAGE-SECRET-KEY lines\nEOF\n)\"", false),
        ("cargo test age_secret", false), ("echo hi > out.txt; cat out.txt", false),
        ("cargo test > log.txt; bash scripts/check.sh", false), ("jq '.env' f", false),
        ("jq -r .env.name file", false),
        ("grep -rn AGE-SECRET-KEY src/", false), ("rg 'SECRET-KEY-1' .bosun/risk.toml", false),
        ("echo AGE-SECRET-KEY", true), ("sops --version 2>&1 | head -1", false),
        ("sops --version > /dev/null", false), ("age --version 2>/dev/null", false),
        ("sops --version > v.txt && cat v.txt", false), ("sops -d f > out.yaml", true),
        ("sops f 2>/dev/null", true),
        ("grep -r SECRET-KEY ~", true), ("grep -ri secret-key $HOME", true),
        ("rg SECRET-KEY ~/.config", true), ("grep -r x ~/.config/sops", true),
        ("grep -R SECRET-K ${HOME}", true), ("rg AGE-SE /root", false), ("ag age-sec /home/bob", false),
        ("grep --recursive SECRET-KEY $XDG_CONFIG_HOME", true), ("rg secret ~", false),
        ("grep -rn SECRET-KEY ~/.config/nvim", false), ("rg x ~/.config/sops/foo", true),
        ("grep -rn SECRET-KEY src/", false), ("rg secret ~/projects/x", false),
        ("grep -r TODO ~/.config/nvim", false), ("grep -r TODO ~", true),
        ("grep SECRET-KEY ~/notes.txt", false), ("rg AGE ~", false),
        ("bun -e 'console.log(Bun.env)'", true), ("bun -p 'Bun.env.HOME'", true),
        ("bun --eval 'console.log(process.env)'", true), ("bun --print 'Bun.env'", true),
        ("bun -e \"require('child_process').execSync('sops -d f')\"", true),
        ("grep -r AGE ~", true), ("grep -ri age ~/.config", true), ("grep -r . ~/.config", true),
        ("grep -r '' ~/.config", true), ("rg --hidden . ~/.config", true), ("rg -uu KEY ~", true),
        ("grep -rE 'SECRET.KEY' ~", true), ("rg x ~/.config", true), ("rg x .config", true),
        ("grep -r x ${XDG_CONFIG_HOME}", true), ("rg --hidden x $HOME", true), ("rg -. x ~", true),
        ("grep -r x /home", true), ("rg TODO ~", false), ("rg AGE src/", false),
        ("rg x ~/.config/nvim", false), ("grep -r x $XDG_CONFIG_HOME/nvim", false),
        ("bun -e \"Bun.spawnSync(['sops','-d','f'])\"", true), ("bun -p \"Bun.spawnSync(['sops','-d','f'])\"", true),
        ("grep -r x ~/.config/../.config", true), ("grep -r x ~/.config/./", true),
        ("grep -r x ~/./.config", true), ("grep -r x ~//.config", true),
        ("grep -d recurse x ~/.config", true), ("grep -drecurse x ~/.config", true),
        ("grep --directories=recurse x ~/.config", true), ("grep --directories recurse x ~/.config", true),
        ("grep -r x ~/.conf*", true), ("grep -r x ~/../bob", true),
        ("grep -ri secret ~/.config/sops-test", false), ("grep -r x ~/.config/sops/", true),
        ("bun test", false), ("bun run build", false), ("bun install", false),
        ("bun -e 'console.log(1)'", false),
        ("egrep -r AGE ~/.config", true), ("fgrep -r x ~", true), ("rgrep x ~/.config", true),
        ("rgrep x ~", true), ("zgrep -r x ~/.config", true), ("ugrep -r x ~", true),
        ("ugrep -R x ~/.config", true), ("ack x ~/.config", true), ("ack x ~", true),
        ("git grep --no-index x ~/.config", true), ("git grep --no-index -e x -- ~", true),
        ("git -C . grep --no-index -n x /root", true), ("egrep x ~/.config", false),
        ("fgrep -r x src/", false), ("rgrep TODO src", false), ("zgrep x f.gz", false),
        ("ugrep x f.txt", false), ("ugrep -r x src", false), ("ack TODO src", false),
        ("ack AGE-SECRET-KEY src/", false), ("git grep --no-index x src", false),
        ("git grep --no-index x", false), ("git grep -n x ~/.config", false),
        ("ack --pager='sops -d f' x", true), ("ugrep --pager='sops -d f' x f", true),
        ("grep -r AGE ..", true), ("rg x ../..", true), ("grep -R x ../", true),
        ("rg x src/../..", true), ("grep x ../f.txt", false), ("rg x ./src", false),
        ("grep -r x ~root", true), ("rg x ~root/.config", true), ("grep -r x ~bob/.config", true),
        ("rg x ~root", false), ("grep -r x ~bob/.config/nvim", false), ("rg x ~bob/src", false),
        ("grep -r x /etc/../root/.config", true), ("rg x /etc/../root/.config/./sops", true),
        ("grep -r x /etc/../home/bob", true), ("rg x /tmp/../../..", false),
        ("grep -r x /root/../tmp", false), ("rg x src/../src", false), ("grep -r x ~/../bob", true),
        ("ugrep --view='sops -d' x f", true), ("ugrep -Q --view='sops -d' x", true),
        ("ugrep --filter='*:sh -c \"sops -d %\"' x f", true), ("ack --pag='sops -d f' x", true),
        ("ack --page='sops -d f' x", true), ("ugrep --pag='sops -d f' x f", true),
        ("ACK_PAGER='sops -d f' ack x", true), ("ACK_OPTIONS=\"--pager='sops -d f'\" ack x", true),
        ("ack --pag 'sops -d f' x", true), ("ack 'a b' src", false), ("ack sops docs/", false),
        ("ugrep -e 'sops -d' src", false), ("ugrep --filter='pdf:pdftotext % -' x f", false),
        ("ugrep --filter='yaml:sops -d %' '' secrets.enc.yaml", true),
        ("ugrep --filter=\"yaml:age -d -i k %\" x f.age", true),
        ("grep -r x $PWD/..", true), ("rg x ${PWD}/../..", true), ("rg x \"$PWD/..\"", true),
        ("rg x /proc/self/cwd/../..", true), ("rg x ..", true), ("rg x ../.config", true),
        ("grep -r x ../bob/.config", true), ("grep -r x ../bosun", true),
        ("grep -rn foo ../other-repo/src", false), ("rg x ../bosun/src", false),
        ("git -C ~/.config grep --no-index -e x .", true), ("git -C ~ grep --no-index x", true),
        ("git --work-tree=/root grep --no-index x", true), ("git -C src grep --no-index x", false),
        ("ug -r x ~", true), ("pcregrep -r x ~/.config", true), ("pcre2grep -r x ~", true),
        ("ug --pager='sops -d f' x f", true), ("pcregrep x f.txt", false), ("ug x src", false),
        ("grep -r x ../home/bob", true), ("grep -r x ../../home/bob", true),
        ("rg -uu x ../../../home/user", true), ("grep -r x ../*/*", true),
        ("grep -r x ../../*/*", true), ("rg TODO ../bosun", false), ("rg x ../.claude", false),
        ("grep -r x /*", true), ("grep -r x /r*", true), ("grep -r x /ro?t", true),
        ("grep -r x /home/b*", true), ("grep -r x /tmp/*", true), ("grep -r x src/*", true),
        // A pager or view is judged as a command run on a file; a shell there runs the output.
        ("ugrep --view sops x f", false), ("ug --view=sops x f", true), ("ugrep --vi=sops x f", true),
        ("ugrep --pager sops x f", false), ("ugrep --pager=sops x f", true), ("ack --pager sops x", true),
        ("ACK_PAGER=sh ack x", true), ("ack --pager=bash x", true), ("ugrep --pager=sh x f", true),
        ("ugrep --view=zsh x f", true), ("ack --pager=/bin/sh x", true), ("ack --pag dash x", true),
        ("ACK_PAGER_COLOR=/usr/bin/bash ack x", true), ("ACK_OPTIONS=--pager=sh ack x", true),
        ("env ACK_PAGER=sh ack x", true), ("ugrep --pager='env bash' x f", true),
        ("ack --pager=less x", false), ("ugrep --pager='less -R' x f", false),
        ("ugrep --view=vim x f", false), ("ACK_PAGER=less ack x", false),
        ("ack --pager less TODO src", false),
        // git, GNU grep and ag take unique prefixes of long options.
        ("git grep --no-in x ~", true), ("git grep --no-ind x ~/.config", true),
        ("git grep --no-inde -e x -- ~", true), ("git -C ~ grep --no-ind x", true),
        ("git grep --no- x ~", false), ("git grep --no-ind x src", false),
        ("grep --rec x ~/.config", true), ("grep --recu x ~", true), ("grep --der x ~", true),
        ("grep --dereference-rec x ~/.config", true), ("grep --dir=recurse x ~/.config", true),
        ("grep --dire recurse x ~", true), ("grep --directories=rec x ~/.config", true),
        ("grep -d rec x ~", true), ("ag --hid x ~", true), ("ag --unres x ~", true),
        ("ag --u x ~", true), ("grep --dir=skip x ~", false), ("grep --rec x src", false),
        ("ag --hid x src", false), ("grep --directories=read x ~/.config", false),
        ("rg pattern src", false), ("grep -rn foo .", false), ("ugrep -rn x src", false),
        ("ack --type rust TODO src", false), ("grep --recursive foo src", false),
        // Over-denial: a sibling repo reads as a home.
        ("grep -rn foo ../other-repo/*", true), ("echo ack --pager=sh", true),
        // Review: expanded and unknown pagers, `-nd`, exported pagers, fallbackToNoIndex.
        ("ack --pager=$SHELL x", true), ("ack --pager=\"${SHELL}\" x", true),
        ("ack --pager=$0 x", true), ("ack --pager=`which sh` x", true),
        ("ack --pager=\"$(which sh)\" x", true), ("ugrep --view=\"$EDITOR\" x f", true),
        ("grep -nd recurse x ~/.config", true), ("grep -hd recurse x ~", true),
        ("ack --pager=mksh x", true), ("ack --pager=/bin/ash x", true), ("ack --pager=csh x", true),
        ("ack --pager=tcsh x", true), ("ugrep --pager=python3.12 x f", true),
        ("ack --pager=lua x", true), ("ugrep --view=tclsh x f", true), ("ack --pager=pwsh x", true),
        ("ack --pager='less -R' x", false), ("ugrep --view='nvim -R' x f", true),
        ("ack --pager=/usr/bin/bat x", true), ("grep -nd skip x ~", false),
        ("export ACK_PAGER=sh; ack x", true), ("declare -x ACK_PAGER=sh; ack x", true),
        ("export ACK_OPTIONS=--pager=sh; ack x", true), ("export ACK_PAGER=less; ack x", false),
        ("git -C ~ -c grep.fallbackToNoIndex=true grep x", true),
        ("git -c grep.fallbackToNoIndex=1 -C /root grep -e x", true),
        ("git -c grep.fallbackToNoIndex=true grep x src", false),
        // Re-review: shell syntax, strict pager and view allowlists, pager env, git env config.
        ("ack --pager='less|sh' x", true), ("ack --pager='less;sh' x", true),
        ("ack --pager='less && sh' x", true), ("ack --pager='less & sh' x", true),
        ("ACK_PAGER='less|sh' ack x", true), ("ack --pager='less\nsh' x", true),
        ("ack --pager=\"vim -c ':%!sh' -\" x", true), ("ack --pager=\"nvim '+%!sh' -\" x", true),
        ("ack --pager=\"emacs -l evil.el\" x", true), ("ack --pager=\"bat --pager=sh\" x", true),
        ("ack --pager=\"less +!sh\" x", true), ("LESS='+!sh' ack --pager=less x", true),
        ("ack --pager=\"code --install-extension x\" x", true), ("ack --pager=/tmp/less x", true),
        ("ugrep --view='vim -c x' x f", true), ("ack --pager=./less x", true),
        ("ack --pager=vim x", true), ("export LESSOPEN=x; ack x", true),
        ("env PAGER=sh ack x", true), ("BAT_PAGER=sh ug x f", true),
        ("ack --pager=/usr/bin/less x", false), ("ack --nopager TODO src", false),
        ("ack --pager='less -' x", false), ("ugrep --view=/usr/bin/nvim x f", false),
        ("GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=grep.fallbackToNoIndex GIT_CONFIG_VALUE_0=true git -C ~ grep x", true),
        ("git --config-env=grep.fallbackToNoIndex=X -C ~ grep x", true),
        ("git --config-env grep.FallbackToNoIndex=X -C ~ grep x", true),
        // Aliases, less key and output options, pager variables that pass the allowlist.
        ("ug+ --pager=sh x f", true), ("ugrep+ --pager=sh x f", true), ("ack-grep --pager=sh x", true),
        ("ack-grep x ~/.config", true), ("ugrep+ -r x ~", true), ("ack-grep TODO src", false),
        ("ack --pager='less -kevil' x", true), ("ack --pager='less -oout' x", true),
        ("ack --pager='less -Ofile' x", true), ("LESSKEY=evil ack --pager=less x", true),
        ("LESSSECURE=0 ack x", true), ("LESS=-k ack x", true),
        ("export PAGER=cat; ack TODO src", false), ("PAGER=cat ugrep x src", false),
        ("LESS=-R ack x", false), ("PAGER='less -R' ack x", false),
        // git grep on a key root falls back to no-index whatever the config says.
        ("git -C ~ grep x", true), ("git -C ~/.config grep -e x", true),
        ("GIT_CONFIG_PARAMETERS=\"'grep.fallbackToNoIndex=true'\" git -C ~ grep x", true),
        ("GIT_CONFIG_GLOBAL=/tmp/g git -C /root grep x", true), ("git -C / grep x", true),
        ("git config --global grep.fallbackToNoIndex true; git -C ~ grep x", true),
        ("git --work-tree=~ grep x", true), ("git -C .. grep x", true), ("git -C~ grep x", true),
        ("git grep x", false), ("git grep x src", false), ("git -C src grep x", false),
        ("git -C ../other-repo grep x", false), ("git -C ~ grep x src", false),
        ("git -C ~/projects/x grep x", false), ("git -C ../bosun grep x", false),
        ("git -C ../../root grep x", true), ("git -C ../../home/user grep x", true),
        ("git -C ../../home grep x", true),
        // A `:` root is pathspec magic (`:(exclude)x`, `:!x`, `:/`): it searches the base.
        ("git -C ~ grep x -- ':(exclude)foo'", true), ("git -C ~ grep x ':!foo'", true),
        ("git -C ~ grep --no-index x ':(top)'", true), ("git -C ~ grep x ':/'", true),
        ("git -C ~ grep x ':^foo'", true), ("git -C /root grep x ':(glob)*'", true),
        ("git grep --no-index x ':(exclude)y'", false), ("git grep x -- ':(exclude)vendor'", false),
        ("git -C src grep x ':!tests'", false), ("git -C ~ grep x -- src ':!src/t'", true),
        ("git -C ~ grep -eSECRET ':(top)' src", true), ("git -C ~ grep -eSECRET ':/' src", true),
        ("git -C ~ grep -fpat ':(top)' src", true), ("git -C ~ grep x src $':(top)'", true),
        ("git grep --no-index ':x' ~", true), ("git -C ~ grep ':x' src", true), ("GIT_ICASE_PATHSPECS=1 git -C ~ grep x -- .CONFIG", true),
        ("export GIT_GLOB_PATHSPECS=1; git -C ~ grep x src", true),
        ("env GIT_ICASE_PATHSPECS=1 git grep --no-index x src", true),
        ("env git grep --no-index x ~", true), ("env -i X=1 git -C ~ grep x", true),
        // `env -C dir` is a leading `git -C dir`; wrappers run the search tool after them.
        ("env -C ~ git grep x", true), ("env --chdir=~ git grep x", true),
        ("env --chdir ~ git grep x", true), ("env -C ~ git grep --no-index x", true),
        ("env -C~ git grep x", true), ("env -C src git grep x", false),
        ("command git grep --no-index x ~", true), ("nice git grep --no-index x ~", true),
        ("timeout 5 git grep --no-index x ~", true), ("nice -n 5 git -C ~ grep x", true),
        ("sudo git -C ~ grep x", true), ("nohup git -C ~ grep x", true),
        ("command grep -r x ~/.config", true), ("nice rg -uu x ~", true),
        ("timeout 5 grep -r x ~", true), ("xargs grep -r x ~", true),
        ("timeout -k 1 -s KILL 5 grep -r x ~", true), ("sudo -u bob nice -n5 grep -r x ~", true),
        ("stdbuf -oL ionice -c 3 setsid rg -uu x ~", true), ("xargs -I x -n 1 grep -r y ~", true),
        ("nice cargo test", false), ("timeout 60 cargo nextest run", false),
        ("command -v git", false), ("time rg pattern src", false), ("sudo apt-get install x", false),
        ("timeout 5 grep -r x src", false), ("xargs -n 1 grep -l x", false),
        // More runners, with their value options and positionals (`chrt 1`, `flock FILE`).
        ("doas grep -r x ~/.config", true), ("doas printenv", true),
        ("doas git grep --no-index x ~", true), ("doas -u bob -n grep -r x ~/.config", true),
        ("chrt 1 grep -r x ~/.config", true), ("chrt -f 1 printenv", true),
        ("chrt -T 5 -d 0 grep -r x ~/.config", true), ("taskset 1 grep -r x ~/.config", true),
        ("taskset -c 0 printenv", true), ("unbuffer grep -r x ~/.config", true),
        ("unbuffer printenv", true), ("unbuffer -p printenv", true),
        ("watch grep -r x ~/.config", true), ("watch -n1 printenv", true),
        ("watch -n 2 -d 'grep -r' x ~/.config", true), ("watch -x -q 3 grep -r x ~/.config", true),
        ("flock /tmp/l grep -r x ~/.config", true), ("flock /tmp/l printenv", true),
        ("flock -w 5 /tmp/l -c printenv", true), ("flock -E 3 /tmp/l rg -uu x ~", true),
        ("busybox grep -r x ~/.config", true), ("busybox env", true),
        ("strace -f grep -r x ~/.config", true), ("strace -o /tmp/t -e trace=open printenv", true),
        ("strace -ff -tt -s 99 -E X=1 grep -r x ~/.config", true), ("ltrace -o /tmp/t -n 2 printenv", true),
        ("strace --trace open grep -r x ~/.config", true), ("strace --signal none printenv", true),
        ("strace --status failed --kvm vcpu printenv", true), ("ltrace --library x printenv", true),
        // Optional values (`--decode-fds[=set]`) are judged as a flag and as taking the next word.
        ("strace --decode-fds all printenv", true), ("strace --decode-fds printenv", true),
        ("strace --quiet attach --timestamps grep -r x ~/.config", true),
        ("strace -c -f cargo build", false), ("strace --timestamps -f cargo test", false),
        ("unshare -r grep -r x ~/.config", true), ("unshare --map-root-user -S 0 printenv", true),
        ("unshare --mount=/tmp/m -G 0 grep -r x ~/.config", true), ("unshare -w ~ git grep x", true),
        ("unshare --wd=~ git grep x", true), ("unshare -R /jail git grep x", true),
        ("chroot / grep -r x ~/.config", true), ("chroot --userspec=1:1 /jail printenv", true),
        ("chroot /jail git grep x", true), ("caffeinate grep -r x ~/.config", true),
        ("caffeinate -t 9 -w 1 printenv", true), ("nice -n 5 doas grep -r x ~/.config", true),
        ("watch -n 2 git status", false), ("strace -f cargo test", false),
        ("flock /tmp/l cargo build", false), ("taskset -c 0 cargo test", false),
        ("doas apt-get install x", false), ("busybox ls", false), ("caffeinate -i cargo build", false),
        ("unshare -w src git grep x", false), ("chrt -p 1 1234", false), ("taskset -p 1234", false),
        // After a runner every word is still tried as a command (an unknown option may
        // take the next word), and a runner hides no env dump.
        ("sudo -R cp sops -d f", true), ("sudo --chroot cp sops -d f", true),
        ("sudo -h cp sops -d f", true), ("sudo -R tar sops -d f", true),
        ("sudo -R cargo test sops -d f", true), ("sudo -R echo sops -d f", true),
        ("sudo -R ack sops -d f", true), ("xargs --process-slot-var cp sops -d f", true),
        ("sudo -Z cp sops -d f", true), ("nice cargo test -- sops -d f", true),
        ("xargs -0 rm -f", false), ("nohup ./run.sh", false), ("nice ack sops src", true),
        ("nice printenv", true), ("command env", true), ("timeout 5 env", true),
        ("sudo printenv", true), ("nice printenv HOME", false), ("env -u X git status", false),
        ("env FOO=1 cargo test", false), ("sudo -R /x printenv", true),
        ("git -C ~ grep -ieSECRET .config src", true), ("git -C ~ grep -ifpat .config src", true),
        ("git --icase-pathspecs -C ~ grep x -- .CONFIG", true),
        ("git --glob-pathspecs grep --no-index x src", true),
        ("GIT_ICASE_PATHSPECS=1 git grep x src", false), ("git grep -n foo ':!tests'", false),
        ("git -C src grep x ':/'", false), ("git -C ../bosun grep foo", false),
        ("git -C $PWD/../../x grep x", true), ("git -C /proc/self/cwd/../../x grep x", true),
        ("git -C ${PWD}/../../x grep x", true), ("git -C \"$PWD\"/../../x grep x", true),
        // Two or more `..` and one segment may be a home (`../../user` from `~/repo/sub`).
        ("git -C ../../user grep x", true), ("git -C ../../../root grep x", true),
        ("git -C ../../bob grep -e x", true), ("git -C ../../../x grep x", true),
        ("git -C ../other-repo/src grep x", false), ("git -C ../../x/y grep x", false),
        // Vim and nvim init variables run commands in a ugrep view (`-Q`, `--query`, `--view`).
        ("VIMINIT=':!sh' ugrep --view=vim x f", true), ("EXINIT='!sh' ug --view=vi x f", true),
        ("export MYVIMRC=/tmp/e; ugrep --view=vim x f", true), ("GVIMINIT=x ug -Q x f", true),
        ("XDG_CONFIG_HOME=/tmp/c ugrep --view=nvim x f", true), ("VIMRUNTIME=/tmp ug -rQ x", true),
        ("EDITOR=sh ugrep --view x f", true), ("VISUAL='vim -c x' ug --view x f", true),
        ("EDITOR=sh ugrep --query x", true), ("EDITOR=vim ugrep --view x f", false),
        ("VIMINIT=x vim f", false), ("EDITOR=true ugrep x src", false),
        ("XDG_CONFIG_HOME=/tmp/x ugrep x src", false), ("VIMINIT=x ack x", false),
        // ugrep's pager and view take a value only with `=`; ack's pager takes the next word.
        ("ugrep --pager TODO src", false), ("ugrep --view TODO src", false), ("ug --view x f", false),
        ("ugrep --pag TODO src", false), ("ugrep --pager=sh x f", true), ("ug --view=zsh x f", true),
        ("ugrep --vi=sh x f", true), ("ugrep --pag=sh x f", true), ("ack --pager sh x", true),
    ];

    #[test]
    fn key_root_resolves_dot_dot_lexically() {
        for (root, want) in [
            ("/etc/../root/.config", 2),
            ("/etc/../root", 1),
            ("/root/../tmp", 0),
            ("/..", 1),
            ("a/../..", 2),
            ("../x", 1),
            ("~/../bob", 1),
            ("../other/src", 0),
            ("../home/bob", 1),
            ("../root/.config", 2),
            ("../*/*", 2),
            ("/r*", 2),
            ("/tmp/*", 0),
            ("../.config", 2),
            ("../a/sops", 2),
            ("$PWD/..", 2),
            ("${PWD}/../..", 2),
            ("/proc/self/cwd/../..", 2),
            ("~root", 1),
            ("~root/.config/", 2),
            ("~+/src", 0),
            (".", 0),
            ("./.config", 2),
            ("src/..", 0),
        ] {
            assert_eq!(key_root(root), want, "{root}");
        }
    }

    #[test]
    fn a_heredoc_body_line_that_starts_with_the_delimiter_is_judged_as_code() {
        // Bash ends the first heredoc at the plain `EOF` line, so the next line runs.
        for hidden in ["gh pr merge 3", "sops -d f", "echo x > src/a.rs"] {
            let cmd = format!("cat <<'EOF'\nEOF)\ncat <<'Z'\nEOF\n{hidden}\nZ");
            let (code, _) = split_heredocs(&cmd);
            assert!(code.contains(hidden), "{cmd}");
            let (text, _, _) = strip_data_heredocs(&cmd, 0);
            assert!(text.contains(hidden), "{cmd}");
        }
        for hidden in ["gh pr merge 3", "sops -d f"] {
            let cmd = format!("cat <<'EOF'\nEOF)\ncat <<'Z'\nEOF\n{hidden}\nZ");
            assert!(bash(&cmd, "x").is_some(), "{cmd}");
        }
    }

    #[test]
    fn label_lock_keeps_prose_when_any_word_edits_labels() {
        let s = "git commit -m 'bosun:state/x'";
        for tail in [
            "gh pr create -t t -b b -dl \"$(git log -1 --format=%s)\"",
            "gh issue create -t x -b y \"-l\" \"$(git log -1 --format=%s)\"",
            "gh issue create -t x -b y \\-l \"$(git log -1 --format=%s)\"",
            "gh -R o/r label edit bug --name \"$(git log -1 --format=%s)\"",
            "gh  label create \"$(git log -1 --format=%s)\"",
            "gh issue edit 3 --add-\"label\" \"$(git log -1 --format=%s)\"",
        ] {
            let cmd = format!("{s} && {tail}");
            assert!(bash(&cmd, "x").is_some(), "{cmd}");
        }
    }

    #[test]
    fn decrypt_lock_table() {
        let wrong: Vec<_> = DECRYPT_CASES
            .iter()
            .filter(|(cmd, deny)| bash(cmd, "feature").is_some() != *deny)
            .collect();
        assert!(wrong.is_empty(), "wrong verdict: {wrong:#?}");
        let why = bash("sops -d f", "feature").unwrap();
        assert!(!why.contains('\n') && why.contains("hoist secrets exec"));
    }

    /// A prose string literal in an interpreter's quoted heredoc is data; a literal that reads
    /// as a command (or holds shell syntax) is still judged, and so is a shell's heredoc.
    #[test]
    fn decrypt_lock_treats_prose_literals_in_interpreter_heredocs_as_data() {
        let py = |body: &str| format!("python3 - <<'EOF'\n{body}\nEOF");
        for body in [
            "s = \"CI decrypts with hoist, agents never do\"\nprint(s)",
            "msg = 'decrypting secrets is refused for agents, age key or not'\nprint(msg)",
            "print(\"Secrets stay encrypted: only the owner edits them, never sops.\")",
            "x = {'why': 'agents never decrypt with age, nor sops.'}",
        ] {
            assert_eq!(bash(&py(body), "feature"), None, "{body}");
        }
        for body in [
            "import subprocess\nsubprocess.run(['sops', '-d', 's.yaml'])",
            "import os\nos.system('sops -d s.yaml')",
            "import os\nos.system('sops --decrypt s.yaml > out')",
            "import os\nos.system('x=1 sops -d s.yaml')",
            "import os\nos.system('nice sops -d s.yaml')",
            "import os\nos.system('bash -c \"sops -d f\"')",
            "import os\nos.system('the docs say: run sops -d f')",
            "import os\nos.system('hoist secrets edit')",
            "import os\nos.system(\"cat ~/.config/sops/age/keys.txt\")",
            "s = 'unterminated sops -d f",
            "import os\nos.system('''sops -d f''')",
            // Env dumps and decrypt calls behind unknown wrappers are judged as commands.
            "import os\nos.system('ps eww 1')",
            "import os\nos.system('ps auxe')",
            "import os\nos.system('export -p')",
            "import os\nos.system('declare -x')",
            "import os\nos.system('typeset -p')",
            "import os\nos.system(\"compgen -v\")",
            "import os\nos.system('fakeroot sops a')",
            // Prose-shaped key reads: a key path or a search of home is never prose.
            "import os\nos.system('grep -r AGE ~/.config/sops')",
            "import os\nos.system('rg AGE ~/.config/sops')",
            "import os\nos.system('grep -rh KEY ~/.config/sops')",
            "import os\nos.system('grep -r AGE ~')",
            "import os\nos.system('cat ~/.config/sops/age/keys.txt')",
            // Code inside a literal (`exec("..")`) and any `hoist secrets` call are never prose.
            "from subprocess import run as r\nexec('r ((\"sops\", \"-d\", \"s.yaml\"))')",
            "from subprocess import run as r\nexec(\"r (('sops',  '-d',  's.yaml'))\")",
            "from subprocess import run as r\nexec(\"r (('sops',\t'-d',\t's.yaml'))\")",
            "import os\nos.system('fakeroot (sops) -d f')",
            // Paths and globs are never prose: a glob can name the key file.
            "import os\nos.system('fakeroot cat /root/.con*/so*/age/k*')",
            "msg = 'decrypting secrets is refused for agents (age key)'\nprint(msg)",
            "exec(\"print (__import__('subprocess').run(['sops','-d','s.yaml']))\")",
            "exec(\"import subprocess as s; s.run(['sops', '-d', 'f'])\")",
            "eval(\"print (run(['age', '-d', 'f']))\")",
            "import os\nos.system('hoist secrets exec -- env')",
            "import os\nos.system('hoist secrets exec -- printenv')",
            "getattr(__import__('os'), 'system')('fakeroot sops -d f')",
            "getattr(__import__(\"os\"), \"system\")(\"run so'ps' x\")",
            "eval(\"getattr(os, 'system')('fakeroot sops a')\")",
            "import os\nos.system('see `sops -d f` for the plain text')",
            // Quotes join and vanish in the shell: a quoted name is still the tool.
            "import os\nos.system(\"fakeroot 'sops' -d s.yaml\")",
            "import os\nos.system(\"fakeroot so'ps' -d s.yaml\")",
            "import os\nos.system(\"fakeroot 'age' -d -i k f\")",
            "import os\nos.system(\"fakeroot 'hoist' secrets edit\")",
            "import os\nos.system('fakeroot sops and')",
            "x = 'agents never decrypt with sops or age'",
        ] {
            assert!(bash(&py(body), "feature").is_some(), "{body}");
        }
        // The heredoc's own command decides, and a heredoc that writes a file stays judged.
        for cmd in [
            "cat <<'EOF' | python3 -c 'import os,sys; os.system(sys.stdin.read())'\nbash -c 'ps eww 1'\nEOF",
            "cat <<'EOF' | python3 -c 'import os,sys; os.system(sys.stdin.read())'\neval 'fakeroot sops a'\nEOF",
            "cat > r.sh <<'EOF' && python3 -V\nbash -c 'declare -x'\nEOF\nbash r.sh",
            "python3 - > x.sh <<'EOF'\nprint('export -p')\nEOF\nbash x.sh",
            "python3 - > x.sh <<'EOF'\nprint('fakeroot sops a')\nEOF\nbash x.sh",
        ] {
            assert!(bash(cmd, "feature").is_some(), "{cmd}");
        }
        // A shell's heredoc stays code, prose or not.
        assert!(
            bash(
                "bash <<'EOF'\necho 'agents never decrypt with sops'\nEOF",
                "feature"
            )
            .is_some()
        );
        assert!(bash("perl - <<'EOF'\nsystem('sops -d f');\nEOF", "feature").is_some());
        assert_eq!(
            bash(
                "node - <<'EOF'\nconsole.log('we never decrypt with sops')\nEOF",
                "feature"
            ),
            None
        );
        let node = "node - <<'EOF'\nconst {execFileSync: x} = require('child_process'); eval('x (\"sops\", (\"-d s.yaml\").split (\" \"))')\nEOF";
        assert!(bash(node, "feature").is_some());
    }

    fn own(issue: u64, session: &str, touch: &[&str]) -> Lease {
        Lease {
            issue,
            session: session.into(),
            origin: "claimed".into(),
            expires: Timestamp(i64::MAX),
            touch: touch.iter().map(|t| t.to_string()).collect(),
            comment_id: 1,
        }
    }

    #[test]
    fn edit_lock_keeps_edits_inside_the_own_touch() {
        let mine = [own(8, "me", &["src/core/**", "Cargo.toml"])];
        // Covered by a glob, or a lockfile companion.
        assert_eq!(edit_lock("src/core/a.rs", true, &mine, "me"), None);
        assert_eq!(edit_lock("Cargo.lock", true, &mine, "me"), None);
        // Fragments and new files are always free.
        assert_eq!(edit_lock("changelog.d/x.md", true, &mine, "me"), None);
        assert_eq!(edit_lock(".changeset/x.md", true, &mine, "me"), None);
        // The changesets README and config are not fragments.
        assert!(edit_lock(".changeset/README.md", true, &mine, "me").is_some());
        assert!(edit_lock(".changeset/config.json", true, &mine, "me").is_some());
        assert!(edit_lock(".changeset/sub/x.md", true, &mine, "me").is_some());
        assert_eq!(edit_lock("docs/progress.d/x.md", true, &mine, "me"), None);
        assert_eq!(edit_lock("src/new.rs", false, &mine, "me"), None);
        // An existing file outside the touch is denied, naming issue, globs and the fix.
        let why = edit_lock("src/cli/x.rs", true, &mine, "me").unwrap();
        assert!(
            why.contains("src/cli/x.rs is outside your lease (touch: src/core/**, Cargo.toml)")
                && why.contains("`bosun claim 8 --touch src/cli/x.rs`")
                && why.contains("exit 3"),
            "{why}"
        );
        // Several own leases are one union; the first names the issue.
        let two = [mine[0].clone(), own(9, "me", &["docs/**"])];
        assert_eq!(edit_lock("docs/a.md", true, &two, "me"), None);
        let why = edit_lock("web/a.ts", true, &two, "me").unwrap();
        assert!(why.contains("claim 8"), "{why}");
    }

    #[test]
    fn edit_lock_without_an_own_touch_allows() {
        assert_eq!(edit_lock("src/a.rs", true, &[], "me"), None);
        let epic = [own(3, "me", &[])];
        assert_eq!(edit_lock("src/a.rs", true, &epic, "me"), None);
        // Another session's lease does not make one of mine, and still denies.
        let other = [own(4, "other", &["src/**"])];
        assert!(edit_lock("src/a.rs", true, &other, "me").is_some());
        // An orchestrator holding only per-task wave leases is not restricted by them.
        let wave = [own(7, "me/x", &["src/a/**"]), own(8, "me/y", &["src/c/**"])];
        assert_eq!(edit_lock("src/b.rs", true, &wave, "me"), None);
    }

    #[test]
    fn edit_lock_bad_own_glob_covers_nothing() {
        let bad = [own(6, "me", &["src/**", "src/[a"])];
        assert_eq!(edit_lock("src/a.rs", true, &bad, "me"), None);
        let why = edit_lock("docs/a.md", true, &bad, "me").unwrap();
        assert!(why.contains("re-claim with valid globs"), "{why}");
        let only = [own(6, "me", &["src/[a"])];
        assert!(edit_lock("src/[a", true, &only, "me").is_some());
    }

    const H: &str = "0123456789abcdef0123456789abcdef01234567";

    fn chat_body() -> String {
        crate::core::approve::chat_comment(
            H,
            "session_x",
            "2026-11-10T00:00:00Z",
            Some("1-0123456-1"),
        )
    }

    fn mcp(tool: &str, input: Value) -> Option<String> {
        tool_rules(tool, &input, &|_| Some("feat".into()), "main")
    }

    #[test]
    fn mcp_comment_with_marker_denied() {
        let body = chat_body();
        let marker_only = format!("lgtm\n<!-- {} v1 -->", crate::core::approve::CHAT_MARKER);
        let form_only = format!("/bosun approve {} chat", H.to_uppercase());
        for (tool, input) in [
            (
                "add_issue_comment",
                json!({"issue_number": 3, "body": body}),
            ),
            (
                "update_issue_comment",
                json!({"comment_id": 9, "body": marker_only}),
            ),
            (
                "create_pull_request",
                json!({"title": "x", "body": form_only}),
            ),
            (
                "pull_request_review_write",
                json!({"method": "create", "body": body}),
            ),
            ("issue_write", json!({"method": "update", "body": body})),
            (
                "add_comment_to_pending_review",
                json!({"x": [{"body": body}]}),
            ),
        ] {
            let why = mcp(&format!("mcp__github__{tool}"), input)
                .unwrap_or_else(|| panic!("{tool} allowed"));
            assert!(why.contains("bosun approve <pr>@<head> --chat"), "{why}");
        }
        // Reading is not writing.
        let q = json!({"query": "bosun:chat-approval"});
        assert_eq!(mcp("mcp__github__search_issues", q), None);
    }

    #[test]
    fn plain_comment_allowed() {
        let ok = [
            "lgtm",
            "/bosun approve 0123456789abcdef0123456789abcdef01234567",
            "/bosun approve <head> chat",
            "chat approvals are recorded by the orchestrator",
        ];
        for body in ok {
            let input = json!({"issue_number": 3, "body": body});
            assert_eq!(mcp("mcp__github__add_issue_comment", input), None, "{body}");
            let cmd = format!("gh pr comment 3 --body '{body}'");
            assert_eq!(bash(&cmd, "feat"), None, "{cmd}");
        }
    }

    #[test]
    fn any_mcp_write_tool_with_the_form_is_denied() {
        let body = chat_body();
        let tool =
            |t: &str| tool_rules(t, &json!({"body": body}), &|_| Some("feat".into()), "main");
        assert!(tool("mcp__x__post").is_some());
        assert!(tool("mcp__other_server__create_comment").is_some());
        assert!(tool("mcp__x__get_thing").is_none());
        assert!(tool("mcp__x__search_things").is_none());
    }

    #[test]
    fn child_flag_tamper_is_caught() {
        for c in [
            "env -u CLAUDE_CODE_CHILD_SESSION bosun approve 3@h --chat",
            "unset CLAUDE_CODE_CHILD_SESSION; bosun approve 3@h --chat",
            "env -i bosun approve 3@h --chat",
            "CLAUDE_CODE_CHILD_SESSION=0 bosun status",
            "env --unset=CLAUDE_CODE_CHILD_SESSION /usr/bin/bosun status",
            "env -u 'CLAUDE_CODE_CHILD_SESSION' bos''un status",
            "cd /r && env -u CLAUDE_CODE_CHILD_SESSION bosun approve 3@h --chat",
            "env FOO=1 -i bosun status",
        ] {
            assert!(child_flag_tamper(c), "{c}");
        }
        for c in [
            "bosun status",
            "env FOO=1 bosun status",
            "unset FOO",
            "env -i ls",
            "grep -i x && bosun status",
            "grep -u x; bosun status",
            "ls - && bosun status",
            "grep -i CLAUDE_CODE_CHILD_SESSION notes && bosun status",
            "bosun status | grep -i x",
            "bosun comment unset CLAUDE_CODE_CHILD_SESSION",
        ] {
            assert!(!child_flag_tamper(c), "{c}");
        }
    }

    #[test]
    fn guard_denies_every_record_the_counter_counts() {
        use crate::core::approve::parse_chat_record;
        let body = chat_body();
        assert!(parse_chat_record(&body, H, None).is_some());
        assert!(carries_chat_form(&body));
        // Each line on its own carries the form: a record cannot be split past the guard.
        assert!(
            body.lines()
                .filter(|l| !l.contains("Approved"))
                .all(carries_chat_form)
        );
        let shouty = body.replace("bosun:chat-approval", "BOSUN:Chat-Approval");
        assert!(carries_chat_form(&shouty));
    }

    #[test]
    fn bash_gh_and_curl_marker_denied() {
        let body = chat_body().replace('\n', "\\n");
        let cmds = [
            format!("gh api repos/o/r/issues/3/comments -f body='{body}'"),
            format!("gh pr comment 3 --body \"{body}\""),
            format!("gh issue comment 3 -b '/bosun approve {H} chat'"),
            format!(
                "curl -X POST -H 'Authorization: token x' https://api.github.com/repos/o/r/issues/3/comments -d '{{\"body\":\"{body}\"}}'"
            ),
            format!(
                "gh api repos/o/r/issues/3/comments --input - <<'EOF'\n{{\"body\": \"/bosun approve {H} chat\"}}\nEOF"
            ),
            format!(
                "bash -c \"gh pr comment 3 -b '<!-- bosun:chat-approval v1 session=s head={H} -->'\""
            ),
            format!("bosun handoff-pr 3 --note '/bosun approve {H} chat'"),
            format!("git commit -m '/bosun approve {H} chat'"),
        ];
        for cmd in &cmds {
            let why = bash(cmd, "feat").unwrap_or_else(|| panic!("allowed: {cmd}"));
            assert!(why.contains("chat approval record"), "{cmd}: {why}");
        }
        // Read-only commands may name it.
        assert_eq!(bash("grep -rn bosun:chat-approval src", "feat"), None);
        assert_eq!(
            bash(
                "gh pr view 3 --json comments | grep 'bosun:chat-approval'",
                "feat"
            ),
            None
        );
    }

    #[test]
    fn bosun_approve_chat_is_recognised_for_the_orchestrator_check() {
        for cmd in [
            format!("bosun approve 12@{H} --chat"),
            format!("cd x && bosun approve 12@{H} --chat --question q"),
            format!("bash -c \"bosun approve 12@{H} --chat\""),
            format!("bos''un approve 12@{H} --chat"),
            format!("bo\\sun approve 12@{H} --chat"),
            format!("\"bo\"sun \"approve\" 12@{H} '--chat'"),
            format!("/usr/bin/bosun approve 12@{H} --chat"),
        ] {
            assert!(records_chat_approval(&cmd), "{cmd}");
            assert_eq!(bash(&cmd, "feat"), None, "{cmd}");
        }
        assert!(!records_chat_approval("bosun approve 12 --sign-key k"));
        assert!(!records_chat_approval("git log --chat"));
    }

    #[test]
    fn chat_approve_with_identity_assignment_is_flagged() {
        for cmd in [
            format!("BOSUN_ORIGIN=local bosun approve 1@{H} --chat"),
            format!("env BOSUN_SESSION=root bosun approve 1@{H} --chat"),
            format!("export CLAUDE_CODE_CHILD_SESSION=0; bosun approve 1@{H} --chat"),
            format!("BOSUN_ORIGIN='local' bos''un approve 1@{H} --chat"),
        ] {
            assert!(chat_identity_override(&cmd), "{cmd}");
        }
        assert!(!chat_identity_override(&format!(
            "bosun approve 1@{H} --chat"
        )));
        assert!(!chat_identity_override("BOSUN_ORIGIN=local bosun status"));
    }

    #[test]
    fn escaped_records_are_still_records() {
        let u = r"gh api repos/o/r/issues/3/comments -f body='\u002fbosun approve 0123456789abcdef0123456789abcdef01234567 chat'";
        let x = r"printf '\x3c!-- bosun:chat-approval v1 --\x3e' | gh pr comment 3 -F -";
        for cmd in [u, x] {
            assert!(
                bash(cmd, "feat").is_some_and(|w| w.contains("chat approval record")),
                "{cmd}"
            );
        }
    }

    #[test]
    fn bosun_approve_chat_chained_denied() {
        let tail = format!("gh api repos/o/r/issues/12/comments -f body='/bosun approve {H} chat'");
        for cmd in [
            format!("bosun approve 1@{H} --chat && {tail}"),
            format!("bosun approve 1@{H} --chat; {tail}"),
            format!("bosun approve 1@{H} --chat | {tail}"),
            format!("bosun approve 1@{H} --chat\n{tail}"),
            format!("bosun approve 1@{H} --chat --note \"$({tail})\""),
            format!("bosun approve 1@{H} --chat --note '/bosun approve {H} chat' `true`"),
        ] {
            assert!(bash(&cmd, "feat").is_some(), "allowed: {cmd}");
        }
    }
}
