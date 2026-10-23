//! The ts test adapters (spec §7): test files, pending markers, test and assertion counts for
//! TypeScript. Pure functions on text, like the rust adapters.

use super::adapters::Marker;

const EXTS: [&str; 4] = [".test.ts", ".spec.ts", ".test.tsx", ".spec.tsx"];

/// The text of a line without its `//` comment.
fn code(line: &str) -> &str {
    line.find("//").map_or(line, |i| &line[..i])
}

/// Occurrences of `needle` in `line` that do not continue an identifier (`exit(` is not `xit(`).
fn count_word(line: &str, needle: &str) -> usize {
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    line.match_indices(needle)
        .filter(|(i, _)| *i == 0 || !is_ident(line.as_bytes()[i - 1]))
        .count()
}

/// Total occurrences of any of `pats` in the code part of every line.
fn count_all(text: &str, pats: &[&str]) -> usize {
    text.lines()
        .map(code)
        .map(|l| pats.iter().map(|p| count_word(l, p)).sum::<usize>())
        .sum()
}

/// `*.test.ts`, `*.spec.ts`, `*.test.tsx`, `*.spec.tsx`, and any `.ts`/`.tsx` under `__tests__/`.
pub fn is_test_file(path: &str) -> bool {
    EXTS.iter().any(|e| path.ends_with(e))
        || ((path.ends_with(".ts") || path.ends_with(".tsx"))
            && (path.starts_with("__tests__/") || path.contains("/__tests__/")))
}

/// New-skip candidates: `it.skip(`, `test.skip(`, `describe.skip(`, `it.todo(`, `test.todo(`,
/// `xit(`, `xtest(`, `xdescribe(`. `.only(` is not a skip. TypeScript has no use-case marker, so
/// `uc` is always none.
pub fn pending_markers(text: &str) -> Vec<Marker> {
    const PATS: [&str; 8] = [
        "it.skip(",
        "test.skip(",
        "describe.skip(",
        "it.todo(",
        "test.todo(",
        "xit(",
        "xtest(",
        "xdescribe(",
    ];
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        for _ in 0..count_all(raw, &PATS) {
            out.push(Marker {
                line: i + 1,
                uc: None,
            });
        }
    }
    out
}

/// The number of test declarations outside `//` comments: `it(`, `test(`, their `.each`, `.skip(`,
/// `.only(` and `.todo(` forms, and `xit(`/`xtest(`. A skipped test still counts, so skipping is
/// reported as a new skip and not also as a lost test.
pub fn test_count(text: &str) -> usize {
    count_all(
        text,
        &[
            "it(",
            "test(",
            "it.each",
            "test.each",
            "it.skip(",
            "test.skip(",
            "it.only(",
            "test.only(",
            "it.todo(",
            "test.todo(",
            "xit(",
            "xtest(",
        ],
    )
}

/// The number of `expect(`, `assert(` and `assert.` uses outside `//` comments.
pub fn assertions(text: &str) -> usize {
    count_all(text, &["expect(", "assert(", "assert."])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files() {
        for p in [
            "a.test.ts",
            "src/a.spec.tsx",
            "src/__tests__/helper.ts",
            "__tests__/x.tsx",
        ] {
            assert!(is_test_file(p), "{p}");
        }
        for p in [
            "src/a.ts",
            "a.test.js",
            "src/__tests__/data.json",
            "a.test.ts.snap",
        ] {
            assert!(!is_test_file(p), "{p}");
        }
    }

    #[test]
    fn skips_are_found_and_only_is_not() {
        let text = "it.skip('a', f);\n  test.skip(\"b\", f);\ndescribe.skip('c', f);\n\
                    it.todo('d');\ntest.todo('e');\nxit('f', f);\nxdescribe('g', f);\n\
                    it.only('h', f);\ndescribe.only('i', f);\n// it.skip('j')\nexit(1);\nit('k', f);\n";
        let m = pending_markers(text);
        assert_eq!(m.len(), 7);
        assert_eq!(m[0].line, 1);
        assert!(m.iter().all(|m| m.uc.is_none()));
    }

    #[test]
    fn counts_assertions_and_tests() {
        let text = "it('a', () => {\n expect(x).toBe(1);\n assert(y);\n assert.equal(a, b);\n\
                    // expect(z)\n expectTypeOf(q);\n});\ntest('b', f);\nmyexpect(1);\n";
        assert_eq!(assertions(text), 3);
        assert_eq!(test_count(text), 2);
        assert_eq!(
            test_count("it.skip('a', f);\nxit('b', f);\nit.only('c', f);\n"),
            3
        );
        assert_eq!(assertions(&text.replace("expect(x).toBe(1);", "")), 2);
    }
}
