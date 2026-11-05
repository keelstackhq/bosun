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
    /// A prefix rule: kind, pattern (the match or its first group is the token), and a check
    /// on the token that rules out prose such as `sk-fix-the-flaky-test`.
    type Check = fn(&str) -> bool;
    type Rule = (&'static str, Regex, Check);
    static KINDS: OnceLock<Vec<Rule>> = OnceLock::new();
    let kinds = KINDS.get_or_init(|| {
        let rules: [(&str, &str, Check); 6] = [
            (
                "GitHub token",
                r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})",
                |_| true,
            ),
            ("API key", r"\b(sk-(?:ant-)?[A-Za-z0-9_-]{20,})", |k| {
                k.chars().any(|c| c.is_ascii_digit()) || k.matches('-').count() <= 2
            }),
            ("AWS access key", r"\bAKIA[0-9A-Z]{16}\b", |_| true),
            ("Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}", |_| true),
            (
                "bearer token",
                r"(?i)\bbearer\s+([A-Za-z0-9._~+/=-]{20,})",
                |k| {
                    k.chars().any(|c| c.is_ascii_digit())
                        && k.chars().any(|c| c.is_ascii_alphabetic())
                },
            ),
            (
                "JWT",
                r"\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}",
                |_| true,
            ),
        ];
        rules
            .into_iter()
            .map(|(k, re, ok)| (k, Regex::new(re).expect("secret regex"), ok))
            .collect()
    });
    for (kind, re, ok) in kinds {
        for c in re.captures_iter(t) {
            let token = c.get(1).or_else(|| c.get(0)).map_or("", |m| m.as_str());
            if ok(token) {
                return Some(kind);
            }
        }
    }
    t.split(|c: char| !(c.is_ascii_alphanumeric() || "+/_=-".contains(c)))
        .any(|w| {
            if is_digest_word(w) {
                return false;
            }
            high_entropy(w)
                || w.split_once('=')
                    .is_some_and(|(_, v)| !is_digest_value(w, v) && high_entropy(v))
        })
        .then_some("high-entropy token")
}

/// A bare base64 hash digest: exactly 28, 44, 64 or 88 characters (sha1, sha256, sha384,
/// sha512) with the padding that length needs and nothing else.
fn is_b64_digest(v: &str) -> bool {
    let body = v.trim_end_matches('=');
    let pad = v.len() - body.len();
    let want = match v.len() {
        28 | 44 => 1,
        64 => 0,
        88 => 2,
        _ => return false,
    };
    pad == want
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
}

/// A public integrity value `sha1|sha256|sha384|sha512-<digest>` and nothing more.
fn is_integrity(w: &str) -> bool {
    ["sha1-", "sha256-", "sha384-", "sha512-"]
        .iter()
        .find_map(|p| w.strip_prefix(p))
        .is_some_and(is_b64_digest)
}

/// An integrity value, or a `NAME=value` word whose name says it holds a digest.
fn is_digest_word(w: &str) -> bool {
    is_integrity(w)
        || w.split_once('=')
            .is_some_and(|(n, v)| is_digest_value(n, v))
}

/// `v` is a digest (or integrity value) held by a variable named `name` that mentions a hash.
fn is_digest_value(name: &str, v: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["SHA", "HASH", "INTEGRITY", "DIGEST", "CHECKSUM"]
        .iter()
        .any(|k| n.contains(k))
        && (is_b64_digest(v) || is_integrity(v))
}

/// A 32+ character word of base64/URL-safe characters that looks random: hex of a length other
/// than a git sha's (40, 64), or letters and digits with Shannon entropy of at least 4.0 bits
/// per character. Not a session id (`session_`, `cse_`), an integrity hash (see `is_integrity`), a
/// snake or kebab case phrase (3+ lowercase words) or a lowercase path.
fn high_entropy(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if w.len() < 32 || ["session_", "cse_"].iter().any(|p| w.starts_with(p)) {
        return false;
    }
    let hex = w.chars().all(|c| c.is_ascii_hexdigit());
    if hex && matches!(w.len(), 40 | 64) {
        return false;
    }
    let has = |f: fn(&char) -> bool| w.chars().any(|c| f(&c));
    let (upper, lower) = (has(char::is_ascii_uppercase), has(char::is_ascii_lowercase));
    if !(upper || lower) || !has(char::is_ascii_digit) {
        return false;
    }
    let words = w
        .split(['_', '-'])
        .filter(|s| s.len() >= 2 && s.chars().all(|c| c.is_ascii_lowercase()))
        .count();
    if words >= 3 || (w.contains('/') && !(upper && lower)) {
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
    // Hex has at most 4 bits per character, so its bar is lower.
    entropy >= if hex { 3.0 } else { 4.0 }
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
    fn secret_kind_catches_more_forms() {
        let hex48 = "0123456789abcdef".repeat(3);
        let aws = format!("wJalrXUtnFEMI/K7MDENG/{}", "bPxRfiCYEXAMPLEKEY");
        let jwt = format!(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.{}",
            "abcDEF123xyz"
        );
        let key = format!("key={}", "q8Zr3KxP0vLm5NwT2bYc7HdJ4sFg9AeU");
        for (text, kind) in [
            (hex48, "high-entropy token"),
            (aws, "high-entropy token"),
            (jwt, "JWT"),
            (
                format!("password:{}", "q8Zr3KxP0vLm5NwT2bYc7HdJ4sFg9AeU"),
                "high-entropy token",
            ),
            (format!("https://x.test/?{key}"), "high-entropy token"),
            ("bearer abcdefghij0123456789xy".to_string(), "bearer token"),
        ] {
            assert_eq!(secret_kind(&text), Some(kind), "{kind}");
        }
    }

    #[test]
    fn secret_kind_passes_known_lookalikes() {
        let integrity = format!("sha512-{}==", b64(86));
        for ok in [
            "session_01SsFUkfeX2mg29KV4Lhoi5P",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "cse_01SsFUkfeX2mg29KV4Lhoi5P",
            "E0308_mismatched_types_in_src_cli_handoff_rs_line_72",
            "sk-fix-the-flaky-integration-test",
            "Bearer tokens/credentials-are-never-in-argv-anymore",
            integrity.as_str(),
        ] {
            assert_eq!(secret_kind(ok), None, "{ok}");
        }
    }

    fn b64(n: usize) -> String {
        let a = "q8Zr3KxP0vLm5NwT2bYc7HdJ4sFg9AeUZ1Xo6RiEt/+";
        a.chars().cycle().take(n).collect()
    }

    #[test]
    fn secret_kind_passes_integrity_values_and_digest_variables() {
        let d88 = format!("{}==", b64(86));
        let d44 = format!("{}=", b64(43));
        let d64 = b64(64);
        let d28 = format!("{}=", b64(27));
        for ok in [
            format!("sha512-{d88}"),
            format!("sha256-{d44}"),
            format!("sha384-{}", b64(64)),
            format!("sha1-{d28}"),
            format!("BOSUN_SHA512={d88}"),
            format!("bosun_sha256={d44}"),
            format!("FILE_INTEGRITY={d88}"),
            format!("RELEASE_CHECKSUM={d64}"),
            format!("PKG_DIGEST={d28}"),
            format!("MY_HASH={d44}"),
        ] {
            assert_eq!(secret_kind(&ok), None, "{ok}");
        }
    }

    #[test]
    fn secret_kind_still_refuses_near_integrity_shapes() {
        let d44 = format!("{}=", b64(43));
        for bad in [
            format!("API_KEY={d44}"),
            format!("API_KEY={}", b64(40)),
            format!("SECRET={}==", b64(86)),
            format!("sha512-{}x!extra", b64(86)),
            format!("sha512-{}extra", b64(86)),
            format!("HASH_TOKEN_KEY={}", b64(40)),
            format!("MY_HASH={}", b64(45)),
        ] {
            assert!(secret_kind(&bad).is_some(), "{bad}");
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
