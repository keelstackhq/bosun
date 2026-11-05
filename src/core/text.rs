//! Text that bosun shows a person (an issue body, a chat message) when an agent or a plan wrote
//! it: made inert so it cannot hide, reorder or fake lines, links or mentions. Pure.

use std::sync::OnceLock;

use regex::Regex;

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

/// Name of the secret `t` looks like, or `None`. Used to refuse tokens, keys and PEM blocks in
/// text bosun posts or stores (lessons, handoffs, messages, signatures). Never returns the
/// matched text. Pure.
pub fn secret_kind(t: &str) -> Option<&'static str> {
    if t.contains("-----BEGIN ") {
        return Some(if t.contains("PRIVATE KEY-----") {
            "private key"
        } else {
            "PEM block"
        });
    }
    static KINDS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    let kinds = KINDS.get_or_init(|| {
        [
            (
                "GitHub token",
                r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})",
            ),
            ("API key", r"\bsk-(?:ant-)?[A-Za-z0-9_-]{20,}"),
            ("AWS access key", r"\bAKIA[0-9A-Z]{16}\b"),
            ("Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
            ("bearer token", r"\bBearer\s+[A-Za-z0-9._~+/=-]{20,}"),
        ]
        .into_iter()
        .map(|(k, re)| (k, Regex::new(re).expect("secret regex")))
        .collect()
    });
    if let Some((k, _)) = kinds.iter().find(|(_, re)| re.is_match(t)) {
        return Some(k);
    }
    t.split_whitespace()
        .any(high_entropy)
        .then_some("high-entropy token")
}

/// A 32+ character word of base64/URL-safe characters with letters, digits and Shannon entropy
/// of at least 4.0 bits per character; not a git sha and not a path.
fn high_entropy(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if w.len() < 32
        || !w
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/_=-".contains(c))
        || !w.chars().any(|c| c.is_ascii_alphabetic())
        || !w.chars().any(|c| c.is_ascii_digit())
    {
        return false;
    }
    if matches!(w.len(), 40 | 64) && w.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    // A path: short segments between slashes.
    if w.matches('/').count() >= 2 && w.split('/').all(|s| s.len() <= 24) {
        return false;
    }
    let mut counts = std::collections::HashMap::new();
    for c in w.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let n = w.len() as f64;
    let entropy: f64 = counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum();
    entropy >= 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_kind_names_each_kind() {
        let body = "a1B2".repeat(9);
        let cases = [
            (format!("token ghp_{body}"), "GitHub token"),
            (format!("x gho_{body}"), "GitHub token"),
            (format!("x ghs_{body}"), "GitHub token"),
            (format!("x ghu_{body}"), "GitHub token"),
            (format!("x github_pat_{body}"), "GitHub token"),
            (format!("key sk-{body}"), "API key"),
            (format!("key sk-ant-{body}"), "API key"),
            (format!("id AKIA{}", "Q7ZX".repeat(4)), "AWS access key"),
            ("x xoxb-1234567890-abcdefghij".to_string(), "Slack token"),
            (
                format!("-----BEGIN RSA PRIV{}", "ATE KEY-----"),
                "private key",
            ),
            (format!("-----BEGIN CERTIFI{}", "CATE-----"), "PEM block"),
            (format!("Authorization: Bearer {body}"), "bearer token"),
            (
                "q8Zr3KxP0vLm5NwT2bYc7HdJ4sFg9AeU".to_string(),
                "high-entropy token",
            ),
        ];
        for (text, kind) in cases {
            assert_eq!(secret_kind(&text), Some(kind), "{kind}");
        }
    }

    #[test]
    fn secret_kind_passes_ordinary_text() {
        let sha64 = "0123456789abcdef".repeat(4);
        for ok in [
            "Run cargo metadata once per gate run, not once per package.",
            "see src/core/some_long_directory/another_long_directory/file_name_v2.rs",
            "https://github.com/keelstackhq/bosun/pull/1234567890123456789012345",
            "risk-assessment-for-the-task-at-hand-and-more",
            "fixed in 807da0cf2fdc26ac6bcdfd3ad9f862942c2fa55e",
            sha64.as_str(),
            "",
        ] {
            assert_eq!(secret_kind(ok), None, "{ok}");
        }
    }

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
