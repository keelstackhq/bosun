//! The ts test adapters (spec §7): test files, pending markers, test and assertion counts for
//! TypeScript and JavaScript (jest, vitest, mocha). Pure functions on text, like the rust adapters.
//!
//! Every function first blanks comments and the inside of string and template literals
//! (`strip`), so `// it.skip(`, `"it.skip("` and URLs with `//` do not count.

use super::adapters::Marker;

/// Extensions of the files these adapters read.
const EXTS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Does the ts adapter read this path (by extension)?
pub fn handles(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| EXTS.contains(&e))
}

/// `*.test.<ext>`, `*.spec.<ext>` and any file under `__tests__/`, for the extensions in `EXTS`.
pub fn is_test_file(path: &str) -> bool {
    if !handles(path) {
        return false;
    }
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    stem.ends_with(".test")
        || stem.ends_with(".spec")
        || path.starts_with("__tests__/")
        || path.contains("/__tests__/")
}

/// The text with comments removed and string and template literal contents blanked. Newlines are
/// kept, so line numbers stay. A `'` or `"` string ends at the end of its line.
fn strip(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let at = |i: usize| c.get(i).copied();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '/' if at(i + 1) == Some('/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if at(i + 1) == Some('*') => {
                i += 2;
                while i < c.len() && !(c[i] == '*' && at(i + 1) == Some('/')) {
                    if c[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i += 2;
                out.push(' ');
            }
            '/' if regex_can_start(&out) => {
                // A regex literal: blank it, so quotes inside it open no string.
                out.push('/');
                i += 1;
                let mut class = false;
                while i < c.len() && c[i] != '\n' && (class || c[i] != '/') {
                    match c[i] {
                        '\\' => {
                            out.push(' ');
                            i += 1;
                        }
                        '[' => class = true,
                        ']' => class = false,
                        _ => {}
                    }
                    if i < c.len() && c[i] != '\n' {
                        out.push(' ');
                        i += 1;
                    }
                }
                if at(i) == Some('/') {
                    out.push('/');
                    i += 1;
                }
            }
            q @ ('\'' | '"' | '`') => {
                out.push(q);
                i += 1;
                while i < c.len() && c[i] != q {
                    match c[i] {
                        '\\' => {
                            if at(i + 1) == Some('\n') {
                                out.push('\n');
                            }
                            i += 2;
                            continue;
                        }
                        '\n' if q != '`' => break,
                        '\n' => out.push('\n'),
                        _ => out.push(' '),
                    }
                    i += 1;
                }
                if at(i) == Some(q) {
                    out.push(q);
                    i += 1;
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// Can a `/` after `out` start a regex literal? After `(`, `,`, `=`, `:`, `[`, `!`, `&`, `|`,
/// `?`, `{`, `}`, `;`, the word `return`, or at the start of a line; elsewhere it divides.
fn regex_can_start(out: &str) -> bool {
    let t = out.trim_end_matches([' ', '\t']);
    match t.chars().last() {
        None | Some('\n') => true,
        Some(c) if "(,=:[!&|?{};".contains(c) => true,
        _ => t.strip_suffix("return").is_some_and(|r| {
            !r.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        }),
    }
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Occurrences of `needle` in `line` that do not continue an identifier (`exit(` is not `xit(`).
fn count_word(line: &str, needle: &str) -> usize {
    line.match_indices(needle)
        .filter(|(i, _)| *i == 0 || !is_ident(line.as_bytes()[i - 1]))
        .count()
}

/// One test or suite declaration: `it(`, `test.skip(`, `describe.each(...)(`, `xit(`, ...
struct Call {
    line: usize,
    suite: bool,
    skip: bool,
    only: bool,
}

/// Modifiers that may follow `it`, `test` or `describe` in a declaration.
const MODS: [&str; 14] = [
    "skip",
    "todo",
    "only",
    "skipIf",
    "runIf",
    "concurrent",
    "sequential",
    "each",
    "fails",
    "failing",
    "shuffle",
    "fixme",
    "slow",
    "describe",
];

/// Every declaration in `text`. The name must start at a word boundary and not follow a `.`
/// (`re.test(` is not a test); its `.modifier` chain must be known and end in `(` or a template.
fn calls(text: &str) -> Vec<Call> {
    const WORDS: [&str; 9] = [
        "it",
        "test",
        "describe",
        "suite",
        "xit",
        "xtest",
        "xdescribe",
        "fit",
        "fdescribe",
    ];
    let src = strip(text);
    let mut out = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    for (n, line) in lines.iter().copied().enumerate() {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if !is_ident(b[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            if start > 0 && b[start - 1] == b'.' {
                continue;
            }
            let word = &line[start..i];
            if !WORDS.contains(&word) {
                continue;
            }
            let mut mods: Vec<&str> = Vec::new();
            let mut j = i;
            while j < b.len() && b[j] == b'.' {
                let s = j + 1;
                let mut e = s;
                while e < b.len() && is_ident(b[e]) {
                    e += 1;
                }
                if e == s {
                    break;
                }
                mods.push(&line[s..e]);
                j = e;
            }
            let opens = j < b.len() && (b[j] == b'(' || b[j] == b'`');
            if !opens || mods.iter().any(|m| !MODS.contains(m)) {
                continue;
            }
            // `function fit(x) {}` declares a helper; it is not a test.
            if line[..start].trim_end().ends_with("function") {
                continue;
            }
            // `fit` and `fdescribe` are common names: they count only when called with a title
            // (a string or template literal, possibly on the next line).
            if matches!(word, "fit" | "fdescribe") && mods.is_empty() {
                let rest = line[j..].trim_start_matches('(').trim_start();
                let next = if rest.is_empty() {
                    lines.get(n + 1).map_or("", |l| l.trim_start())
                } else {
                    rest
                };
                if b[j] != b'`' && !next.starts_with(['\'', '"', '`']) {
                    continue;
                }
            }
            out.push(Call {
                line: n + 1,
                suite: matches!(word, "describe" | "suite" | "xdescribe" | "fdescribe")
                    || mods.contains(&"describe"),
                skip: word.starts_with('x')
                    || mods
                        .iter()
                        .any(|m| matches!(*m, "skip" | "todo" | "skipIf" | "runIf" | "fixme")),
                only: word.starts_with('f') || mods.contains(&"only"),
            });
            i = j;
        }
    }
    out
}

/// Skip markers: `it.skip(`, `test.skip(`, `describe.skip(` (also `.skip.each`), `.todo(`,
/// `.skipIf(`, `.runIf(`, `.fixme(`, `test.concurrent.skip(`, `xit(`, `xtest(`, `xdescribe(`.
/// `.only(` is not a skip (see `only_count`). TypeScript has no use-case marker, so `uc` is none.
pub fn pending_markers(text: &str) -> Vec<Marker> {
    calls(text)
        .into_iter()
        .filter(|c| c.skip)
        .map(|c| Marker {
            line: c.line,
            uc: None,
        })
        .collect()
}

/// Focused tests: `it.only(`, `test.only(`, `describe.only(`, `fit(`, `fdescribe(`. Under jest and
/// vitest they silently switch every other test in the file off.
pub fn only_count(text: &str) -> usize {
    calls(text).iter().filter(|c| c.only).count()
}

/// The number of test declarations (`it` and `test` in any form, skipped ones included, so a skip
/// is reported as a new skip and not also as a lost test). Suites are not tests.
pub fn test_count(text: &str) -> usize {
    calls(text).iter().filter(|c| !c.suite).count()
}

/// The number of `expect(`, `assert(` and `assert.` uses in code.
pub fn assertions(text: &str) -> usize {
    let src = strip(text);
    src.lines()
        .map(|l| {
            ["expect(", "assert(", "assert."]
                .iter()
                .map(|p| count_word(l, p))
                .sum::<usize>()
        })
        .sum()
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
            "a.test.js",
            "a.spec.jsx",
            "a.test.mjs",
            "a.test.cjs",
            "a.test.mts",
            "a.spec.cts",
            "src/__tests__/x.js",
        ] {
            assert!(is_test_file(p), "{p}");
        }
        for p in [
            "src/a.ts",
            "src/__tests__/data.json",
            "a.test.ts.snap",
            "a.test.ts.disabled",
            "a.test.md",
            "latest.ts",
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
    fn describe_level_and_conditional_skips() {
        let text = "describe.skipIf(win)('a', f);\ndescribe.skip.each([1])('b', f);\n\
                    describe.todo('c');\nit.runIf(ci)('d', f);\ntest.concurrent.skip('e', f);\n\
                    test.skipIf(x)('f', f);\ntest.describe.skip('g', f);\ntest.fixme('h', f);\n\
                    suite.skip('i', f);\n";
        assert_eq!(pending_markers(text).len(), 9);
    }

    #[test]
    fn comments_and_strings_do_not_count() {
        let text = "/* it.skip('a', f);\n expect(1) */\nconst s = \"it.skip(\";\n\
                    const u = 'http://x/it.skip(';\nconst t = `\n it.skip(\n`;\n\
                    /** doc */ it('real', () => { expect(2); });\n";
        assert!(pending_markers(text).is_empty());
        assert_eq!(assertions(text), 1);
        assert_eq!(test_count(text), 1);
        // Line numbers survive block comments and multi-line templates.
        let m = pending_markers("/*\n*/\nit.skip('a', f);\n");
        assert_eq!(m[0].line, 3);
    }

    #[test]
    fn regex_test_is_not_a_test() {
        let text = "if (re.test(x)) {}\nlet ok = /a/.test(y);\ntest('real', f);\n";
        assert_eq!(test_count(text), 1);
        assert!(pending_markers("re.skip(x)\nobj.it.skip(x)\n").is_empty());
    }

    #[test]
    fn only_is_counted_apart() {
        let text = "it.only('a', f);\ntest.only('b', f);\ndescribe.only('c', f);\nfit('d', f);\n\
                    fdescribe('e', f);\ndescribe.only.each([1])('f', f);\nit('g', f);\n";
        assert_eq!(only_count(text), 6);
        assert!(pending_markers(text).is_empty());
    }

    #[test]
    fn fit_helpers_are_not_focused_tests() {
        let text = "function fit(x) { return x; }\nconst a = fit(width);\nfit(width, 2);\n\
                    export async function fdescribe(n) {}\nit('t', f);\n";
        assert_eq!(only_count(text), 0);
        assert_eq!(test_count(text), 1);
        let titled = "fit('a', f);\nfdescribe(\n  `b`, f);\nfit(\n\"c\", f);\n";
        assert_eq!(only_count(titled), 3);
        assert_eq!(test_count("function it(x) {}\n"), 0);
    }

    #[test]
    fn regex_literals_do_not_open_strings() {
        let text = "const r = /`/;\nit.skip('a', f);\nconst q = x.replace(/[\"']/g, '');\n\
                    it.skip('b', f);\nconst d = a / b; it.skip('c', f); const e = c / 2;\n";
        assert_eq!(pending_markers(text).len(), 3);
        assert_eq!(pending_markers("x = /\\//; it.skip('a', f);\n").len(), 1);
        assert_eq!(pending_markers("x = /[/]`/; it.skip('a', f);\n").len(), 1);
    }

    #[test]
    fn counts_assertions_and_tests() {
        let text = "it('a', () => {\n expect(x).toBe(1);\n assert(y);\n assert.equal(a, b);\n\
                    // expect(z)\n expectTypeOf(q);\n});\ntest('b', f);\nmyexpect(1);\n";
        assert_eq!(assertions(text), 3);
        assert_eq!(test_count(text), 2);
        assert_eq!(assertions(&text.replace("expect(x).toBe(1);", "")), 2);
        assert_eq!(
            test_count("it.skip('a', f);\nxit('b', f);\nit.only('c', f);\ndescribe('d', f);\n"),
            3
        );
    }
}
