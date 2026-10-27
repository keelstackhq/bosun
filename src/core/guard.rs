//! What the PreToolUse hook refuses (spec §4 edit hook, label-lock; D13 merge-lock). Pure: the
//! hook passes the tool call, the repo-relative path, the current branch and the live leases.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::core::glob;
use crate::core::lease::{Conflict, Lease};

const MERGE_FIX: &str = "agents never merge, tag or force-push (spec D13): open the PR with \
    `bosun pr <issue>` and let `bosun pr <issue> --auto-merge` hand it to GitHub after the gates";

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
    if !exists || path.starts_with("changelog.d/") || path.starts_with("docs/progress.d/") {
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
    if let Some(name) = tool.strip_prefix("mcp__github__") {
        if matches!(name, "merge_pull_request" | "enable_pr_auto_merge") {
            return Some(format!("`{tool}` is refused: {MERGE_FIX}"));
        }
        // Reading or listing sets no label, even when the filter names a state label. Only known
        // read-only tools are skipped; a tool not listed is judged below (fails closed).
        if READ_ONLY_GITHUB.contains(&name) {
            return None;
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
static MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<!--\s*bosun:(?:prereqs?|task|epic)(?:[\s{]|-->|$)").expect("marker regex")
});

/// MCP tools that write an issue body or comment, and the input fields that hold the text.
const ISSUE_TEXT_TOOLS: [&str; 4] = [
    "issue_write",
    "update_issue",
    "add_issue_comment",
    "update_issue_comment",
];

fn has_marker(text: &str) -> bool {
    MARKER.is_match(text)
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
fn writes_marker(command: &str, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && (shell_words(command)
            .split(|w| w == SEP)
            .any(|seg| writes_issue_text(seg) && has_marker(&seg.join(" ")))
            || nested(command).iter().any(|n| writes_marker(n, depth + 1)))
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
    let hit = match tool.strip_prefix("mcp__github__") {
        Some(name) => {
            ISSUE_TEXT_TOOLS.contains(&name)
                && ["body", "comment"]
                    .iter()
                    .filter_map(|k| input.get(k).and_then(Value::as_str))
                    .any(has_marker)
        }
        None if tool == "Bash" => {
            let command = input.get("command").and_then(Value::as_str).unwrap_or("");
            has_marker(command) && !read_only(command, 0) && writes_marker(command, 0)
        }
        None => false,
    };
    hit.then(|| {
        "bosun's markers (owner checklist, task and epic metadata) are written by bosun only \
         (bosun prereqs --post writes the checklist); fix the plan instead"
            .to_string()
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

/// Nested commands are checked first (merge-lock; the label-lock judges them too), to this depth.
const MAX_DEPTH: usize = 3;

fn bash_rules(command: &str, branch_at: BranchAt, default_branch: &str) -> Option<String> {
    decrypt_rule(command, 0)
        .or_else(|| check(command, branch_at, default_branch, 0))
        .or_else(|| (command.contains(STATE) && !read_only(command, 0)).then(label_reason))
}

fn check(command: &str, branch_at: BranchAt, default_branch: &str, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(format!(
            "commands nested deeper than {MAX_DEPTH} levels are refused: {MERGE_FIX}"
        ));
    }
    for inner in nested(command) {
        if let Some(why) = check(&inner, branch_at, default_branch, depth + 1) {
            return Some(why);
        }
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
        if words.first() == Some(&"cd") {
            let start = segment.as_ptr() as usize - command.as_ptr() as usize;
            let before = &command[..start];
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
                branch_at,
                default_branch,
            };
            if let Some(why) = merge_rule(&words, segment, &target) {
                return Some(format!(
                    "`{}` is refused ({why}): {MERGE_FIX}",
                    segment.trim()
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
    let words = shell_words(command);
    for (i, w) in words.iter().enumerate() {
        let name = w.rsplit('/').next().unwrap_or(w);
        if name == "eval" {
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
    "if", "while", "until",
];

/// Mutations and endpoints that merge, tag or move refs through the GitHub API.
const GRAPHQL_WRITES: &[&str] = &[
    "mergePullRequest",
    "enablePullRequestAutoMerge",
    "createRef",
    "updateRef",
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
struct Target<'a> {
    dir: Option<&'a [String]>,
    branch_at: BranchAt<'a>,
    default_branch: &'a str,
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

fn merge_rule(words: &[&str], segment: &str, target: &Target) -> Option<&'static str> {
    let words: Vec<&str> = words
        .iter()
        .copied()
        .skip_while(|w| (w.contains('=') && !w.starts_with('-')) || WRAPPERS.contains(w))
        .collect();
    match words.first().copied() {
        Some("gh") => gh_rule(&words[1..], segment),
        Some("git") => git_rule(&words[1..], target),
        Some("curl" | "wget" | "http" | "https" | "xh") => http_rule(segment),
        _ => None,
    }
}

const DECRYPT_FIX: &str = "secrets stay encrypted; the owner runs `hoist secrets edit`, CI \
    decrypts with `hoist secrets exec` (spec §8)";

fn decrypt_reason(what: &str) -> String {
    format!("decrypting secrets is refused for agents ({what}): {DECRYPT_FIX}")
}

const HOIST_SECRETS: &[&str] = &["edit", "sync", "grant", "revoke", "rotate", "rotate-key"];
/// Readers: their arguments are data (patterns, text).
#[rustfmt::skip]
const READERS: &[&str] = &["grep", "egrep", "fgrep", "rgrep", "zgrep", "ugrep", "rg", "ag", "ack",
    "cat", "head", "tail", "ls", "wc", "echo", "printf", "which", "type"];
/// The grep family; rg, ag, ack and rgrep always recurse, rg and ag skip hidden directories.
#[rustfmt::skip]
const SEARCHERS: &[&str] = &["grep", "egrep", "fgrep", "rgrep", "zgrep", "ugrep", "rg", "ag",
    "ack"];
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
/// a glob or another `..`, and `/..` is `/`.
fn lexical(path: &str) -> String {
    let abs = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for s in path.split('/') {
        let plain =
            |l: &&str| *l != ".." && !l.starts_with(['~', '$']) && !l.contains(['*', '?', '[']);
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
/// of `.config/sops` inside it (`~/.config`, `$XDG_CONFIG_HOME`), or a root above an unknown
/// cwd (`..`); 1 for home or above it (`~`, `~user`, `/root`, `/home/<user>`, `/home`, `/`);
/// 0 for anything else (`~/projects/x`).
fn key_root(word: &str) -> u8 {
    let n = lexical(word.trim_matches(|c| c == '"' || c == '\''));
    let word = n.as_str();
    let t = word.trim_end_matches('/');
    if t == ".." || t.starts_with("../") {
        return 2;
    }
    let odd = t.contains("..") || t.contains(['*', '?', '[']);
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
        return Some(decrypt_reason("commands nested too deep"));
    }
    let (text, bodies, targets) = strip_data_heredocs(command);
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
        let name = base(first);
        let args = &rest[1..];
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
            return Some(decrypt_reason("the environment"));
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
            "ack" | "ugrep" => long("--pager").map(str::to_string),
            "rg" => long("--pre").map(|c| format!("{c} FILE")),
            #[rustfmt::skip]
            "git" if verb == "grep" => long("--open-files-in-pager").or_else(|| args.iter().enumerate().find_map(|(i, a)| a.strip_prefix("-O").map(|v| if v.is_empty() { args.get(i + 1).copied().unwrap_or("") } else { v }))).map(|c| format!("{c} FILE")),
            _ => None,
        };
        if let Some(why) = run.and_then(|c| decrypt_rule(&c, depth + 1)) {
            return Some(why);
        }
        // `git grep --no-index` searches its paths (or the cwd) like `grep -r`, hidden ones too.
        if git && verb == "grep" && args.contains(&"--no-index") {
            #[rustfmt::skip]
            const VALUE: &[&str] = &["-e", "-f", "-A", "-B", "-C", "-m", "--max-depth", "--threads"];
            // The first bare word is the pattern, unless `-e` or `-f` gave it.
            let mut pat = args.iter().any(|a| matches!(*a, "-e" | "-f"));
            let (mut roots, mut it, mut paths) = (Vec::new(), it.clone(), false);
            while let Some(a) = it.next() {
                match *a {
                    "--" if !paths => paths = true,
                    _ if !paths && VALUE.contains(a) => drop(it.next()),
                    _ if !paths && a.starts_with('-') => {}
                    _ if !std::mem::replace(&mut pat, true) => {}
                    _ => roots.push(*a),
                }
            }
            if key_search(&roots, true, true) {
                return Some(decrypt_reason("a search of the age key directory"));
            }
        }
        if name == "bosun" {
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
                matches!(name.as_str(), "rg" | "ag" | "ack" | "rgrep"),
            );
            let dirs = args
                .windows(2)
                .any(|w| matches!(w[0], "-d" | "--directories") && w[1] == "recurse")
                || args
                    .iter()
                    .any(|a| matches!(*a, "-drecurse" | "--directories=recurse"));
            let recursive = always
                || dirs
                || args.iter().any(|a| {
                    matches!(*a, "--recursive" | "--dereference-recursive")
                        || a.starts_with('-') && !a.starts_with("--") && a.contains(['r', 'R'])
                });
            let hidden = !rg
                || args.iter().any(|a| {
                    matches!(*a, "--hidden" | "--unrestricted")
                        || a.starts_with('-') && !a.starts_with("--") && a.contains(['u', '.'])
                });
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
                return Some(decrypt_reason("the process environment"));
            }
            continue;
        }
        #[rustfmt::skip]
        let sub = args.iter().find(|a| !a.starts_with('-')).is_some_and(|a| PKG_SUBS.contains(a));
        let safe = FILE_TOOLS.contains(&name.as_str()) || PACKAGERS.contains(&name.as_str()) && sub;
        // Data words: message and query option values, search patterns.
        let (msg, vi) = (
            matches!(name.as_str(), "git" | "gh"),
            segment.iter().position(|x| x == verb),
        );
        let (gs, ss) = (
            name == "git" && verb == "grep",
            name == "gh" && verb == "search",
        );
        let api = (name.as_str(), verb) == ("gh", "api");
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
        // Options run command strings: a word or option value with whitespace is a command.
        for w in segment
            .iter()
            .zip(&data)
            .filter(|(_, d)| !**d)
            .map(|(w, _)| w)
        {
            let w = w.as_str();
            if procenv(w) {
                return Some(decrypt_reason("the process environment"));
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
        for i in 0..if safe { 0 } else { rest.len() } {
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
/// those bodies, and the files the opening lines write (`> f`, `>> f`, `tee f`).
fn strip_data_heredocs(command: &str) -> (String, String, Vec<String>) {
    let (mut out, mut bodies, mut skip) = (String::new(), String::new(), None::<(String, bool)>);
    let mut targets = Vec::new();
    for line in command.split_inclusive('\n') {
        if let Some((delim, dash)) = &skip {
            let l = line.trim_end_matches('\n');
            if (if *dash { l.trim_start_matches('\t') } else { l }) == delim {
                skip = None;
                out.push_str(line);
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
            let reads = shell_words(line)
                .iter()
                .any(|w| SHELLS.contains(&w.rsplit('/').next().unwrap_or(w)));
            if let Some(q) = quote
                && !reads
                && let Some(end) = spec[1..].find(q)
            {
                skip = Some((spec[1..=end].to_string(), dash));
                targets.extend(write_targets(&shell_words(line)));
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
    match args {
        ["pr", "merge", ..] => Some("gh pr merge"),
        ["release", "create", ..] => Some("a release makes a tag"),
        ["api", ..] if GRAPHQL_WRITES.iter().any(|m| segment.contains(m)) => {
            Some("GraphQL merge or ref mutation")
        }
        ["api", rest @ ..] if rest.iter().any(|a| a.contains("/merge")) => Some("merge API"),
        ["api", rest @ ..] if rest.iter().any(|a| a.contains("auto_merge")) => {
            Some("auto-merge API")
        }
        ["api", rest @ ..]
            if rest
                .iter()
                .any(|a| a.contains("git/refs") || a.contains("/tags")) =>
        {
            Some("ref or tag API")
        }
        _ => None,
    }
}

/// `v1.2.3`, `1.2` ...: a ref that looks like a version tag.
fn version_like(r: &str) -> bool {
    let t = r.strip_prefix('v').unwrap_or(r);
    t.split_once('.').is_some_and(|(a, b)| {
        !a.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit())
            && b.starts_with(|c: char| c.is_ascii_digit())
    })
}

fn git_rule(args: &[&str], target: &Target) -> Option<&'static str> {
    let default_branch = target.default_branch;
    // git's global options: `-C dir`, `--git-dir`, `--work-tree` move the target; `-c k=v`,
    // `--no-pager` and the like do not. A target that cannot be read literally is unknown.
    let (mut i, mut opts, mut known) = (0, Vec::new(), true);
    while i < args.len() && args[i].starts_with('-') {
        let (a, next) = (args[i], args.get(i + 1).copied());
        let (flag, value, step) = match a.split_once('=') {
            Some((k @ ("--git-dir" | "--work-tree"), v)) => (Some(format!("{k}=")), Some(v), 1),
            _ => match a {
                "-C" => (Some("-C".to_string()), next, 2),
                "--git-dir" | "--work-tree" => (Some(format!("{a}=")), next, 2),
                "-c" => (None, None, 2),
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
    let (sub, rest) = (args.get(i).copied()?, &args[(i + 1).min(args.len())..]);
    let on_default = match target.dir.filter(|_| known) {
        None => true,
        Some(dir) => {
            let all: Vec<String> = dir.iter().cloned().chain(opts).collect();
            match (target.branch_at)(&all) {
                Some(b) => b == default_branch,
                // A moved target git cannot read (a failed `cd`, a missing `-C` dir) may
                // leave git where the shell started: fail closed.
                None => !all.is_empty(),
            }
        }
    };
    match sub {
        "push" => {
            if rest.iter().any(|a| {
                matches!(
                    *a,
                    "-f" | "--force" | "--tags" | "--mirror" | "--delete" | "-d"
                ) || a.starts_with("--force-with-lease")
                    || a.starts_with("--force-if-includes")
            }) {
                return Some("force, tag or delete push");
            }
            let refs: Vec<&str> = rest
                .iter()
                .copied()
                .filter(|a| !a.starts_with('-'))
                .skip(1)
                .collect();
            if refs
                .iter()
                .any(|r| r.starts_with('+') || r.starts_with(':') || r.contains("refs/tags/"))
                || refs.iter().any(|r| r.split(':').any(version_like))
            {
                return Some("force, tag or delete push");
            }
            let to_default = refs.iter().any(|r| {
                let dst = r.rsplit(':').next().unwrap_or(r);
                dst == default_branch
                    || dst == format!("refs/heads/{default_branch}")
                    || (dst == "HEAD" && on_default)
            });
            if to_default || (refs.is_empty() && on_default) {
                return Some("push to the default branch");
            }
            None
        }
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
            "cd /wt && git push -u origin feature",
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
        // Past depth 3 the guard refuses rather than guessing.
        assert!(bash("(((((gh pr view 1)))))", "feature").is_some());
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
            assert!(
                tool_rules(t, &json!({}), &|_| None, "main")
                    .unwrap()
                    .contains("D13")
            );
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
    ];

    #[test]
    fn key_root_resolves_dot_dot_lexically() {
        for (root, want) in [
            ("/etc/../root/.config", 2),
            ("/etc/../root", 1),
            ("/root/../tmp", 0),
            ("/..", 1),
            ("a/../..", 2),
            ("../x", 2),
            ("~/../bob", 2),
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
    fn decrypt_lock_table() {
        let wrong: Vec<_> = DECRYPT_CASES
            .iter()
            .filter(|(cmd, deny)| bash(cmd, "feature").is_some() != *deny)
            .collect();
        assert!(wrong.is_empty(), "wrong verdict: {wrong:#?}");
        let why = bash("sops -d f", "feature").unwrap();
        assert!(!why.contains('\n') && why.contains("hoist secrets exec"));
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
}
