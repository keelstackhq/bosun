//! Failure signatures for lessons (spec §6): the same failure on another run, with another sha,
//! line number, timestamp or temp directory, gets the same signature.

/// Lines of normalised text that count toward a signature.
const MAX_LINES: usize = 20;

/// True when `s` has the shape 8-4-4-4-12 of hex digits.
fn is_uuid(s: &[char]) -> bool {
    s.len() == 36
        && s.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The length of an RFC 3339 date or timestamp at the start of `s` (`2026-10-23`,
/// `2026-10-23t10:00:00.5z`), 0 when there is none.
fn time_len(s: &[char]) -> usize {
    let digits = |from: usize, n: usize| {
        s.len() >= from + n && s[from..from + n].iter().all(char::is_ascii_digit)
    };
    let at = |i: usize, c: char| s.get(i) == Some(&c);
    if !(digits(0, 4) && at(4, '-') && digits(5, 2) && at(7, '-') && digits(8, 2)) {
        return 0;
    }
    let mut n = 10;
    if (at(n, 't') || at(n, ' ')) && digits(n + 1, 2) && at(n + 3, ':') && digits(n + 4, 2) {
        n += 6;
        if at(n, ':') && digits(n + 1, 2) {
            n += 3;
            if at(n, '.') && digits(n + 1, 1) {
                n += 1;
                while digits(n, 1) {
                    n += 1;
                }
            }
        }
        if at(n, 'z') {
            n += 1;
        } else if (at(n, '+') || at(n, '-'))
            && digits(n + 1, 2)
            && at(n + 3, ':')
            && digits(n + 4, 2)
        {
            n += 6;
        }
    }
    n
}

/// A temp-dir path starts at `s[0]`: `/tmp/...`, or a `/home/...` path with a `tmp` or `.tmp`
/// segment. Returns its length (up to whitespace, a quote or a bracket), 0 when it is not one.
fn tmp_path_len(s: &[char]) -> usize {
    let end = s
        .iter()
        .position(|c| c.is_whitespace() || "\"'`)(<>,;".contains(*c))
        .unwrap_or(s.len());
    let text: String = s[..end].iter().collect();
    let temp = text.starts_with("/tmp/")
        || (text.starts_with("/home/")
            && text
                .split('/')
                .any(|seg| seg.starts_with("tmp") || seg.starts_with(".tmp")));
    if temp { end } else { 0 }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn normalise_line(line: &str) -> String {
    let s: Vec<char> = line.to_lowercase().chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let prev_word = i > 0 && is_word(s[i - 1]);
        if rest[0] == '/' && !prev_word {
            let n = tmp_path_len(rest);
            if n > 0 {
                out.push_str("<tmp>");
                i += n;
                continue;
            }
        }
        if is_word(rest[0]) {
            if !prev_word {
                if is_uuid(&rest[..rest.len().min(36)]) {
                    out.push_str("<id>");
                    i += 36;
                    continue;
                }
                let n = time_len(rest);
                if n > 0 {
                    out.push_str("<time>");
                    i += n;
                    continue;
                }
            }
            let n = rest.iter().position(|&c| !is_word(c)).unwrap_or(rest.len());
            let word = &rest[..n];
            let hex = word.iter().all(char::is_ascii_hexdigit);
            if word.iter().all(char::is_ascii_digit) {
                out.push_str("<n>");
            } else if n >= 7 && hex && word.iter().any(char::is_ascii_digit) {
                out.push_str("<id>");
            } else if word[0].is_ascii_digit() {
                // A number with a suffix (`15ms`): every digit run is `<n>`, the suffix stays.
                let mut prev_digit = false;
                for &c in word {
                    if c.is_ascii_digit() {
                        if !prev_digit {
                            out.push_str("<n>");
                        }
                    } else {
                        out.push(c);
                    }
                    prev_digit = c.is_ascii_digit();
                }
            } else {
                out.extend(word);
            }
            i += n;
            continue;
        }
        out.push(rest[0]);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The failure text with the parts that change between runs replaced: lowercase; hex runs of 7
/// or more characters (with a digit) and UUIDs become `<id>`, RFC 3339 times and dates `<time>`,
/// other digit runs `<n>`, temp-dir paths `<tmp>`. Whitespace is collapsed, empty lines are
/// dropped and only the first 20 non-empty lines are kept.
pub fn normalise(text: &str) -> String {
    text.lines()
        .map(normalise_line)
        .filter(|l| !l.is_empty())
        .take(MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n")
}

/// FNV-1a 64 bit of `kind + "\n" + normalise(text)`, as 16 lowercase hex characters.
pub fn signature(kind: &str, text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{kind}\n{}", normalise(text)).bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

const MARKER_OPEN: &str = "<!-- bosun:lesson ";
const MARKER_CLOSE: &str = " -->";

/// The marker line of a lesson issue body: `<!-- bosun:lesson {"signature":"..","kind":".."} -->`.
pub fn lesson_marker(sig: &str, kind: &str) -> String {
    let json = serde_json::json!({"signature": sig, "kind": kind});
    format!("{MARKER_OPEN}{json}{MARKER_CLOSE}")
}

/// The `(signature, kind)` of the first lesson marker in an issue body.
pub fn parse_lesson_marker(body: &str) -> Option<(String, String)> {
    let start = body.rfind(MARKER_OPEN)? + MARKER_OPEN.len();
    let len = body[start..].find(MARKER_CLOSE)?;
    let v: serde_json::Value = serde_json::from_str(&body[start..start + len]).ok()?;
    Some((
        v.get("signature")?.as_str()?.to_string(),
        v.get("kind")?.as_str()?.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "error: test failed at 2026-10-23T10:00:00Z\n  --> src/core/lease.rs:42:7\nsha 3f9c2ab81d04 in /tmp/.tmpAbC123/work\n";
    const B: &str = "Error: test failed at 2026-11-02T08:15:30.250+01:00\n\n  --> src/core/lease.rs:97:13\nsha 00aa11bb22cc in /tmp/.tmpXyZ999/work\n";

    #[test]
    fn same_failure_with_other_volatile_parts_has_one_signature() {
        assert_eq!(signature("stop-rule", A), signature("stop-rule", B));
        assert_eq!(signature("stop-rule", A).len(), 16);
    }

    #[test]
    fn kind_and_message_change_the_signature() {
        assert_ne!(signature("stop-rule", A), signature("flake", A));
        assert_ne!(
            signature("stop-rule", A),
            signature("stop-rule", "error: another failure")
        );
    }

    #[test]
    fn normalise_replaces_and_trims() {
        let n = normalise("ID 123e4567-e89b-12d3-a456-426614174000 took 15 ms\n\n  \nplain   word");
        assert_eq!(n, "id <id> took <n> ms\nplain word");
        assert_eq!(normalise("defaced e0308"), "defaced e0308");
        let many: String = (0..30).map(|i| format!("line {i}x\n")).collect();
        assert_eq!(normalise(&many).lines().count(), 20);
    }

    #[test]
    fn timings_do_not_change_the_signature() {
        let a = "FAIL [   1.404s] foo::bar\nfinished in 0.06s, took 15ms";
        let b = "FAIL [  12.071s] foo::bar\nfinished in 3.9s, took 207ms";
        assert_eq!(signature("flake", a), signature("flake", b));
        assert_eq!(normalise("took 15ms in 0.06s"), "took <n>ms in <n>.<n>s");
        assert_ne!(
            signature("flake", "error[E0308]: mismatched"),
            signature("flake", "error[E0599]: mismatched")
        );
        assert_eq!(normalise("foo_1 utf8"), "foo_1 utf8");
    }

    #[test]
    fn the_last_marker_wins() {
        let body = format!("{}\n{}", lesson_marker("aa", "x"), lesson_marker("bb", "x"));
        assert_eq!(parse_lesson_marker(&body).unwrap().0, "bb");
    }

    #[test]
    fn marker_round_trips() {
        let m = lesson_marker("00ff00ff00ff00ff", "flake");
        assert_eq!(
            parse_lesson_marker(&format!("text\n```\nx\n```\n{m}\n")),
            Some(("00ff00ff00ff00ff".into(), "flake".into()))
        );
        assert_eq!(parse_lesson_marker("no marker"), None);
    }
}
