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
}
