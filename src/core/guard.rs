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
        if set.contains("bosun:state/") {
            return Some(label_reason());
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

fn label_reason() -> String {
    "a command that names a bosun:state/* label and can write labels (gh, curl, an interpreter, \
     a /labels path) is refused: those labels change only through bosun commands (spec §4); use `bosun claim`, \
     `bosun release`, `bosun pr` or `bosun verdict post|comment`"
        .to_string()
}

/// Nested commands are checked first (label-lock and merge-lock both), to this depth.
const MAX_DEPTH: usize = 3;

fn bash_rules(command: &str, branch_at: BranchAt, default_branch: &str) -> Option<String> {
    check(command, branch_at, default_branch, 0)
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
    let (mut names_label, mut writes_labels) = (false, false);
    for (segment, sep) in segments_with_sep(command) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        if ["GIT_DIR", "GIT_WORK_TREE", "core.worktree"]
            .iter()
            .any(|v| segment.contains(v))
        {
            dir = None;
        }
        // A `bosun` segment is exempt from the label-lock only; its nested commands were
        // checked above and the merge rules see no `gh` or `git` in it. The lock judges the
        // whole command: a label named in one segment may be written by another.
        if words.first() != Some(&"bosun") {
            names_label |= segment.contains("bosun:state/");
            writes_labels |= label_writer(&words, segment);
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
    (names_label && writes_labels).then(label_reason)
}

/// Whether a segment can write labels: `gh` except its read-only subcommands, HTTP clients,
/// any interpreter (inline code cannot be told from a script that reads the text), and
/// anything with a label REST path or GraphQL label mutation in it.
fn label_writer(words: &[&str], segment: &str) -> bool {
    if ["/labels", "abelsTo", "abelsFrom", "Labelable"]
        .iter()
        .any(|m| segment.contains(m))
    {
        return true;
    }
    let mut words: Vec<&str> = words
        .iter()
        .copied()
        .skip_while(|w| (w.contains('=') && !w.starts_with('-')) || WRAPPERS.contains(w))
        .collect();
    if words.first() == Some(&"xargs") {
        words.remove(0);
        while words.first().is_some_and(|w| w.starts_with('-')) {
            words.remove(0);
        }
        words.retain(|w| !WRAPPERS.contains(w));
    }
    let Some(first) = words.first() else {
        return false;
    };
    let name = first.rsplit('/').next().unwrap_or(first);
    match name {
        "gh" => !matches!(
            &words[1..],
            ["issue", "list" | "view" | "status", ..]
                | ["pr", "list" | "view" | "status" | "diff" | "checks", ..]
                | ["search", ..]
                | ["label", "list", ..]
        ),
        "curl" | "wget" | "http" | "https" | "xh" | "httpie" => true,
        _ => ["python", "node", "ruby", "perl", "deno", "bun"]
            .iter()
            .any(|i| name.starts_with(i)),
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
            "rg bosun:state/ docs",
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
