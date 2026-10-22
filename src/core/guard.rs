//! What the PreToolUse hook refuses (spec §4 edit hook, label-lock; D13 merge-lock). Pure: the
//! hook passes the tool call, the repo-relative path, the current branch and the live leases.

use serde_json::Value;

use crate::core::glob::Glob;
use crate::core::lease::{self, Lease};

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

/// Edit-lock: a path inside a live lease of another session. An agent may edit the paths its
/// own orchestrator leased (`S/task` runs inside `S`); see spec §4.
pub fn edit_lock(path: &str, leases: &[Lease], me: &str) -> Option<String> {
    let glob = Glob::parse(path).ok()?;
    let hit = lease::conflicts(&[glob], leases, me, None)
        .into_iter()
        .next()?;
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

fn bash_rules(command: &str, branch: Option<&str>, default_branch: &str) -> Option<String> {
    for segment in segments(command) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        if words.first() == Some(&"bosun") {
            continue;
        }
        if segment.contains("bosun:state/") {
            return Some(label_reason());
        }
        if let Some(why) = merge_rule(&words, branch, default_branch) {
            return Some(format!(
                "`{}` is refused ({why}): {MERGE_FIX}",
                segment.trim()
            ));
        }
    }
    None
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

fn merge_rule(words: &[&str], branch: Option<&str>, default_branch: &str) -> Option<&'static str> {
    let words: Vec<&str> = words
        .iter()
        .copied()
        .skip_while(|w| w.contains('=') && !w.starts_with('-'))
        .collect();
    match words.first().copied() {
        Some("gh") => gh_rule(&words[1..]),
        Some("git") => git_rule(&words[1..], branch, default_branch),
        _ => None,
    }
}

fn gh_rule(args: &[&str]) -> Option<&'static str> {
    match args {
        ["pr", "merge", ..] => Some("gh pr merge"),
        ["release", "create", ..] => Some("a release makes a tag"),
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
        assert_eq!(edit_lock("src/core/wave.rs", &leases, "other"), None);
        assert_eq!(
            edited_path("Write", &json!({"file_path": "a.rs"})),
            Some("a.rs".to_string())
        );
        assert_eq!(edited_path("Read", &json!({"file_path": "a.rs"})), None);
    }
}
