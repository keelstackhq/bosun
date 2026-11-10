//! Edit-lock for Bash (spec §4): the files a command plainly writes, so the hook can judge
//! them like an Edit. Only literal paths count; the rest is a known miss.

use super::{
    RUNNERS, SEP, WRAPPERS, base, literal_path, shell_words, split_heredocs, unwrap_runners,
};

/// The file a redirection word writes: `>f`, `>>f`, `N>f`, `&>f`, glued `x>f` (the target is
/// the rest of the word, or the next word when the rest is empty). `None` for other words.
fn redirect<'a>(word: &'a str, next: Option<&'a String>) -> Option<&'a str> {
    if word.contains(char::is_whitespace) {
        return None; // a quoted argument (`-m 'a > b'`), not a redirection
    }
    let at = word.find('>')?;
    let lead = &word[..at];
    let glued = !lead.is_empty() && !lead.bytes().all(|c| c.is_ascii_digit()) && lead != "&";
    let rest = word[at..].trim_start_matches('>');
    match (rest, glued) {
        ("", false) => next.map(String::as_str),
        ("", true) => None,
        (r, _) => Some(r),
    }
}

/// The command word of a segment and its arguments, after assignments, wrappers (`if`, `then`,
/// `{`, a leading `(`) and runners with their options (`sudo -u x`, `time`, `env`).
fn command_of(seg: &[String]) -> (&str, Vec<&str>) {
    let all: Vec<&str> = seg.iter().map(|w| w.trim_start_matches('(')).collect();
    let mut cmd: &[&str] = &all;
    loop {
        let wrapper = |w: &str| {
            (w.contains('=') && !w.starts_with('-'))
                || w.is_empty()
                || (WRAPPERS.contains(&w) && !RUNNERS.iter().any(|r| r.0 == base(w)))
        };
        let skip = cmd.iter().take_while(|w| wrapper(w)).count();
        let (rest, _, _) = unwrap_runners(&cmd[skip..]);
        if rest.len() == cmd.len() {
            break;
        }
        cmd = rest;
    }
    let start = seg.len() - cmd.len();
    match seg.get(start..) {
        Some([c, rest @ ..]) => (
            base(c.trim_start_matches('(')),
            rest.iter().map(String::as_str).collect(),
        ),
        _ => ("", Vec::new()),
    }
}

/// The paths that `> f`, `>> f`, `N> f`, `tee f`, `cp|mv|install .. DEST` and `sed -i .. FILES`
/// write. A heredoc body that no shell or interpreter reads is data; `>&N`, `/dev/*` and
/// expansions are skipped; after a `cd`, `pushd` or `popd` relative targets are skipped (the
/// hook could not tell where they land).
pub fn bash_writes(command: &str) -> Vec<String> {
    let words = shell_words(&split_heredocs(command).0);
    let (mut out, mut moved) = (Vec::new(), false);
    for seg in words.split(|w| w == SEP) {
        let mut found: Vec<String> = Vec::new();
        let (cmd, all) = command_of(seg);
        for (i, w) in seg.iter().enumerate() {
            found.extend(redirect(w, seg.get(i + 1)).map(str::to_string));
        }
        if cmd == "tee" {
            found.extend(
                all.iter()
                    .find(|a| !a.starts_with('-'))
                    .map(|s| s.to_string()),
            );
        }
        // Operands end at the first redirection (`2>/dev/null`, `>&2`, `> f`).
        let args: Vec<&str> = all
            .iter()
            .copied()
            .take_while(|a| !a.contains('>'))
            .collect();
        let plain: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !a.starts_with('-'))
            .collect();
        let target_dir = args
            .iter()
            .any(|a| *a == "-t" || a.starts_with("--target-directory"));
        let in_place = |a: &&str| {
            a.starts_with("--in-place")
                || (a.starts_with('-') && !a.starts_with("--") && a.contains('i'))
        };
        match cmd {
            "cd" | "pushd" | "popd" => moved = true,
            "cp" | "mv" | "install" if plain.len() >= 2 && !target_dir => {
                found.extend(plain.last().map(|s| s.to_string()));
            }
            "sed" if args.iter().any(in_place) => {
                // `-e S`, `-f F` take the next word; without them the first operand is the script.
                let (mut files, mut script, mut skip_next) = (Vec::new(), false, false);
                for a in &args {
                    if std::mem::take(&mut skip_next) {
                        continue;
                    }
                    if matches!(*a, "-e" | "-f" | "--expression" | "--file") {
                        (script, skip_next) = (true, true);
                    } else if a.starts_with("--expression=") || a.starts_with("--file=") {
                        script = true;
                    } else if !a.starts_with('-') {
                        files.push(a.to_string());
                    }
                }
                found.extend(files.into_iter().skip(usize::from(!script)));
            }
            _ => {}
        }
        out.extend(
            found
                .into_iter()
                .filter(|p| !p.starts_with('&') && !p.starts_with("/dev/"))
                .map(|p| p.trim_end_matches(')').to_string())
                .filter(|p| !moved || p.starts_with('/'))
                .filter_map(|p| literal_path(&p)),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_writes_are_found() {
        for (cmd, want) in [
            ("echo x > src/a.rs", vec!["src/a.rs"]),
            ("printf y >> docs/b.md && cat c", vec!["docs/b.md"]),
            ("cargo test 2>&1 | tee out.log", vec!["out.log"]),
            ("cp -r a b src/dest.rs", vec!["src/dest.rs"]),
            ("mv old.rs 'src/new.rs'", vec!["src/new.rs"]),
            (
                "sed -i 's/a/b/' src/x.rs src/y.rs",
                vec!["src/x.rs", "src/y.rs"],
            ),
            ("sed -i -e 's/a/b/' src/x.rs", vec!["src/x.rs"]),
            (
                "FOO=1 sudo install -m 644 f /usr/local/bin/f",
                vec!["/usr/local/bin/f"],
            ),
            ("cp a src/b.rs 2>/dev/null", vec!["src/b.rs"]),
            ("cp a src/b.rs 2>&1", vec!["src/b.rs"]),
            ("cargo build 2> err.log", vec!["err.log"]),
            ("echo x>src/a.rs", vec!["src/a.rs"]),
            ("echo x 1>src/a.rs", vec!["src/a.rs"]),
            ("sed -ni 's/a/b/p' src/x.rs", vec!["src/x.rs"]),
            ("sudo -u bob cp a src/b.rs", vec!["src/b.rs"]),
            ("env cp a src/b.rs", vec!["src/b.rs"]),
            ("time cp a src/b.rs", vec!["src/b.rs"]),
            ("if true; then cp a src/b.rs; fi", vec!["src/b.rs"]),
            ("(cp a src/b.rs)", vec!["src/b.rs"]),
            ("{ cp a src/b.rs; }", vec!["src/b.rs"]),
            ("cd /tmp/w && cp a /abs/b.rs", vec!["/abs/b.rs"]),
        ] {
            assert_eq!(bash_writes(cmd), want, "{cmd}");
        }
    }

    #[test]
    fn reads_redirects_to_streams_and_heredoc_bodies_are_not_writes() {
        for cmd in [
            "cargo test 2>&1",
            "echo x >&2",
            "ls > /dev/null",
            "cat src/a.rs",
            "sed 's/a/b/' src/x.rs",
            "git commit -F - <<'EOF'\necho x > src/a.rs\nEOF",
            "cp $SRC $DEST",
            "cp one",
            "cd /tmp/work && cp a README.md",
            "cd /tmp/probe && cat > Cargo.toml <<'EOF'\nx\nEOF",
            "cd src && echo x > lib.rs",
            "cp -t /tmp src/a.rs",
            "cp --target-directory=/tmp src/a.rs src/b.rs",
            "git commit -m 'a > b'",
            "grep -- '->' src/a.rs",
        ] {
            assert!(bash_writes(cmd).is_empty(), "{cmd}: {:?}", bash_writes(cmd));
        }
    }
}
