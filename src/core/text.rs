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

/// Whether a log line looks like a real failure: `error[`, `error:` or `error ` at a word start
/// (any case), `FAILED`, `FAIL `, `panicked`, `✗` or `##[error]`. Passing-test noise (`... ok`,
/// `test result: ok`, `PASS ...`) never matches.
fn is_error_line(line: &str) -> bool {
    let t = line.trim_end();
    if t.ends_with(" ok") || t == "ok" || t.starts_with("PASS") || t.contains("test result: ok") {
        return false;
    }
    if ["FAILED", "FAIL ", "panicked", "✗", "##[error]"]
        .iter()
        .any(|k| t.contains(k))
    {
        return true;
    }
    let l = t.to_lowercase();
    ["error[", "error:", "error "].iter().any(|k| {
        l.match_indices(k).any(|(i, _)| {
            !l[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    })
}

/// `s` without ANSI CSI sequences (ESC `[`, parameter bytes, a final byte 0x40-0x7E). An
/// unfinished sequence at the end is dropped.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for f in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&f) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One log line, cut to 500 characters with a trailing `…` when longer.
fn cap_line(line: &str) -> String {
    if line.chars().count() <= 500 {
        return line.to_string();
    }
    let mut s: String = line.chars().take(500).collect();
    s.push('…');
    s
}

/// Shrink a CI log: keep the first `head` and last `tail` lines plus up to `max_err` error lines
/// from the middle (the first ones), in original order. A `… N lines cut …` line stands for every
/// gap before the first kept line and between kept groups; each line is capped at 500 characters. A log of at most `head + tail` lines is returned whole.
pub fn squash_log(text: &str, head: usize, tail: usize, max_err: usize) -> String {
    let stripped: Vec<String> = text.lines().map(strip_ansi).collect();
    let lines: Vec<&str> = stripped.iter().map(String::as_str).collect();
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
    let mut next = 0;
    for i in keep {
        if i > next {
            out.push(format!("… {} lines cut …", i - next));
        }
        out.push(cap_line(lines[i]));
        next = i + 1;
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
        assert_eq!(
            squash_log(&numbered(10), 0, 3, 5),
            "… 7 lines cut …\nl8\nl9\nl10"
        );
        assert_eq!(squash_log(&numbered(10), 0, 0, 5), "");
    }

    #[test]
    fn error_words_match_real_failures_only() {
        for l in [
            "error[E0308]: mismatched types",
            "error: could not compile",
            "an error occurred",
            "test a ... FAILED",
            "FAIL src/x.test.ts",
            "thread 'x' panicked at src/a.rs:1",
            "✗ gate",
            "##[error]Process completed with exit code 1.",
        ] {
            assert!(is_error_line(l), "{l}");
        }
        for l in [
            "all fine",
            "test x_errors ... ok",
            "test result: ok. 3 passed; 0 failed",
            "PASS src/x.test.ts",
            "terror here",
            "test a ... ok",
        ] {
            assert!(!is_error_line(l), "{l}");
        }
    }

    #[test]
    fn passing_test_names_with_error_are_not_kept() {
        let mut v: Vec<String> = (1..=300).map(|i| format!("l{i}")).collect();
        for l in v.iter_mut().skip(30).take(40) {
            *l = "test x_errors ... ok".into();
        }
        v[149] = "error[E0308]: mismatched types".into();
        let out = squash_log(&v.join("\n"), 2, 2, 20);
        assert_eq!(
            out,
            "l1\nl2\n… 147 lines cut …\nerror[E0308]: mismatched types\n… 148 lines cut …\nl299\nl300"
        );
    }

    #[test]
    fn ansi_sequences_are_stripped() {
        assert_eq!(
            strip_ansi("\x1b[1m\x1b[31merror\x1b[0m\x1b[1m[E0308]\x1b[0m: x"),
            "error[E0308]: x"
        );
        assert_eq!(
            strip_ansi("plain ✗ \x1b[2K\x1b[38;5;9mred\x1b[m"),
            "plain ✗ red"
        );
        assert_eq!(strip_ansi("cut\x1b[31"), "cut");
    }

    #[test]
    fn colored_error_lines_are_found_and_output_is_stripped() {
        let mut v: Vec<String> = (1..=300).map(|i| format!("l{i}")).collect();
        v[99] = "\x1b[1m\x1b[31merror\x1b[0m\x1b[1m[E0308]\x1b[0m: mismatched".into();
        v[120] = "test a ... \x1b[31mFAILED\x1b[0m".into();
        v[130] = "test b ... \x1b[32mok\x1b[0m".into();
        v[140] = "\x1b[32mtest x_errors ... ok\x1b[0m".into();
        let out = squash_log(&v.join("\n"), 2, 2, 20);
        assert_eq!(
            out,
            "l1\nl2\n… 97 lines cut …\nerror[E0308]: mismatched\n… 20 lines cut …\ntest a ... FAILED\n… 177 lines cut …\nl299\nl300"
        );
        assert!(!out.contains('\x1b'));
    }

    #[test]
    fn long_lines_are_capped_at_500_chars() {
        let long = "x".repeat(600);
        let out = squash_log(&format!("{long}\nshort"), 5, 5, 20);
        let first = out.lines().next().unwrap();
        assert_eq!(first.chars().count(), 501);
        assert!(first.ends_with('…'));
        let exact = "y".repeat(500);
        assert_eq!(squash_log(&exact, 1, 1, 0), exact);
        let multi = "é".repeat(700);
        assert_eq!(squash_log(&multi, 1, 1, 0).chars().count(), 501);
    }

    #[test]
    fn lines_cut_before_the_first_kept_line_get_a_leading_marker() {
        let mut v: Vec<String> = (1..=10).map(|i| format!("l{i}")).collect();
        v[3] = "error: boom".into();
        assert_eq!(
            squash_log(&v.join("\n"), 0, 2, 20),
            "… 3 lines cut …\nerror: boom\n… 4 lines cut …\nl9\nl10"
        );
    }
}
