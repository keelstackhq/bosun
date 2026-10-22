//! Touch-path globs (spec §4): repo-relative, `*` and `?` inside one segment, `**` across
//! segments. A path ending in `/` means everything below it.

/// A parsed glob: one pattern per path segment; `**` is its own segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    text: String,
    segs: Vec<String>,
}

impl Glob {
    /// Parse a touch path. Leading `./` and `/` are dropped; `dir/` becomes `dir/**`.
    /// Empty globs, `..` segments and braces are refused (the error says why).
    pub fn parse(raw: &str) -> Result<Glob, String> {
        let mut text = raw
            .trim()
            .trim_start_matches("./")
            .trim_start_matches('/')
            .to_string();
        if text.ends_with('/') {
            text.push_str("**");
        }
        if text.is_empty() {
            return Err(format!("touch glob `{raw}` is empty"));
        }
        if text.contains(['{', '}', '[', ']', '\\']) {
            return Err(format!(
                "touch glob `{raw}` uses braces, classes or escapes; use * ? ** only"
            ));
        }
        let segs: Vec<String> = text
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(str::to_string)
            .collect();
        if segs.iter().any(|s| s == "..") {
            return Err(format!("touch glob `{raw}` leaves the repo with `..`"));
        }
        if segs.iter().any(|s| s.contains("**") && s != "**") {
            return Err(format!("touch glob `{raw}`: `**` must be a whole segment"));
        }
        Ok(Glob { text, segs })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Does this glob match a repo-relative file path?
    pub fn matches(&self, path: &str) -> bool {
        let parts: Vec<&str> = path
            .trim_start_matches("./")
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        match_segs(&self.segs, &parts)
    }

    /// Can some path match both globs?
    pub fn overlaps(&self, other: &Glob) -> bool {
        overlap_segs(&self.segs, &other.segs)
    }
}

/// Parse a list of touch paths; the first bad one is the error.
pub fn parse_all(raw: &[String]) -> Result<Vec<Glob>, String> {
    raw.iter().map(|r| Glob::parse(r)).collect()
}

/// The first pair (a, b) of globs that overlap, as text.
pub fn first_overlap(a: &[Glob], b: &[Glob]) -> Option<(String, String)> {
    a.iter()
        .flat_map(|x| b.iter().map(move |y| (x, y)))
        .find(|(x, y)| x.overlaps(y))
        .map(|(x, y)| (x.text.clone(), y.text.clone()))
}

fn match_segs(pat: &[String], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((p, rest)) if p == "**" => {
            (0..=path.len()).any(|skip| match_segs(rest, &path[skip..]))
        }
        Some((p, rest)) => match path.split_first() {
            Some((s, tail)) => seg_match(p.as_bytes(), s.as_bytes()) && match_segs(rest, tail),
            None => false,
        },
    }
}

/// `*` and `?` inside one segment against a literal segment.
fn seg_match(p: &[u8], s: &[u8]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|skip| seg_match(rest, &s[skip..])),
        Some((c, rest)) => match s.split_first() {
            Some((d, tail)) => (*c == b'?' || c == d) && seg_match(rest, tail),
            None => false,
        },
    }
}

fn overlap_segs(a: &[String], b: &[String]) -> bool {
    match (a.split_first(), b.split_first()) {
        (None, None) => true,
        (Some((x, ra)), _) if x == "**" => {
            overlap_segs(ra, b) || (!b.is_empty() && overlap_segs(a, &b[1..]))
        }
        (_, Some((y, rb))) if y == "**" => {
            overlap_segs(a, rb) || (!a.is_empty() && overlap_segs(&a[1..], b))
        }
        (Some((x, ra)), Some((y, rb))) => {
            seg_overlap(x.as_bytes(), y.as_bytes()) && overlap_segs(ra, rb)
        }
        _ => false,
    }
}

/// Do two segment patterns share a matching non-empty string?
fn seg_overlap(p: &[u8], q: &[u8]) -> bool {
    match (p.split_first(), q.split_first()) {
        (None, None) => true,
        (Some((b'*', rp)), _) => seg_overlap(rp, q) || (!q.is_empty() && seg_overlap(p, &q[1..])),
        (_, Some((b'*', rq))) => seg_overlap(p, rq) || (!p.is_empty() && seg_overlap(&p[1..], q)),
        (Some((c, rp)), Some((d, rq))) => {
            (*c == b'?' || *d == b'?' || c == d) && seg_overlap(rp, rq)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(s: &str) -> Glob {
        Glob::parse(s).unwrap()
    }

    #[test]
    fn matches_table() {
        let cases = [
            ("src/**", "src/a.rs", true),
            ("src/**", "src/a/b/c.rs", true),
            ("src/**", "srcx/a.rs", false),
            ("src/*.rs", "src/a.rs", true),
            ("src/*.rs", "src/a/b.rs", false),
            ("**/*.rs", "a.rs", true),
            ("**/*.rs", "x/y/a.rs", true),
            ("src/?.rs", "src/a.rs", true),
            ("src/?.rs", "src/ab.rs", false),
            ("docs/", "docs/spec.md", true),
            ("./Cargo.toml", "Cargo.toml", true),
            ("a/**/z", "a/z", true),
            ("a/**/z", "a/b/c/z", true),
        ];
        for (pat, path, want) in cases {
            assert_eq!(g(pat).matches(path), want, "{pat} vs {path}");
        }
    }

    #[test]
    fn overlap_table() {
        let cases = [
            ("src/**", "src/a/*.rs", true),
            ("src/**", "tests/**", false),
            ("a/*/c", "a/b/*", true),
            ("a/*/c", "a/b/d", false),
            ("src/core/lease.rs", "src/core/lease.rs", true),
            ("src/core/lease.rs", "src/core/glob.rs", false),
            ("src/*.rs", "src/lib.rs", true),
            ("src/*.rs", "src/cli/mod.rs", false),
            ("**", "anything/at/all", true),
            ("**/mod.rs", "src/cli/*", true),
            ("*.md", "docs/*.md", false),
            ("a*b", "ab*", true),
            ("a*b", "c*", false),
            ("x?", "?y", true),
            ("x?", "?", false),
            ("docs/", "docs/spec.md", true),
            ("src/**/test.rs", "src/a/**", true),
            ("src/**/x.rs", "src/**/y.rs", false),
            ("*", "src/a.rs", false),
            ("**/*.toml", "Cargo.toml", true),
        ];
        for (a, b, want) in cases {
            assert_eq!(g(a).overlaps(&g(b)), want, "{a} vs {b}");
            assert_eq!(g(b).overlaps(&g(a)), want, "{b} vs {a} (symmetric)");
        }
    }

    #[test]
    fn bad_globs_are_refused() {
        for bad in ["", "  ", "../x", "src/{a,b}.rs", "a/**b", "[ab].rs"] {
            assert!(Glob::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn first_overlap_names_the_pair() {
        let a = parse_all(&["docs/**".into(), "src/core/*.rs".into()]).unwrap();
        let b = parse_all(&["tests/**".into(), "src/**".into()]).unwrap();
        assert_eq!(
            first_overlap(&a, &b),
            Some(("src/core/*.rs".into(), "src/**".into()))
        );
        assert_eq!(first_overlap(&a[..1], &b), None);
    }
}
