//! The rust test adapters (spec §7): which files are tests, which tests are pending, and how many
//! assertions a file holds. Pure functions on text, so test-integrity can compare base and head.

/// A `#[ignore` line. `uc` is set when the reason is a use-case id (`#[ignore = "UC-M1-2"]`):
/// that marks a pending test written by the plan, not a skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub line: usize,
    pub uc: Option<String>,
}

/// The text of a line without its `//` comment.
fn code(line: &str) -> &str {
    line.find("//").map_or(line, |i| &line[..i])
}

/// Test files are `tests/**/*.rs` and any `.rs` file with a `#[test]` or `#[tokio::test]`.
pub fn is_test_file(path: &str, text: &str) -> bool {
    if !path.ends_with(".rs") {
        return false;
    }
    let under_tests = path.starts_with("tests/") || path.contains("/tests/");
    under_tests
        || text.lines().map(code).any(|l| {
            let l = l.trim_start();
            l.starts_with("#[test]") || l.starts_with("#[tokio::test")
        })
}

/// Every `#[ignore` attribute line, with its UC id when it has one.
pub fn pending_markers(text: &str) -> Vec<Marker> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let l = code(raw).trim_start();
        let Some(rest) = l.strip_prefix("#[ignore") else {
            continue;
        };
        if !rest.starts_with(|c: char| c == ']' || c == '=' || c == '(' || c.is_whitespace()) {
            continue;
        }
        let uc = rest
            .split('"')
            .nth(1)
            .filter(|r| r.starts_with("UC-"))
            .map(|r| r.trim().to_string());
        out.push(Marker { line: i + 1, uc });
    }
    out
}

/// The number of assertion macros: `assert*!`, `debug_assert*!`, `prop_assert*!` (which also
/// covers `insta::assert_snapshot!` and its siblings). Matches in `//` comments do not count.
pub fn assertions(text: &str) -> usize {
    // Bytes, not chars: a non-ASCII byte is never part of an identifier, so a slice never ends
    // inside a multi-byte character (`§`, `é`).
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut count = 0;
    for raw in text.lines() {
        let l = code(raw);
        let bytes = l.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !is_ident(bytes[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident(bytes[i]) {
                i += 1;
            }
            let ident = &l[start..i];
            let is_macro = l[i..].trim_start().starts_with('!');
            if is_macro
                && ["assert", "debug_assert", "prop_assert"]
                    .iter()
                    .any(|p| ident.starts_with(p))
            {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files() {
        assert!(is_test_file("tests/cli.rs", ""));
        assert!(is_test_file("a/tests/x/y.rs", "fn f() {}"));
        assert!(is_test_file("src/lib.rs", "  #[test]\nfn t() {}"));
        assert!(is_test_file(
            "src/lib.rs",
            "#[tokio::test]\nasync fn t() {}"
        ));
        assert!(!is_test_file("src/lib.rs", "fn f() {}"));
        assert!(!is_test_file("tests/data.json", "#[test]"));
        assert!(!is_test_file("src/lib.rs", "// #[test]\n"));
    }

    #[test]
    fn markers_split_pending_from_skips() {
        let text = "#[ignore]\nfn a() {}\n    #[ignore = \"UC-M1-2\"]\nfn b() {}\n#[ignore = \"slow\"]\n// #[ignore]\n#[ignored_thing]\n";
        let m = pending_markers(text);
        assert_eq!(
            m,
            vec![
                Marker { line: 1, uc: None },
                Marker {
                    line: 3,
                    uc: Some("UC-M1-2".into())
                },
                Marker { line: 5, uc: None },
            ]
        );
    }

    #[test]
    fn counts_assertions() {
        let text = "assert!(a);\nassert_eq!(a, b);\nassert_ne!(a, b);\ndebug_assert!(a);\n\
                    debug_assert_eq!(a, b);\ndebug_assert_ne!(a, b);\nprop_assert!(a);\n\
                    prop_assert_eq!(a, b);\ninsta::assert_snapshot!(s);\nassert_json_snapshot!(v);\n\
                    assert_debug_snapshot!(v);\n";
        assert_eq!(assertions(text), 11);
    }

    #[test]
    fn ignores_comments_and_lookalikes() {
        let text = "// assert!(a);\nlet x = 1; // assert_eq!(a, b)\nmy_assert!(a);\nassert_it(a);\nlet assert = 1;\nassert!(a);\n";
        assert_eq!(assertions(text), 1);
    }

    #[test]
    fn assertions_survive_multibyte_text() {
        // Found by the gate on its own repo: `§` next to an identifier made a slice panic.
        let text = "// spec §7 says\nlet é§x = 1; assert!(ok, \"§\");\nassert_eq!(a§, b);\n";
        assert_eq!(assertions(text), 2);
    }
}
