//! Merge-lock inside interpreter code (spec §4): `python -c`, `node -e`, a heredoc fed to an
//! interpreter, and the script files they or a shell run. The code is not parsed: it is cut
//! into statements and words, and each `gh`/`git` word is judged like a shell command.

use super::{
    BranchAt, MAX_DEPTH, MERGE_FIX, RUNNERS, SEP, Sinks, Target, WRAPPERS, base, check, gh_rule,
    git_rule, heredoc_at, http_rule, literal_path, nested, shell_words, unwrap_runners,
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
/// wrappers and runners with their option values (`sudo -u root`, `timeout -s KILL 5`, as
/// `unwrap_runners` reads them), and also, failing closed, after skipping every flag and number.
fn command_words(words: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut s = 0;
    for seg in words.split(|w| w == SEP) {
        let all: Vec<&str> = seg.iter().map(String::as_str).collect();
        let mut cmd: &[&str] = &all;
        loop {
            let wrapper = |w: &str| {
                (w.contains('=') && !w.starts_with('-'))
                    || (WRAPPERS.contains(&w) && !RUNNERS.iter().any(|r| r.0 == base(w)))
            };
            let skip = cmd.iter().take_while(|w| wrapper(w)).count();
            let (rest, _, _) = unwrap_runners(&cmd[skip..]);
            if rest.len() == cmd.len() {
                break;
            }
            cmd = rest;
        }
        if !cmd.is_empty() {
            out.push(s + all.len() - cmd.len());
        }
        s += seg.len() + 1;
    }
    out.extend(flag_skipped(words));
    out.sort_unstable();
    out.dedup();
    out
}

fn flag_skipped(words: &[String]) -> Vec<usize> {
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

/// A script path in a command: the path, whether a shell runs it, and whether it is the
/// script itself (the first operand or the command word) rather than a later argument.
struct ScriptPath {
    path: String,
    shell: bool,
    first: bool,
}

/// The literal paths an interpreter or shell is given in a command (also a `./x` or `/x`
/// command word), at every nesting level (`bash -c '..'`, `$( )`).
fn script_paths(command: &str, depth: usize) -> Vec<ScriptPath> {
    let w = shell_words(command);
    let mut out = Vec::new();
    for i in command_words(&w) {
        let (word, name) = (&w[i], runner_name(&w[i]));
        if word.contains('/') && !RUNS.contains(&name) {
            out.extend(literal_path(word).map(|path| ScriptPath {
                path,
                shell: false,
                first: true,
            }));
        }
        if RUNS.contains(&name) {
            let shell = SHELL_RUNS.contains(&name);
            let args = w[i + 1..].iter().take_while(|a| *a != SEP);
            for (k, a) in args.filter(|a| !a.starts_with('-')).take(4).enumerate() {
                out.extend(literal_path(a).map(|path| ScriptPath {
                    path,
                    shell,
                    first: k == 0,
                }));
            }
        }
    }
    if depth < MAX_DEPTH {
        for inner in nested(command) {
            out.extend(script_paths(&inner, depth + 1));
        }
    }
    out
}

/// What the hook found at a script path.
pub enum Script {
    /// The text (lossy UTF-8) of a regular file of at most 1 MiB.
    Text(String),
    /// A file the hook will not read in full: why (too large, not a regular file, /proc).
    Unreadable(&'static str),
}

const STDIN: &[&str] = &["/dev/stdin", "/dev/fd/0", "/proc/self/fd/0"];

/// Whether a heredoc feeds the stdin of the command that names `path`: one on that command
/// itself, or on an earlier stage of the same pipeline (`cat <<'EOF' | python3 /dev/stdin`).
/// Pipelines end at `;`, `&&`, `||`, a newline and a lone `&`; the `&` of `>&`, `<&` and `&>`
/// is part of a redirection.
fn heredoc_feeds(command: &str, path: &str) -> bool {
    let c: Vec<char> = command.chars().collect();
    let (mut pipelines, mut cur) = (Vec::new(), String::new());
    let mut i = 0;
    while i < c.len() {
        let (ch, next, prev) = (c[i], c.get(i + 1).copied(), i.checked_sub(1).map(|p| c[p]));
        let redir_amp = ch == '&' && (matches!(prev, Some('>' | '<')) || next == Some('>'));
        let ends = match ch {
            ';' | '\n' => 1,
            '&' if next == Some('&') => 2,
            '|' if next == Some('|') => 2,
            '&' if !redir_amp => 1,
            _ => 0,
        };
        if ends > 0 {
            pipelines.push(std::mem::take(&mut cur));
            i += ends;
            continue;
        }
        cur.push(ch);
        i += 1;
    }
    pipelines.push(cur);
    pipelines.iter().any(|p| {
        let stages: Vec<&str> = p.split('|').collect();
        stages
            .iter()
            .position(|s| s.contains(path))
            .is_some_and(|at| {
                let feeds = |(i, s): (usize, &&str)| {
                    heredoc_at(s).is_some() && !input_redirect(s) && (i == at || heredoc_only(s))
                };
                stages[..=at].iter().enumerate().any(feeds)
            })
    })
}

/// Whether a stage has an input redirection besides its heredoc (`< f`, `0< f`, `<<< s`),
/// which may replace the heredoc on stdin.
fn input_redirect(stage: &str) -> bool {
    shell_words(stage).iter().any(|w| {
        let w = w.trim_start_matches(|c: char| c.is_ascii_digit());
        w.starts_with("<<<") || (w.starts_with('<') && !w.starts_with("<<"))
    })
}

/// Whether a stage with a heredoc reads only it: no operand after the command word besides
/// options and redirections (`cat <<'EOF' evil.py` prints evil.py too).
fn heredoc_only(stage: &str) -> bool {
    let (mut skip, mut operands) = (false, 0);
    for w in shell_words(stage).iter().skip(1) {
        let glued = w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
        let op = glued.trim_start_matches(['<', '>', '-']);
        if std::mem::take(&mut skip) {
            continue;
        }
        if glued.starts_with(['<', '>']) {
            skip = op.is_empty() && !glued.starts_with(">&");
        } else if !w.starts_with('-') {
            operands += 1;
        }
    }
    operands == 0
}

/// A `#!` line that runs a shell, also with arguments (`#!/bin/bash -e`, `#!/usr/bin/env sh`).
fn shell_shebang(first: &str) -> bool {
    first.strip_prefix("#!").is_some_and(|l| {
        l.split_whitespace()
            .any(|w| SHELL_RUNS.contains(&runner_name(w)))
    })
}

/// The script rule: each script file an interpreter or shell runs, read through `read`
/// (`None`: no such file, skipped), is scanned like inline code; a shell script is also judged
/// by the whole shell merge-lock. A script the hook cannot read in full is refused (a later
/// argument, which may be data, is skipped).
pub fn script_rules(
    command: &str,
    read: &dyn Fn(&str) -> Option<Script>,
    branch_at: BranchAt,
    default_branch: &str,
) -> Option<String> {
    script_walk(command, read, branch_at, default_branch, None)
}

/// [`script_rules`], reporting each allowed push to `sinks`.
pub(super) fn script_walk(
    command: &str,
    read: &dyn Fn(&str) -> Option<Script>,
    branch_at: BranchAt,
    default_branch: &str,
    sinks: Option<&Sinks>,
) -> Option<String> {
    let target = Target {
        dir: Some(&[]),
        alone: false,
        branch_at,
        default_branch,
        sink: sinks.map(|s| s.code),
    };
    for ScriptPath { path, shell, first } in script_paths(command, 0) {
        // stdin by name, fed by a heredoc: the body is in the command text, which
        // `inline_rule` scans. A heredoc in another command does not count.
        if STDIN.contains(&path.as_str()) && heredoc_feeds(command, &path) {
            continue;
        }
        let code = match read(&path) {
            None => continue,
            Some(Script::Text(code)) => code,
            Some(Script::Unreadable(_)) if !first => continue,
            Some(Script::Unreadable(why)) => {
                return Some(format!(
                    "the script `{path}` is refused: the guard cannot scan it ({why}); run a \
                     script file of at most 1 MiB, or ask the owner to run it"
                ));
            }
        };
        let line = code.lines().next().unwrap_or("");
        let shell =
            shell || path.ends_with(".sh") || path.ends_with(".bash") || shell_shebang(line);
        if shell && let Some(why) = check(&code, branch_at, default_branch, MAX_DEPTH / 2, sinks) {
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
            if p.ends_with("big.py") || p == "/dev/stdin" {
                return Some(Script::Unreadable("larger than 1 MiB"));
            }
            let known = ["x.py", "x.sh", "x.mjs", "x.php", "x.ts", "x.rb"];
            (known.iter().any(|k| p.ends_with(k)) || p == "./run")
                .then(|| Script::Text(body.to_string()))
        };
        script_rules(cmd, &read, &|_| Some("feature".into()), "main")
    }

    #[test]
    fn every_runner_and_nested_scripts_are_read() {
        let merge = "system('gh pr merge 3')\n";
        for cmd in [
            "php x.php",
            "nodejs x.mjs",
            "ts-node x.ts",
            "tsx x.ts",
            "dash x.sh",
            "ksh x.sh",
            "source x.sh",
            ". x.sh",
            "bash -c 'python3 x.py'",
            "echo $(node x.mjs)",
        ] {
            assert!(script(cmd, merge).is_some(), "allowed: {cmd}");
        }
        let shebang = "#!/bin/bash -e\ncd sub && git push origin main\n";
        assert!(script("./run", shebang).is_some());
    }

    #[test]
    fn a_script_the_hook_cannot_read_in_full_is_refused() {
        let why = script("python3 big.py", "").expect("refused");
        assert!(
            why.contains("cannot scan") && why.contains("owner"),
            "{why}"
        );
        assert_eq!(
            script("python3 x.py big.py", "print(1)"),
            None,
            "an argument is data"
        );
        assert!(
            script("python3 /dev/stdin < run.py; cat <<EOF\nx\nEOF", "").is_some(),
            "a heredoc in another segment does not feed stdin"
        );
        assert_eq!(
            script("python3 /dev/stdin <<'EOF'\nprint(1)\nEOF", ""),
            None
        );
        for cmd in [
            "python3 /dev/stdin 2>&1 <<'EOF'\nprint(1)\nEOF",
            "python3 /dev/stdin &>out.log <<'EOF'\nprint(1)\nEOF",
            "cat <<'EOF' | python3 /dev/stdin\nprint(1)\nEOF",
        ] {
            assert_eq!(script(cmd, ""), None, "{cmd}");
        }
        for cmd in [
            "python3 /dev/stdin & cat <<EOF\nx\nEOF",
            "python3 /dev/stdin | cat <<EOF\nx\nEOF",
            "python3 /dev/stdin < run.py && cat <<EOF\nx\nEOF",
            "cat <<'EOF' evil.py | python3 /dev/stdin\nprint(1)\nEOF",
            "cat <<'EOF' < evil.py | python3 /dev/stdin\nprint(1)\nEOF",
            "cat <<'EOF' 0< evil.py | python3 /dev/stdin\nprint(1)\nEOF",
            "python3 /dev/stdin <<'EOF' < evil.py\nprint(1)\nEOF",
            "cat '<<' evil.py | python3 /dev/stdin\nprint(1)",
            "cat \"<<\" evil.py | python3 /dev/stdin\nprint(1)",
            "cat -n <<'EOF' a/evil.py 2>&1 | python3 /dev/stdin\nprint(1)\nEOF",
        ] {
            assert!(script(cmd, "").is_some(), "{cmd}");
        }
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
            "sudo -u root python3 -c 'os.system(\"gh pr merge 3\")'",
            "env -u FOO python3 -c 'os.system(\"gh pr merge 3\")'",
            "timeout -s KILL 5 python3 -c 'os.system(\"gh pr merge 3\")'",
            "exec -a x python3 -c 'os.system(\"gh pr merge 3\")'",
            "time -f %e python3 -c 'os.system(\"gh pr merge 3\")'",
            "stdbuf -o L python3 -c 'os.system(\"gh pr merge 3\")'",
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
            r#"python3 -c 'subprocess.run(["git", "push", "-u", "origin", "claude/x:claude/x"])'"#,
            r#"node -e 'execSync("git status && git log --oneline -3")'"#,
            "ruby -e 'puts 1'",
            "deno eval 'console.log(1)'",
            "git push -u origin HEAD:claude/x && sh -c 'echo $HOME'",
            "git push -u origin claude/x:claude/x && python3 -c 'print(1.5)'",
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
            "sudo -u root python3 x.py",
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
            script(
                "bash x.sh",
                "cargo test\ngit push -u origin claude/x:claude/x\n"
            ),
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
