//! Edit-lock for Bash (spec §4): the files a command plainly writes, so the hook can judge
//! them like an Edit. Only literal paths count; the rest is a known miss.

use super::{
    RUNNERS, SEP, WRAPPERS, base, literal_path, shell_words, split_heredocs, unwrap_runners,
};

/// Whether a word is a redirection: `>f`, `>>f`, `N>f`, `&>f`. A word with other text before
/// the `>` is an argument whose quotes are gone (`s/a>b/c/`, `-m 'a > b'`), not a redirection.
fn is_redirect(word: &str) -> bool {
    word.find('>').is_some_and(|at| {
        let lead = &word[..at];
        !word.contains(char::is_whitespace)
            && (lead == "&" || lead.bytes().all(|c| c.is_ascii_digit()))
    })
}

/// A `>` inside quotes or after a backslash, which `shell_words` would no longer tell apart.
const QUOTED_GT: char = '\u{1}';

/// The command with every quoted or escaped `>` replaced by `QUOTED_GT`, so only a real
/// redirection keeps its `>` (`grep '>' f` reads, `echo '>x'` writes nothing).
fn mask_quoted(command: &str) -> String {
    let (mut out, mut quote, mut chars) = (String::new(), None, command.chars());
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (Some('\''), c) if c != '>' => {}
            (_, '\\') => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(if n == '>' { QUOTED_GT } else { n });
                }
                continue;
            }
            (Some(_), '>') => {
                out.push(QUOTED_GT);
                continue;
            }
            _ => {}
        }
        out.push(c);
    }
    out
}

/// The file a redirection word writes: the rest of the word, or the next word.
fn redirect<'a>(word: &'a str, next: Option<&'a String>) -> Option<&'a str> {
    if !is_redirect(word) {
        return None;
    }
    let at = word.find('>')?;
    match word[at..].trim_start_matches('>') {
        "" => next.map(String::as_str),
        r => Some(r),
    }
}

/// The command word of a segment and its arguments, after assignments, wrappers (`if`, `then`,
/// `{`, a leading `(`) and runners with their options (`sudo -u x`, `time`, `env`); and whether
/// a runner changes directory (`env -C dir`, `sudo -D dir`).
fn command_of(seg: &[String]) -> (&str, Vec<&str>, bool) {
    let all: Vec<&str> = seg.iter().map(|w| w.trim_start_matches('(')).collect();
    let mut cmd: &[&str] = &all;
    let mut moved = false;
    loop {
        let wrapper = |w: &str| {
            (w.contains('=') && !w.starts_with('-'))
                || w.is_empty()
                || (WRAPPERS.contains(&w) && !RUNNERS.iter().any(|r| r.0 == base(w)))
        };
        let skip = cmd.iter().take_while(|w| wrapper(w)).count();
        let (rest, dirs, _) = unwrap_runners(&cmd[skip..]);
        moved |= !dirs.is_empty();
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
            moved,
        ),
        _ => ("", Vec::new(), moved),
    }
}

/// The paths that `> f`, `>> f`, `N> f`, `tee f`, `cp|mv|install .. DEST` and `sed -i .. FILES`
/// write. A heredoc body that no shell or interpreter reads is data; `>&N`, `/dev/*` and
/// expansions are skipped; after a `cd`, `pushd` or `popd` relative targets are skipped (the
/// hook could not tell where they land).
pub fn bash_writes(command: &str) -> Vec<String> {
    let words = shell_words(&mask_quoted(&split_heredocs(command).0));
    let (mut out, mut moved) = (Vec::new(), false);
    for seg in words.split(|w| w == SEP) {
        let mut found: Vec<String> = Vec::new();
        let (cmd, all, here_moved) = command_of(seg);
        let moved_now = moved || here_moved;
        for (i, w) in seg.iter().enumerate() {
            found.extend(redirect(w, seg.get(i + 1)).map(str::to_string));
        }
        if cmd == "tee" {
            found.extend(
                all.iter()
                    .find(|a| !a.starts_with('-') && !is_redirect(a))
                    .map(|s| s.to_string()),
            );
        }
        // Operands end at the first redirection (`2>/dev/null`, `>&2`, `> f`).
        let args: Vec<&str> = all
            .iter()
            .copied()
            .take_while(|a| !is_redirect(a))
            .collect();
        let plain: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !a.starts_with('-'))
            .collect();
        let target_dir = args.iter().any(|a| {
            a.starts_with("--target-directory")
                || (a.starts_with('-') && !a.starts_with("--") && a.contains('t'))
        });
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
                .filter(|p| !p.starts_with(['&', '(']) && !p.starts_with("/dev/"))
                .map(|p| p.trim_end_matches(')').to_string())
                .filter(|p| !moved_now || p.starts_with('/'))
                .filter_map(|p| literal_path(&p.replace(QUOTED_GT, ">"))),
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
            ("echo 'a > b' > src/a.rs", vec!["src/a.rs"]),
            ("grep '>' x >> 'src/o.rs'", vec!["src/o.rs"]),
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
            (
                "sed -i 's/Vec<String>/Vec<&str>/' src/x.rs",
                vec!["src/x.rs"],
            ),
            ("sed -i 's/->/=>/g' src/x.rs", vec!["src/x.rs"]),
            ("sed -i -e 's/a>b/c/' src/x.rs", vec!["src/x.rs"]),
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
            "cp -rt /tmp src/a.rs",
            "env -C /tmp cp a README.md",
            "sudo -D /tmp cp a README.md",
            "echo \"a>README.md\"",
            "tee >(wc -l)",
            "grep '>' README.md",
            "grep -c '>>' src/a.rs",
            "rg '>' Cargo.toml",
            "echo '>src/a.rs'",
            "git commit -m '>x'",
            "grep \">\" README.md",
            "echo \\>x",
            "echo '2>README.md'",
        ] {
            assert!(bash_writes(cmd).is_empty(), "{cmd}: {:?}", bash_writes(cmd));
        }
    }
}
