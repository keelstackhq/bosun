//! Prose string literals in the quoted heredoc of an interpreter (spec §4, decrypt-lock):
//! `python3 - <<'EOF'` with `msg = 'agents never decrypt (age key)'` is data. A literal that
//! could be a command (shell syntax, a command or runner as its first word, a `sops`, `age`,
//! `rage` or `hoist secrets` call, a key path) stays code, so `os.system('sops -d f')` is
//! still judged. Shells (`bash <<'EOF'`) are not covered: their whole body is code.

use super::{HOIST_SECRETS, INTERPS, RUNNERS, SHELLS};

/// Words after `sops` that read as prose (`sops or age`), not as its file or subcommand.
const PROSE_AFTER: &[&str] = &[
    "and", "are", "but", "itself", "never", "nor", "not", "or", "was", "were", "with",
];
/// Words that run what follows them, beyond RUNNERS, SHELLS and INTERPS.
const RUNS: &[&str] = &[
    "exec", "command", "builtin", "eval", "source", "npx", "bunx", "pnpx", "coproc", "go", "sudo",
    "doas", "env", "xargs", "watch", "flock", "nohup", "time", "parallel",
];
const MARKERS: &[&str] = &[
    "sops/age",
    "keys.txt",
    "SOPS_AGE_KEY",
    "environ",
    "/proc",
    "AGE-SECRET",
    "SECRET-KEY",
];

/// The body with every prose literal emptied (`'..'` -> `''`), else unchanged. A literal is
/// one line between `'` or `"` (a `\` escapes); triple quotes and unterminated quotes stay.
pub(super) fn blank_prose(body: &str, depth: usize) -> String {
    let (mut out, mut rest) = (String::new(), body);
    while let Some(at) = rest.find(['\'', '"']) {
        out.push_str(&rest[..at]);
        let q = rest[at..].chars().next().unwrap_or('"');
        let after = &rest[at + 1..];
        if after.starts_with(q) {
            // `''`, `""` or a triple quote: copy the quotes and go on, never blanking.
            let n = after.chars().take_while(|c| *c == q).count();
            out.push_str(&rest[at..at + 1 + n]);
            rest = &after[n..];
            continue;
        }
        let Some(end) = literal_end(after, q) else {
            out.push_str(&rest[at..]);
            return out;
        };
        let lit = &after[..end];
        out.push(q);
        if !is_prose(lit, depth) {
            out.push_str(lit);
        }
        out.push(q);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The index of the closing quote `q` on this line, skipping `\` escapes.
fn literal_end(s: &str, q: char) -> Option<usize> {
    let mut esc = false;
    for (i, c) in s.char_indices() {
        match c {
            '\n' => return None,
            _ if esc => esc = false,
            '\\' => esc = true,
            _ if c == q => return Some(i),
            _ => {}
        }
    }
    None
}

fn base(w: &str) -> String {
    let w = w.trim_start_matches(['(', '[', '{']);
    let w = w.rsplit('/').next().unwrap_or(w);
    w.split('@').next().unwrap_or(w).to_lowercase()
}

/// Prose: words with spaces, no shell syntax or key marker, a plain first word that runs
/// nothing, no `sops|age|rage|hoist secrets` call in it, and, with those names (not calls)
/// replaced, nothing the decrypt-lock refuses as a command (`ps eww 1`, `export -p`).
fn is_prose(lit: &str, depth: usize) -> bool {
    is_plain(lit) && {
        let named = |w: &&str| matches!(base(w).as_str(), "sops" | "age" | "rage" | "hoist");
        let words: Vec<&str> = lit
            .split_whitespace()
            .map(|w| if named(&w) { "x" } else { w })
            .collect();
        super::decrypt_rule(&words.join(" "), depth + 1).is_none()
    }
}

fn is_plain(lit: &str) -> bool {
    let words: Vec<&str> = lit.split_whitespace().collect();
    let Some(first) = words.first() else {
        return false;
    };
    let syntax = lit.contains([';', '&', '|', '$', '`', '<', '>', '\\', '{', '}', '=']);
    if words.len() < 2 || syntax || MARKERS.iter().any(|m| lit.contains(m)) {
        return false;
    }
    let f = first.trim_start_matches('(');
    let f = f.trim_end_matches([',', '.', ':', '!', '?']);
    let low = f.to_lowercase();
    let runs = matches!(low.as_str(), "sops" | "age" | "rage")
        || SHELLS.contains(&low.as_str())
        || INTERPS.contains(&low.as_str())
        || RUNS.contains(&low.as_str())
        || RUNNERS.iter().any(|r| r.0 == low)
        || super::interp::is_runner(&low);
    if f.is_empty() || !f.chars().all(|c| c.is_ascii_alphabetic()) || runs {
        return false;
    }
    !words.iter().enumerate().any(|(i, w)| {
        let next = words.get(i + 1).copied();
        match base(w).as_str() {
            "sops" => w.ends_with("sops") && next.is_some_and(|n| !PROSE_AFTER.contains(&n)),
            "age" | "rage" => next.is_some_and(|n| n.starts_with('-')),
            // The shell rule's: a write subcommand after `secrets`, before `--` or `exec`.
            "hoist" => words[i + 1..]
                .iter()
                .position(|a| *a == "secrets")
                .is_some_and(|p| {
                    words[i + 2 + p..]
                        .iter()
                        .take_while(|a| **a != "--" && **a != "exec")
                        .any(|a| HOIST_SECRETS.contains(a))
                }),
            _ => false,
        }
    })
}
