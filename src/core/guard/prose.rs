//! Prose values for the label-lock (spec §4): the text of a commit message, title or body
//! names a state label without editing it. Only the value of a prose flag of `git commit|tag`
//! and `gh pr|issue create|comment|edit|review|close|reopen` is data; `--label` and
//! `--add-label` are never prose.

const FLAGS: &[&str] = &["-m", "-b", "-t", "--message", "--body", "--title"];

/// Words that mean the text may edit labels: then no prose is data.
const LABEL_EDITS: &[&str] = &[
    "--label",
    "--add-label",
    "--remove-label",
    "gh label",
    "labels",
    "labelable",
];

/// Whether the words before a flag start a command whose prose flags are data: `git commit|tag`
/// (the verb after global options such as `-C dir`, `-c k=v`) and `gh pr|issue create|comment|..`
/// (after `-R repo`), after leading `VAR=x` assignments.
fn prose_command(before: &str) -> bool {
    let seg = before
        .rsplit([';', '&', '|', '\n', '('])
        .next()
        .unwrap_or("");
    let words: Vec<&str> = seg
        .split_whitespace()
        .skip_while(|a| a.contains('=') && !a.starts_with('-'))
        .collect();
    // The next word that is not an option (or the value of one in `takes`), from `at`.
    let word = |mut at: usize, takes: &[&str]| {
        while let Some(a) = words.get(at) {
            if takes.contains(a) {
                at += 2;
            } else if a.starts_with('-') {
                at += 1;
            } else {
                return Some((*a, at + 1));
            }
        }
        None
    };
    match words.first() {
        Some(&"git") => {
            let takes = ["-C", "-c", "--git-dir", "--work-tree", "--namespace"];
            matches!(word(1, &takes), Some(("commit" | "tag", _)))
        }
        Some(&"gh") => {
            let takes = ["-R", "--repo"];
            word(1, &takes).is_some_and(|(a, at)| {
                matches!(a, "pr" | "issue")
                    && matches!(
                        word(at, &takes),
                        Some((
                            "create" | "comment" | "edit" | "review" | "close" | "reopen",
                            _
                        ))
                    )
            })
        }
        _ => false,
    }
}

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
    let short = command.split_whitespace().any(|w| w.starts_with("-l"));
    if short || LABEL_EDITS.iter().any(|w| command.contains(w)) {
        return command.to_string();
    }
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
            if prose_command(&command[..i])
                && let Some(len) = heredoc_value(value).or_else(|| quoted_value(value))
            {
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
    fn the_git_verb_is_found_after_global_options() {
        assert_eq!(
            strip_prose("git -C d commit -m 'a'"),
            "git -C d commit -m \"x\""
        );
        assert_eq!(
            strip_prose("git -c alias.y=z y -m 'a'"),
            "git -c alias.y=z y -m 'a'"
        );
        assert_eq!(
            strip_prose("git -c alias.y=tag y tag -m 'a'"),
            "git -c alias.y=tag y tag -m 'a'"
        );
    }
}
