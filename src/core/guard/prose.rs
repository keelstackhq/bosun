//! Prose values for the label-lock (spec §4): the text of a commit message, title or body
//! names a state label without editing it. Only the value of a prose flag of `git commit|tag`
//! and `gh pr|issue create|comment|edit|review|close|reopen` is data; `--label` and
//! `--add-label` are never prose.

use super::shell_words;

const FLAGS: &[&str] = &["-m", "-b", "-t", "--message", "--body", "--title"];

/// The end of a `"$(cat <<'EOF' .. EOF\n)"` value that starts at `s`: only a `cat` of a
/// quoted-delimiter heredoc, closed by `)"` right after the delimiter line.
fn heredoc_value(s: &str) -> Option<usize> {
    let rest = s.strip_prefix("\"$(cat <<")?;
    let (dash, rest) = rest.strip_prefix('-').map_or((false, rest), |r| (true, r));
    let rest = rest.trim_start_matches(' ');
    let q = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let end = rest[1..].find(q)?;
    let (delim, after) = (&rest[1..=end], &rest[end + 2..]);
    let (first, mut body) = after.split_once('\n')?;
    if delim.is_empty() || !first.trim().is_empty() {
        return None;
    }
    loop {
        let (line, next) = body.split_once('\n').unwrap_or((body, ""));
        let bare = if dash {
            line.trim_start_matches('\t')
        } else {
            line
        };
        // Bash ends a heredoc inside `$( )` at `EOF)` too: such a line is not plain data.
        if bare.starts_with(delim) && bare != delim {
            return None;
        }
        if bare == delim {
            let tail = next.trim_start_matches([' ', '\t', '\n']);
            return tail.starts_with(")\"").then(|| s.len() - tail.len() + 2);
        }
        if next.is_empty() {
            return None;
        }
        body = next;
    }
}

/// The end of a plain quoted value at the start of `s` with nothing that expands.
fn quoted_value(s: &str) -> Option<usize> {
    let q = s.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let end = 1 + s[1..].find(q)?;
    let inner = &s[1..end];
    (q == '\'' || !inner.contains(['$', '`', '\\'])).then_some(end + 1)
}

/// The command with the prose values of prose flags replaced by `"x"`.
pub fn strip_prose(command: &str) -> String {
    let out = strip(command);
    if simple_command(&out) && !edits_labels(&out) {
        out
    } else {
        command.to_string()
    }
}

/// Whether the text, outside the prose values, is one plain `git commit|tag` or
/// `gh pr|issue VERB` command that starts with those words (no `VAR=`, no global option such as
/// `git -c`) and uses only the flags on its allowlist; any other flag (`-e`, `-t`, `-F`,
/// `--cleanup`, `-R`) may run an editor or read a file, so the text is not prose. No newline,
/// `;`, `&`, `|`, `#`, `$`, backtick, `<`, `>`, parens or backslash outside the values either.
fn simple_command(text: &str) -> bool {
    const BAD: &[char] = &['\n', ';', '&', '|', '#', '$', '`', '<', '>', '(', ')', '\\'];
    if text.contains(BAD) {
        return false;
    }
    let w = shell_words(text);
    let w: Vec<&str> = w.iter().map(String::as_str).collect();
    // Flags with a value, flags without, whether operands (a number, branch or tag) are allowed.
    let (rest, val, bare, operands): (&[&str], &[&str], &[&str], bool) = match w.as_slice() {
        ["git", "commit", rest @ ..] => (
            rest,
            &["-m", "--message"],
            &[
                "-a",
                "--all",
                "-s",
                "--signoff",
                "--allow-empty",
                "-q",
                "--quiet",
            ],
            false,
        ),
        ["git", "tag", rest @ ..] => (rest, &["-m", "--message"], &["-a"], true),
        ["gh", "pr" | "issue", "create", rest @ ..] => (
            rest,
            &[
                "-t", "--title", "-b", "--body", "-B", "--base", "-H", "--head",
            ],
            &["-d", "--draft"],
            false,
        ),
        [
            "gh",
            "pr" | "issue",
            "edit" | "comment" | "close" | "reopen",
            rest @ ..,
        ] => (
            rest,
            &[
                "-t", "--title", "-b", "--body", "-B", "--base", "-H", "--head",
            ],
            &["-d", "--draft"],
            true,
        ),
        ["gh", "pr", "review", rest @ ..] => (
            rest,
            &["-b", "--body"],
            &["-c", "--comment", "-r", "--request-changes"],
            true,
        ),
        _ => return false,
    };
    let mut i = 0;
    while let Some(a) = rest.get(i) {
        i += 1;
        if !a.starts_with('-') {
            if !operands {
                return false;
            }
        } else if let Some((name, _)) = a.split_once('=').filter(|(n, _)| n.starts_with("--")) {
            if !val.contains(&name) {
                return false;
            }
        } else if val.contains(a) {
            i += 1;
            if i > rest.len() {
                return false;
            }
        } else if !bare.contains(a) {
            return false;
        }
    }
    true
}

/// Whether any word outside the prose values may edit labels: a word with `label` (any case,
/// quotes removed, so `--add-label`, `gh label`, `labels`) or, for a gh command, a short flag
/// cluster with `l` (`-l`, `-dl`).
fn edits_labels(text: &str) -> bool {
    let words = shell_words(text);
    let gh = words.iter().any(|w| w == "gh" || w.ends_with("/gh"));
    words.iter().any(|w| {
        w.to_lowercase().contains("label")
            || (gh && w.starts_with('-') && !w.starts_with("--") && w.contains('l'))
    })
}

fn strip(command: &str) -> String {
    let (mut out, mut quote, mut i) = (String::new(), None, 0);
    let b = command.as_bytes();
    while i < command.len() {
        let rest = &command[i..];
        let c = rest.chars().next().unwrap_or(' ');
        let token_start = i == 0 || b[i - 1].is_ascii_whitespace();
        if quote.is_none()
            && token_start
            && let Some(flag) = FLAGS.iter().find(|f| rest.starts_with(**f))
        {
            let after = &rest[flag.len()..];
            let sep = after.chars().next();
            let glued = sep == Some('=') && flag.starts_with("--");
            let value = if glued {
                &after[1..]
            } else if sep.is_some_and(char::is_whitespace) {
                after.trim_start()
            } else {
                ""
            };
            if let Some(len) = heredoc_value(value).or_else(|| quoted_value(value)) {
                let skipped = after.len() - value.len();
                out.push_str(flag);
                out.push_str(if glued { "=" } else { " " });
                out.push_str("\"x\"");
                i += flag.len() + skipped + len;
                continue;
            }
        }
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (Some('"') | None, '\\') => {
                out.push(c);
                i += 1;
                if let Some(n) = command[i..].chars().next() {
                    out.push(n);
                    i += n.len_utf8();
                }
                continue;
            }
            _ => {}
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_prose;

    #[test]
    fn a_heredoc_that_bash_ends_at_a_delimiter_with_a_paren_is_not_prose() {
        let c = "git commit -m \"$(cat <<'EOF'\nx\nEOF)\"; rm y\nEOF\n)\"";
        assert_eq!(strip_prose(c), c);
        let ok = "git commit -m \"$(cat <<'EOF'\nx\nEOF\n)\"";
        assert_eq!(strip_prose(ok), "git commit -m \"x\"");
    }

    #[test]
    fn any_label_edit_word_keeps_the_text_unchanged() {
        for c in [
            "git commit -m 'a' && gh issue edit 3 --add-label \"$(x)\"",
            "git commit -m 'a'; gh api r/labels",
            "git commit -m 'a'; gh label create x",
            "git commit -m 'a' && gh issue create -t x -b y -l \"$(x)\"",
            "git commit -m 'a' && gh issue create -t x -b y -lfoo",
        ] {
            assert_eq!(strip_prose(c), c);
        }
    }

    #[test]
    fn only_plain_commands_with_allowlisted_flags_are_prose() {
        for c in [
            "git commit -m 'a'",
            "git commit -a -s --allow-empty -m 'a'",
            "git tag -a v1 -m 'a'",
            "gh pr create -t 'a' --body='b' -B main -d",
            "gh issue comment 3 --body 'a'",
        ] {
            assert_ne!(strip_prose(c), c, "{c}");
        }
        for c in [
            "git -C d commit -m 'a'",
            "git -c core.editor=sh commit -e -m 'a'",
            "GIT_EDITOR=sh git commit -e -m 'a'",
            "git commit -e -m 'a'",
            "git commit -t tmpl -m 'a'",
            "git commit -F f -m 'a'",
            "git commit --cleanup=x -m 'a'",
            "git commit --amend -m 'a'",
            "git commit -m 'a' file",
            "gh pr create -F f -t 'a'",
            "gh -R o/r issue comment 3 --body 'a'",
            "gh pr review 3 -a -b 'a'",
            "gh pr create --fill -t 'a'",
        ] {
            assert_eq!(strip_prose(c), c, "{c}");
        }
    }
}
