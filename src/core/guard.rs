//! What the PreToolUse hook refuses (spec §4 edit hook, label-lock; D13 merge-lock). Pure: the
//! hook passes the tool call, the repo-relative path, the current branch and the live leases.

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
/// the paths its own orchestrator leased (`S/task` runs inside `S`); see spec §4.
pub fn edit_lock(path: &str, leases: &[Lease], me: &str) -> Option<String> {
    let hit = leases
        .iter()
        .filter(|l| l.session != me && l.session != crate::core::lease::orchestrator_of(me))
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

/// The checked-out branch of the git target a command names, given as `git` global options
/// (`-C dir`, `--git-dir=..`, `--work-tree=..`) applied after the hook's own directory; an
/// empty list is the hook's directory. `None` when it cannot be read.
pub type BranchAt<'a> = &'a dyn Fn(&[String]) -> Option<String>;

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
    check(command, branch_at, default_branch, 0)
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
/// label built at run time (`printf`), a label written to a file in one call and read in
/// another, and repo config or files set up by an earlier call (a git pager, diff driver or
/// fsmonitor, a script on `PATH`).
fn read_only(text: &str, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    if let Some(segments) = plain_segments(text) {
        return segments
            .iter()
            .all(|s| !s.grouped && !s.dollar && segment_ro(&s.words, s.glob));
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
}

/// The simple commands of a plain command line, split the way bash splits it: `'..'`, `".."`
/// and `\` quoting, `#` comments, `\`-newline joins, redirections (`>&`, `&>`, `>|`) are not
/// separators, and heredoc bodies are skipped. `None` for anything this does not model
/// exactly (command or parameter substitution, `$'..'`, an unterminated quote or heredoc, an
/// unquoted-delimiter heredoc body with `$` or a backtick): the caller then splits more
/// roughly.
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
                        if bare == delim {
                            break;
                        }
                        if !quoted && line.contains(['$', '`']) {
                            return None;
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

/// Variables an assignment-only segment may not set: they change which program runs, or make
/// an allowlisted command run one.
fn risky_var(name: &str) -> bool {
    matches!(
        name,
        "PATH" | "IFS" | "ENV" | "CDPATH" | "PAGER" | "MANPAGER" | "PROMPT_COMMAND" | "PS4"
    ) || ["LD_", "GIT_", "GH_", "LESS", "BASH"]
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
    let n = words.iter().take_while(|w| assigned(w).is_some()).count();
    let Some(cmd) = words.get(n) else {
        return !words.iter().filter_map(|w| assigned(w)).any(risky_var);
    };
    if n > 0 && cmd != "bosun" {
        return false;
    }
    let args: Vec<&str> = words[n + 1..].iter().map(String::as_str).collect();
    match cmd.as_str() {
        "grep" | "egrep" | "fgrep" | "cat" | "head" | "tail" | "wc" | "uniq" | "cut" | "tr"
        | "jq" | "diff" | "ls" | "test" | "[" | "true" | "false" | "echo" | "printf" => true,
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
        let why = edit_lock("src/core/wave.rs", &leases, "me").unwrap();
        assert!(
            why.contains("#7") && why.contains("bosun wait --issue 7"),
            "{why}"
        );
        assert_eq!(edit_lock("docs/x.md", &leases, "me"), None);
        // Brackets and braces are literal in an edited path, never a way past the lock.
        let app = Lease {
            touch: vec!["app/**".into()],
            ..leases[0].clone()
        };
        for p in ["app/[id]/page.tsx", "app/{team}/x.tsx", "app/(group)/a.tsx"] {
            assert!(
                edit_lock(p, std::slice::from_ref(&app), "me").is_some(),
                "{p}"
            );
        }
        assert_eq!(edit_lock("web/[id]/p.tsx", &[app], "me"), None);
        assert_eq!(edit_lock("src/core/wave.rs", &leases, "other"), None);
        assert_eq!(
            edited_path("Write", &json!({"file_path": "a.rs"})),
            Some("a.rs".to_string())
        );
        assert_eq!(edited_path("Read", &json!({"file_path": "a.rs"})), None);
    }
}
