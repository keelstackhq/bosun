//! Merge-lock inside interpreter code (spec §4): `python -c`, `node -e`, a heredoc fed to an
//! interpreter, and the script files they or a shell run. The code is not parsed: it is cut
//! into statements and words, and each `gh`/`git` word is judged like a shell command.

use super::{
    BranchAt, MAX_DEPTH, MERGE_FIX, RUNNERS, SEP, Target, WRAPPERS, base, check, gh_rule, git_rule,
    http_rule, literal_path, shell_words,
};

/// Interpreters and shells whose code or script the scan reads (a version suffix is dropped:
/// `python3.12` is `python`).
const RUNS: &[&str] = &[
    "python", "node", "nodejs", "ruby", "perl", "deno", "bun", "php", "tsx", "ts-node", "sh",
    "bash", "zsh", "dash", "ksh", "source", ".",
];
const SHELL_RUNS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "source", "."];

fn runner_name(word: &str) -> &str {
    let b = base(word);
    let t = b.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if t.is_empty() { b } else { t }
}

/// Whether a word names an interpreter or shell this scan reads (`python3.12`, `tsx`).
pub(super) fn is_runner(word: &str) -> bool {
    RUNS.contains(&runner_name(word)) && word != "."
}

/// The command words of a command: the first word of each segment after assignments,
/// wrappers, runners (`timeout 5`, `nice -n 5`) and their flags and numbers.
fn command_words(words: &[String]) -> Vec<usize> {
    let (mut out, mut start) = (Vec::new(), true);
    for (i, w) in words.iter().enumerate() {
        if w == SEP {
            start = true;
            continue;
        }
        let skip = (w.contains('=') && !w.starts_with('-'))
            || WRAPPERS.contains(&w.as_str())
            || RUNNERS.iter().any(|r| r.0 == base(w))
            || w.starts_with('-')
            || w.bytes().all(|c| c.is_ascii_digit());
        if start && !skip {
            out.push(i);
            start = false;
        }
    }
    out
}

pub(super) fn has_interpreter(command: &str) -> bool {
    let w = shell_words(command);
    command_words(&w).into_iter().any(|i| is_runner(&w[i]))
}

/// The fix of a refusal: the same as the shell merge-lock, plus the owner.
fn refuse(what: &str, why: &str) -> String {
    format!(
        "{what} is refused ({why}): {MERGE_FIX}; anything else that merges, tags or releases is the owner's to run"
    )
}

/// Cut code into statements at newlines and `;` outside brackets (a call spread over lines
/// stays one statement), and at `&` and `|` anywhere (a shell chain inside a string).
fn statements(code: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0usize);
    for c in code.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '&' | '|' => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            '\n' | ';' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

/// The words of a statement: quotes, commas and brackets split; `{x}` and `${x}` become `$x`,
/// so a computed push target stays unreadable (refused); object keys (`args:`) are dropped.
fn words(statement: &str) -> Vec<String> {
    let s = statement
        .replace("${", "$")
        .replace('{', "$")
        .replace('}', "");
    s.split(|c: char| !(c.is_alphanumeric() || "-_./:+@*=~$^".contains(c)))
        .filter(|w| !w.is_empty() && !w.ends_with(':'))
        .map(str::to_string)
        .collect()
}

/// Merge-lock over code: each `gh` or `git` word with the rest of its statement, and the
/// GitHub API rules of `curl` on each statement.
pub(super) fn code_rule(code: &str, target: &Target) -> Option<&'static str> {
    for st in statements(code) {
        if let Some(why) = http_rule(&st) {
            return Some(why);
        }
        let w = words(&st);
        for (i, word) in w.iter().enumerate() {
            let rest: Vec<&str> = w[i + 1..].iter().map(String::as_str).collect();
            let why = match base(word) {
                "gh" => gh_rule(&rest, &st),
                "git" => git_rule(&rest, target),
                _ => None,
            };
            if why.is_some() {
                return why;
            }
        }
    }
    None
}

/// The inline-code rule of one command level: when an interpreter runs, its code (`-c`, `-e`,
/// `eval`, a heredoc) is somewhere in the command text, so the whole text is scanned.
pub(super) fn inline_rule(command: &str, target: &Target) -> Option<String> {
    if !has_interpreter(command) {
        return None;
    }
    code_rule(command, target).map(|why| refuse("interpreter code", why))
}

/// The literal paths an interpreter or shell is given in a command (also a `./x` or `/x`
/// command word), with whether a shell runs it.
fn script_paths(command: &str) -> Vec<(String, bool)> {
    let w = shell_words(command);
    let mut out = Vec::new();
    for i in command_words(&w) {
        let (word, name) = (&w[i], runner_name(&w[i]));
        if word.contains('/') && !RUNS.contains(&name) {
            out.extend(literal_path(word).map(|p| (p, false)));
        }
        if RUNS.contains(&name) {
            let shell = SHELL_RUNS.contains(&name);
            let args = w[i + 1..].iter().take_while(|a| *a != SEP);
            for a in args.filter(|a| !a.starts_with('-')).take(4) {
                out.extend(literal_path(a).map(|p| (p, shell)));
            }
        }
    }
    out
}

/// The script rule: each script file an interpreter or shell runs, read through `read` (the
/// hook reads a regular text file of at most 1 MiB; `None` skips it), is scanned like inline
/// code; a shell script is also judged by the whole shell merge-lock.
pub fn script_rules(
    command: &str,
    read: &dyn Fn(&str) -> Option<String>,
    branch_at: BranchAt,
    default_branch: &str,
) -> Option<String> {
    let target = Target {
        dir: Some(&[]),
        branch_at,
        default_branch,
    };
    for (path, shell) in script_paths(command) {
        let Some(code) = read(&path) else { continue };
        let first = code.lines().next().unwrap_or("");
        let shell = shell
            || path.ends_with(".sh")
            || path.ends_with(".bash")
            || (first.starts_with("#!") && first.ends_with("sh"));
        if shell && let Some(why) = check(&code, branch_at, default_branch, MAX_DEPTH / 2) {
            return Some(format!("in the script `{path}`: {why}"));
        }
        if let Some(why) = code_rule(&code, &target) {
            return Some(refuse(&format!("the script `{path}`"), why));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::tool_rules;
    use super::*;
    use serde_json::json;

    fn bash(cmd: &str) -> Option<String> {
        tool_rules(
            "Bash",
            &json!({ "command": cmd }),
            &|_| Some("feature".into()),
            "main",
        )
    }

    fn script(cmd: &str, body: &str) -> Option<String> {
        let read = |p: &str| {
            (p.ends_with("x.py") || p.ends_with("x.sh") || p.ends_with("x.mjs") || p == "./run")
                .then(|| body.to_string())
        };
        script_rules(cmd, &read, &|_| Some("feature".into()), "main")
    }

    #[test]
    fn inline_interpreter_code_that_merges_tags_or_force_pushes_is_refused() {
        for cmd in [
            r#"python -c 'import subprocess; subprocess.run(["gh", "pr", "merge", "3"])'"#,
            r#"python3 -c "import os; os.system('gh pr merge 3 --squash')""#,
            r#"python3.12 -c 'import os; os.system("git push --force origin x")'"#,
            r#"node -e 'require("child_process").execSync("git tag v1.0.0")'"#,
            r#"node --eval 'cp.execSync("git push origin --tags")'"#,
            r#"ruby -e 'system("gh", "release", "create", "v1")'"#,
            r#"perl -e 'system("git push -f origin HEAD")'"#,
            r#"deno eval 'new Deno.Command("gh", {args: ["pr", "merge"]}).outputSync()'"#,
            r#"bun -e 'Bun.spawnSync(["git", "push", "origin", "main"])'"#,
            r#"node -e 'fetch("https://api.github.com/graphql", {method: "POST", body: "mutation { mergePullRequest(input: {}) { clientMutationId } }"})'"#,
            r#"python3 -c 'requests.put(f"https://api.github.com/repos/o/r/pulls/{n}/merge")'"#,
            r#"node -e 'q("enablePullRequestAutoMerge")'"#,
            "python3 - <<'EOF'\nimport subprocess\nsubprocess.run([\n  \"gh\",\n  \"pr\", \"merge\"])\nEOF",
            r#"python -c 'os.system(f"git push origin {branch}")'"#,
            "python3.12 - <<'EOF'\nimport os\nos.system('gh pr merge 3')\nEOF",
            "tsx <<'EOF'\nexecSync('gh pr merge 3')\nEOF",
            "cat <<'EOF' | python3.11\nos.system('gh pr merge 3')\nEOF",
            "env python3.12 - <<'EOF'\nos.system('git push --force')\nEOF",
            "timeout 5 node -e 'execSync(\"git tag v1\")'",
        ] {
            let why = bash(cmd).unwrap_or_else(|| panic!("allowed: {cmd}"));
            assert!(
                why.contains("interpreter code") && why.contains("owner"),
                "{cmd}: {why}"
            );
        }
    }

    #[test]
    fn harmless_interpreter_code_passes() {
        for cmd in [
            "python -c 'print(1)'",
            "python3 -c 'import json; print(json.dumps({\"merge\": 1}))'",
            "node -e 'console.log(process.argv)'",
            "node script.mjs",
            r#"python3 -c 'subprocess.run(["git", "push", "-u", "origin", "claude/x"])'"#,
            r#"node -e 'execSync("git status && git log --oneline -3")'"#,
            "ruby -e 'puts 1'",
            "deno eval 'console.log(1)'",
            "git push -u origin HEAD && sh -c 'echo $HOME'",
            "git push -u origin claude/x && python3 -c 'print(1.5)'",
            "git commit -F - <<'EOF'\nfix: python -c and gh pr merge refusals\nEOF",
        ] {
            assert_eq!(bash(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn script_files_that_merge_are_refused() {
        let merge = "import subprocess\nsubprocess.run(['gh', 'pr', 'merge', '--auto'])\n";
        for cmd in [
            "python x.py",
            "python3 -u x.py arg",
            "cd a && python3 ./x.py",
        ] {
            let why = script(cmd, merge).unwrap_or_else(|| panic!("allowed: {cmd}"));
            assert!(why.contains("x.py") && why.contains("owner"), "{why}");
        }
        assert!(script("node x.mjs", "execSync('git push --force')").is_some());
        assert!(script("bash x.sh", "set -e\ngit tag v2\n").is_some());
        assert!(script("sh x.sh", "cd sub && git push origin main\n").is_some());
        assert!(script("./run", "#!/bin/sh\ngh pr merge 4\n").is_some());
        assert!(
            script(
                "FOO=1 ./run",
                "#!/usr/bin/env python3\nos.system('gh pr merge')\n"
            )
            .is_some()
        );
    }

    #[test]
    fn script_files_without_merge_words_pass() {
        assert_eq!(script("node x.mjs", "console.log('merge sort')\n"), None);
        assert_eq!(script("python x.py", "print(1)\n"), None);
        assert_eq!(
            script("bash x.sh", "cargo test\ngit push -u origin claude/x\n"),
            None
        );
        assert_eq!(
            script("python other.py", "gh pr merge"),
            None,
            "unreadable: skipped"
        );
        assert_eq!(script("cat x.py", "gh pr merge"), None, "not run");
        for cmd in [
            "grep -n python x.py",
            "rg -n sh x.sh",
            "git diff . x.sh",
            "echo bash x.sh",
        ] {
            assert_eq!(script(cmd, "gh pr merge"), None, "not a runner: {cmd}");
        }
    }
}
