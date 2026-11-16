//! Prose string literals in the quoted heredoc of an interpreter (spec §4, decrypt-lock):
//! `python3 - <<'EOF'` with `msg = 'agents never decrypt (age key)'` is data. A literal that
//! could be a command (shell syntax, a command or runner as its first word, a `sops`, `age`,
//! `rage` or `hoist secrets` call, a key path) stays code, so `os.system('sops -d f')` is
//! still judged. Shells (`bash <<'EOF'`) are not covered: their whole body is code.

use super::{INTERPS, RUNNERS, SHELLS};

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
    ".config",
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

/// Punctuation around a word: `(sops,` is `sops`.
const PUNCT: [char; 13] = [
    '(', ')', '[', ']', '{', '}', ',', ';', ':', '\'', '"', '`', '.',
];

fn base(w: &str) -> String {
    let w = w.trim_matches(PUNCT);
    let w = w.rsplit('/').next().unwrap_or(w);
    w.split('@').next().unwrap_or(w).to_lowercase()
}

/// Prose: words with spaces, no shell syntax or key marker, a plain first word that runs
/// nothing, no `sops|age|rage|hoist secrets` call in it, and, with those names (not calls)
/// replaced, nothing the decrypt-lock refuses as a command (`ps eww 1`, `export -p`).
fn is_prose(lit: &str, depth: usize) -> bool {
    // The words as the shell reads them: quotes join and vanish (`'so'ps` is `sops`).
    let words: Vec<String> = lit
        .split_whitespace()
        .map(|w| w.replace(['\'', '"'], ""))
        .collect();
    is_plain(lit, &words) && {
        // Only a bare name (`(age`, `sops.`), never a path: `~/.config/sops` stays judged.
        let named = |w: &str| {
            let bare = w.trim_matches(['(', ')', ',', '.', ':', '!', '?']);
            let name = matches!(
                bare.to_lowercase().as_str(),
                "sops" | "age" | "rage" | "hoist"
            );
            name && !w.contains(['/', '~', '@'])
        };
        let words: Vec<&str> = words
            .iter()
            .map(|w| if named(w) { "x" } else { w.as_str() })
            .collect();
        super::decrypt_rule(&words.join(" "), depth + 1).is_none()
    }
}

fn is_plain(lit: &str, words: &[String]) -> bool {
    let Some(first) = words.first() else {
        return false;
    };
    let syntax = lit.contains([';', '&', '|', '$', '`', '<', '>', '\\', '{', '}', '=']);
    // Code, not prose: any quote, bracket or paren inside the literal, or a dunder.
    let code = lit.contains(['\'', '"', '(', ')', '[', ']', '{', '}']) || lit.contains("__");
    // Paths and globs are not prose: `/`, `~`, `*`, or a `?` inside a word.
    let path = lit.contains(['/', '~', '*'])
        || words.iter().any(|w| w.trim_end_matches('?').contains('?'));
    let code = code || path;
    if words.len() < 2 || syntax || code || MARKERS.iter().any(|m| lit.contains(m)) {
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
        let next = words.get(i + 1);
        match base(w).trim_end_matches(['!', '?']) {
            // `sops FILE` is the decrypting editor: any word after `sops` is a call, unless
            // the sentence ends there (`never sops.`).
            "sops" => !w.ends_with(['.', '!', '?']) && next.is_some(),
            "age" | "rage" => next.is_some_and(|n| n.starts_with('-')),
            // Any `hoist secrets` with a subcommand: `exec -- CMD` runs CMD with the secrets.
            "hoist" => words[i + 1..]
                .iter()
                .position(|a| a == "secrets")
                .is_some_and(|p| words.len() > i + 2 + p),
            _ => false,
        }
    })
}
