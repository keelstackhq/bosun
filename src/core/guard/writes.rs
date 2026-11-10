//! Edit-lock for Bash (spec §4): the files a command plainly writes, so the hook can judge
//! them like an Edit. Only literal paths count; the rest is a known miss.

use super::{SEP, base, literal_path, shell_words, split_heredocs, write_targets};

/// The paths that `> f`, `>> f`, `tee f`, `cp|mv|install .. DEST` and `sed -i .. FILES` write.
/// Heredoc bodies are data; `>&2`, `/dev/*` and expansions are skipped.
pub fn bash_writes(command: &str) -> Vec<String> {
    let words = shell_words(&split_heredocs(command).0);
    let mut out = Vec::new();
    for seg in words.split(|w| w == SEP) {
        let mut found = write_targets(seg);
        let start = seg
            .iter()
            .position(|w| !(w.contains('=') && !w.starts_with('-')) && w != "sudo")
            .unwrap_or(seg.len());
        let (cmd, args) = match seg.get(start..) {
            Some([c, rest @ ..]) => (base(c), rest),
            _ => ("", &[][..]),
        };
        let args: Vec<&String> = args.iter().take_while(|a| !a.starts_with('>')).collect();
        let plain: Vec<&&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        match cmd {
            "cp" | "mv" | "install" if plain.len() >= 2 => {
                found.extend(plain.last().map(|s| s.to_string()));
            }
            "sed"
                if args
                    .iter()
                    .any(|a| a.starts_with("-i") || a.starts_with("--in-place")) =>
            {
                // `-e S`, `-f F` take the next word; without them the first operand is the script.
                let (mut files, mut script, mut skip_next) = (Vec::new(), false, false);
                for a in &args {
                    if std::mem::take(&mut skip_next) {
                        continue;
                    }
                    if matches!(a.as_str(), "-e" | "-f" | "--expression" | "--file") {
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
        ] {
            assert!(bash_writes(cmd).is_empty(), "{cmd}: {:?}", bash_writes(cmd));
        }
    }
}
