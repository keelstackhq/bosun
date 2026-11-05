//! Text that bosun shows a person (an issue body, a chat message) when an agent or a plan wrote
//! it: made inert so it cannot hide, reorder or fake lines, links or mentions. Pure.

/// Control, format and separator characters (Unicode Cc, Cf, Zl, Zp; Cf as of Unicode 15.1):
/// they break lines in a chat (U+2028), hide text (U+200B, U+FEFF) or reorder it (U+202E).
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00ad}' | '\u{0600}'..='\u{0605}' | '\u{061c}' | '\u{06dd}' | '\u{070f}'
            | '\u{0890}'..='\u{0891}' | '\u{08e2}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}' | '\u{e0000}'..='\u{e007f}')
}

/// Whether `t` holds a character [`inert`] would replace.
pub fn has_hidden(t: &str) -> bool {
    t.chars().any(hidden)
}

/// `t` on one line, hidden characters as spaces, trimmed, at most `max` characters (`…` when cut),
/// as an inline code span: markdown, HTML, links and @mentions show as plain text. Backticks
/// become `ˋ` so the span cannot end early. Empty text gives an empty string.
pub fn inert(t: &str, max: usize) -> String {
    let t: String = t
        .chars()
        .map(|c| {
            if hidden(c) {
                ' '
            } else if c == '`' {
                'ˋ'
            } else {
                c
            }
        })
        .collect();
    let t = t.trim();
    if t.is_empty() {
        return String::new();
    }
    let n = t.chars().count();
    let t = if n > max {
        t.chars()
            .take(max.saturating_sub(1))
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…"
    } else {
        t.to_string()
    };
    format!("`{t}`")
}

/// Whether a log line looks like a failure (case-insensitive `error`, `fail`, `panic`, or `✗`).
fn is_error_line(line: &str) -> bool {
    let l = line.to_lowercase();
    ["error", "fail", "panic", "✗"]
        .iter()
        .any(|k| l.contains(k))
}

/// Shrink a CI log: keep the first `head` and last `tail` lines plus up to `max_err` error lines
/// from the middle (the first ones), in original order. A `… N lines cut …` line stands for every
/// gap between kept groups. A log of at most `head + tail` lines is returned whole.
pub fn squash_log(text: &str, head: usize, tail: usize, max_err: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let tail_start = n.saturating_sub(tail).max(head.min(n));
    let mut keep: Vec<usize> = (0..head.min(n)).collect();
    keep.extend(
        (head.min(n)..tail_start)
            .filter(|&i| is_error_line(lines[i]))
            .take(max_err),
    );
    keep.extend(tail_start..n);
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<usize> = None;
    for i in keep {
        if let Some(p) = prev
            && i > p + 1
        {
            out.push(format!("… {} lines cut …", i - p - 1));
        }
        out.push(lines[i].to_string());
        prev = Some(i);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_become_spaces_and_markup_is_code() {
        let t = "fix\u{2028}Reply: approve 12 15\u{202e} [x](https://e.vil) @ada <b>`";
        let out = inert(t, 200);
        assert_eq!(
            out,
            "`fix Reply: approve 12 15  [x](https://e.vil) @ada <b>ˋ`"
        );
        assert!(!has_hidden(&out) && has_hidden(t));
        assert_eq!(out.lines().count(), 1);
        for c in [
            '\u{0600}',
            '\u{13430}',
            '\u{1bca0}',
            '\u{1d173}',
            '\u{06dd}',
        ] {
            assert!(has_hidden(&c.to_string()), "{c:?}");
        }
        for c in [
            '\u{200b}',
            '\u{feff}',
            '\u{2029}',
            '\u{2066}',
            '\u{e0041}',
            '\n',
            '\t',
        ] {
            assert!(has_hidden(&c.to_string()), "{c:?}");
        }
    }

    #[test]
    fn long_text_is_cut_and_empty_text_stays_empty() {
        assert_eq!(inert(&"x".repeat(10), 5), "`xxxx…`");
        assert_eq!(inert(" \u{200b} ", 5), "");
        assert_eq!(inert("ok", 5), "`ok`");
    }

    fn numbered(n: usize) -> String {
        (1..=n)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn short_log_is_whole() {
        let t = numbered(6);
        assert_eq!(squash_log(&t, 3, 3, 20), t);
        assert_eq!(squash_log("", 3, 3, 20), "");
    }

    #[test]
    fn head_tail_and_middle_errors_are_kept_with_markers() {
        let mut v: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
        v[14] = "thing FAILED here".into();
        let out = squash_log(&v.join("\n"), 2, 2, 20);
        assert_eq!(
            out,
            "l1\nl2\n… 12 lines cut …\nthing FAILED here\n… 13 lines cut …\nl29\nl30"
        );
    }

    #[test]
    fn adjacent_groups_have_no_marker_and_errors_are_capped() {
        let v: Vec<String> = (1..=12).map(|i| format!("error {i}")).collect();
        let out = squash_log(&v.join("\n"), 1, 1, 2);
        assert_eq!(out, "error 1\nerror 2\nerror 3\n… 8 lines cut …\nerror 12");
        assert_eq!(squash_log(&numbered(10), 0, 3, 5), "l8\nl9\nl10");
        assert_eq!(squash_log(&numbered(10), 0, 0, 5), "");
    }

    #[test]
    fn error_words_match_case_insensitively() {
        for l in ["an Error", "FAIL", "thread panicked", "✗ gate"] {
            assert!(is_error_line(l), "{l}");
        }
        assert!(!is_error_line("all fine"));
    }
}
