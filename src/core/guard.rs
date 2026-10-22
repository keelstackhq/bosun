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
/// never parsed as a glob itself. A lease with a bad glob covers everything.
pub fn edit_lock(path: &str, leases: &[Lease], me: &str) -> Option<String> {
    let hit = leases.iter().filter(|l| l.session != me).find_map(|l| {
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

/// Label-lock and merge-lock; `None` allows the call. `branch` is the checked-out branch and
/// `default_branch` the repo's default branch.
pub fn tool_rules(
    tool: &str,
    input: &Value,
    branch: Option<&str>,
    default_branch: &str,
) -> Option<String> {
    if tool == "Bash" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        return bash_rules(command, branch, default_branch);
    }
    if let Some(name) = tool.strip_prefix("mcp__github__") {
        if matches!(name, "merge_pull_request" | "enable_pr_auto_merge") {
            return Some(format!("`{tool}` is refused: {MERGE_FIX}"));
        }
        if input.to_string().contains("bosun:state/") {
            return Some(label_reason());
        }
    }
    None
}

fn label_reason() -> String {
    "bosun:state/* labels change only through bosun commands (spec §4): use `bosun claim`, \
     `bosun release`, `bosun pr` or `bosun verdict post`"
        .to_string()
}

/// Nested commands are checked first (label-lock and merge-lock both), to this depth.
const MAX_DEPTH: usize = 3;

fn bash_rules(command: &str, branch: Option<&str>, default_branch: &str) -> Option<String> {
    check(command, branch, default_branch, 0)
}

fn check(
    command: &str,
    branch: Option<&str>,
    default_branch: &str,
    depth: usize,
) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(format!(
            "commands nested deeper than {MAX_DEPTH} levels are refused: {MERGE_FIX}"
        ));
    }
    for inner in nested(command) {
        if let Some(why) = check(&inner, branch, default_branch, depth + 1) {
            return Some(why);
        }
    }
    for segment in segments(command) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        // A `bosun` segment is exempt from the label-lock only; its nested commands were
        // checked above and the merge rules see no `gh` or `git` in it.
        if words.first() != Some(&"bosun") && segment.contains("bosun:state/") {
            return Some(label_reason());
        }
        if let Some(why) = merge_rule(&words, segment, branch, default_branch) {
            return Some(format!(
                "`{}` is refused ({why}): {MERGE_FIX}",
                segment.trim()
            ));
        }
    }
    None
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

/// Split a shell line at `;`, `&&`, `||`, `|` and newlines (quotes are not parsed; a quoted
/// separator only makes the check stricter).
fn segments(command: &str) -> Vec<&str> {
    command
        .split(['\n', ';', '&', '|'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
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

fn merge_rule(
    words: &[&str],
    segment: &str,
    branch: Option<&str>,
    default_branch: &str,
) -> Option<&'static str> {
    let words: Vec<&str> = words
        .iter()
        .copied()
        .skip_while(|w| (w.contains('=') && !w.starts_with('-')) || WRAPPERS.contains(w))
        .collect();
    match words.first().copied() {
        Some("gh") => gh_rule(&words[1..], segment),
        Some("git") => git_rule(&words[1..], branch, default_branch),
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

fn git_rule(args: &[&str], branch: Option<&str>, default_branch: &str) -> Option<&'static str> {
    // Skip git's global options (`-C dir`, `-c k=v`, `--no-pager`).
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += if matches!(args[i], "-C" | "-c") { 2 } else { 1 };
    }
    let (sub, rest) = (args.get(i).copied()?, &args[(i + 1).min(args.len())..]);
    let on_default = branch == Some(default_branch);
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
        tool_rules("Bash", &json!({ "command": cmd }), Some(branch), "main")
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
    fn label_lock() {
        assert!(bash("gh issue edit 4 --add-label bosun:state/merged", "x").is_some());
        assert_eq!(bash("bosun claim 4 # bosun:state/claimed", "x"), None);
        let mcp = json!({"issue_number": 4, "labels": ["bosun:state/queued"]});
        assert!(tool_rules("mcp__github__issue_write", &mcp, None, "main").is_some());
        let other = json!({"issue_number": 4, "labels": ["bug"]});
        assert_eq!(
            tool_rules("mcp__github__issue_write", &other, None, "main"),
            None
        );
        for t in [
            "mcp__github__merge_pull_request",
            "mcp__github__enable_pr_auto_merge",
        ] {
            assert!(
                tool_rules(t, &json!({}), None, "main")
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
