//! Prose values for the label-lock (spec §4): the text of a commit message, title or body
//! names a state label without editing it. Only the value of a prose flag of `git commit|tag`
//! and `gh pr|issue create|comment|edit|review|close|reopen` is data; `--label` and
//! `--add-label` are never prose.

const FLAGS: &[&str] = &["-m", "-b", "-t", "--message", "--body", "--title"];

/// Whether the words before a flag start a command whose prose flags are data.
fn prose_command(before: &str) -> bool {
    let seg = before
        .rsplit([';', '&', '|', '\n', '('])
        .next()
        .unwrap_or("");
    let w: Vec<&str> = seg.split_whitespace().collect();
    match w.as_slice() {
        ["git", rest @ ..] => rest.iter().any(|a| matches!(*a, "commit" | "tag")),
        [
            "gh",
            "pr" | "issue",
            "create" | "comment" | "edit" | "review" | "close" | "reopen",
            ..,
        ] => true,
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
        if (if dash {
            line.trim_start_matches('\t')
        } else {
            line
        }) == delim
        {
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
