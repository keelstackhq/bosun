//! The rust test adapters (spec §7): which files are tests, which tests are pending, and how many
//! assertions a file holds. Pure functions on text, so test-integrity can compare base and head.

use regex::Regex;

use super::profile::RegexKeys;
use crate::core::glob::Glob;

/// A `#[ignore` line. `uc` is set when the reason is a use-case id (`#[ignore = "UC-M1-2"]`):
/// that marks a pending UC test (legacy plan stubs, #179), not a skip.
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

/// The number of test attributes: `#[test]`, `#[tokio::test]` (with or without arguments) and
/// `#[rstest]`. Attributes in `//` comments do not count.
pub fn test_count(text: &str) -> usize {
    text.lines()
        .map(|l| code(l).trim_start())
        .filter(|l| {
            ["#[test]", "#[tokio::test", "#[rstest"]
                .iter()
                .any(|p| l.starts_with(p))
        })
        .count()
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

/// The generic `regex` adapters (spec §7), for stacks without built-in adapters. The patterns come
/// from the profile's `[adapters.regex]`. They match raw text: `//` comments are not stripped (the
/// patterns know the language), and there is no test counter, so `fewer-tests` never fires.
#[derive(Debug)]
pub struct RegexKit {
    /// Repo-relative stack directory with a trailing `/`, or empty for the repo root.
    prefix: String,
    test_files: Vec<Glob>,
    pending: Vec<Regex>,
    assertions: Vec<Regex>,
    uc: Regex,
}

impl RegexKit {
    /// Compile the profile's patterns; the error names the bad key. `stack_dir` is the stack path
    /// relative to the repo (`py`, `py/` or empty); test file globs are relative to it.
    pub fn new(keys: &RegexKeys, stack_dir: &str) -> Result<RegexKit, String> {
        let compile = |key: &str, list: &[String]| -> Result<Vec<Regex>, String> {
            list.iter()
                .map(|p| Regex::new(p).map_err(|e| format!("adapters.regex.{key} `{p}`: {e}")))
                .collect()
        };
        let test_files = keys
            .test_files
            .iter()
            .map(|g| Glob::parse(g).map_err(|e| format!("adapters.regex.test_files: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        if test_files.is_empty() {
            return Err("adapters.regex.test_files is empty: no file would count as a test".into());
        }
        let dir = stack_dir.trim_start_matches("./").trim_end_matches('/');
        Ok(RegexKit {
            prefix: if dir == "." || dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            },
            test_files,
            pending: compile("pending", &keys.pending)?,
            assertions: compile("assertions", &keys.assertions)?,
            uc: Regex::new(r"UC-[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*").expect("static pattern"),
        })
    }

    /// A repo-relative path under the stack that one of the `test_files` globs matches.
    pub fn is_test_file(&self, path: &str) -> bool {
        path.strip_prefix(&self.prefix)
            .is_some_and(|rel| self.test_files.iter().any(|g| g.matches(rel)))
    }

    /// Every line that matches a `pending` regex; a `UC-…` id on the line makes it a plan marker.
    pub fn pending_markers(&self, text: &str) -> Vec<Marker> {
        text.lines()
            .enumerate()
            .filter(|(_, l)| self.pending.iter().any(|r| r.is_match(l)))
            .map(|(i, l)| Marker {
                line: i + 1,
                uc: self.uc.find(l).map(|m| m.as_str().to_string()),
            })
            .collect()
    }

    /// The number of matches of all `assertions` regexes.
    pub fn assertions(&self, text: &str) -> usize {
        self.assertions
            .iter()
            .map(|r| r.find_iter(text).count())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn py_kit(dir: &str) -> RegexKit {
        let keys = RegexKeys {
            test_files: vec!["**/test_*.py".into(), "tests/**".into()],
            pending: vec![r"@pytest\.mark\.skip".into(), r"^\s*@unittest\.skip".into()],
            assertions: vec![r"\bassert\b".into(), r"self\.assert\w+".into()],
        };
        RegexKit::new(&keys, dir).unwrap()
    }

    #[test]
    fn regex_test_files_are_relative_to_the_stack() {
        let k = py_kit("py/");
        assert!(k.is_test_file("py/test_a.py"));
        assert!(k.is_test_file("py/pkg/test_b.py"));
        assert!(k.is_test_file("py/tests/data.txt"));
        assert!(!k.is_test_file("py/app.py"));
        assert!(!k.is_test_file("other/test_a.py"));
        assert!(py_kit("").is_test_file("test_a.py"));
        assert!(py_kit(".").is_test_file("x/test_a.py"));
    }

    #[test]
    fn regex_pending_markers_carry_a_uc_id() {
        let text = "@pytest.mark.skip\ndef a(): pass\n@pytest.mark.skip(reason=\"UC-M7-3\")\n  @unittest.skip('x')\n# pytest.mark.skipif is not it\n";
        let m = py_kit("py").pending_markers(text);
        let lines: Vec<(usize, Option<&str>)> =
            m.iter().map(|m| (m.line, m.uc.as_deref())).collect();
        assert_eq!(lines, [(1, None), (3, Some("UC-M7-3")), (4, None)]);
    }

    #[test]
    fn regex_assertions_count_every_match() {
        let text = "assert a\nassert b and assert c\nself.assertEqual(1, 1)\nassertion = 1\n";
        assert_eq!(py_kit("py").assertions(text), 4);
    }

    #[test]
    fn bad_regex_keys_are_errors_naming_the_key() {
        let mut keys = RegexKeys {
            test_files: vec!["*.py".into()],
            ..RegexKeys::default()
        };
        keys.assertions = vec!["(".into()];
        let e = RegexKit::new(&keys, "").unwrap_err();
        assert!(e.contains("adapters.regex.assertions"), "{e}");
        keys.assertions.clear();
        keys.test_files = vec!["a/../b".into()];
        assert!(RegexKit::new(&keys, "").is_err());
        keys.test_files.clear();
        assert!(RegexKit::new(&keys, "").unwrap_err().contains("test_files"));
    }

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
    fn plan_import_stubs_are_uc_pending_markers() {
        let text = crate::core::plan::uc_stub("UC-M2-3", "each line has a test.");
        let m = pending_markers(&text);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].uc.as_deref(), Some("UC-M2-3"));
        assert_eq!(test_count(&text), 1);
    }

    #[test]
    fn counts_test_attributes() {
        let text = "#[test]\nfn a() {}\n  #[tokio::test(flavor = \"x\")]\nasync fn b() {}\n#[rstest]\n// #[test]\n#[testing]\n";
        assert_eq!(test_count(text), 3);
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
