//! Commit trailers for traceability (spec §4): `Bosun-Session`, `Bosun-Task`, `Bosun-Model`.
//! Pure text logic; `bosun trailers stamp` writes them, `bosun pr` checks them.

pub const SESSION: &str = "Bosun-Session";
pub const TASK: &str = "Bosun-Task";
pub const MODEL: &str = "Bosun-Model";
/// A commit must carry these; the model is added only when known.
pub const REQUIRED: [&str; 2] = [SESSION, TASK];

/// `line` is the trailer `key: value`, ignoring the key's case (git matches that way).
fn has_key(line: &str, key: &str) -> bool {
    let n = key.len() + 2;
    line.get(..n)
        .is_some_and(|h| h[..key.len()].eq_ignore_ascii_case(key) && h.ends_with(": "))
        && is_trailer(line)
}

/// An editor comment: `# text` or a lone `#`. `#123` is body text.
fn is_comment(line: &str) -> bool {
    line == "#" || line.starts_with("# ")
}

fn is_trailer(line: &str) -> bool {
    line.split_once(": ").is_some_and(|(k, v)| {
        !v.trim().is_empty()
            && !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Lines before the trailing comment/blank lines git leaves in an editor message.
fn body_end(lines: &[&str]) -> usize {
    let mut end = lines
        .iter()
        .position(|l| l.starts_with("# ------------------------ >8"))
        .unwrap_or(lines.len());
    while end > 0 && (is_comment(lines[end - 1]) || lines[end - 1].trim().is_empty()) {
        end -= 1;
    }
    end
}

/// The required trailers `msg` lacks (checked in its last paragraph).
pub fn missing(msg: &str) -> Vec<&'static str> {
    let lines: Vec<&str> = msg.lines().collect();
    let end = body_end(&lines);
    let start = lines[..end]
        .iter()
        .rposition(|l| l.trim().is_empty())
        .map_or(0, |i| i + 1);
    let block = &lines[start..end];
    REQUIRED
        .into_iter()
        .filter(|k| !block.iter().any(|l| has_key(l, k)))
        .collect()
}

/// `msg` with each `want` trailer that is missing and has a value appended to its trailer block.
pub fn stamp(msg: &str, want: &[(&str, String)]) -> String {
    let lines: Vec<&str> = msg.lines().collect();
    let end = body_end(&lines);
    let start = lines[..end]
        .iter()
        .rposition(|l| l.trim().is_empty())
        .map_or(0, |i| i + 1);
    let block = &lines[start..end];
    let add: Vec<String> = want
        .iter()
        .filter(|(k, v)| !v.trim().is_empty() && !block.iter().any(|l| has_key(l, k)))
        .map(|(k, v)| format!("{k}: {}", v.trim()))
        .collect();
    if add.is_empty() || end == 0 {
        return msg.to_string();
    }
    let mut out: Vec<String> = lines[..end].iter().map(|l| l.to_string()).collect();
    if !(start < end && block.iter().all(|l| is_trailer(l)) && start > 0) {
        out.push(String::new());
    }
    out.extend(add);
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want() -> Vec<(&'static str, String)> {
        vec![
            (SESSION, "s1".into()),
            (TASK, "7".into()),
            (MODEL, String::new()),
        ]
    }

    #[test]
    fn missing_lists_absent_required() {
        assert_eq!(missing("feat: x\n"), [SESSION, TASK]);
        assert_eq!(
            missing("feat: x\n\nBosun-Session: s\nBosun-Task: 3\n"),
            Vec::<&str>::new()
        );
        assert_eq!(missing("feat: x\n\nBosun-Task: 3\n"), [SESSION]);
        // only the last paragraph counts, and an empty value does not
        assert_eq!(
            missing("Bosun-Session: s\nBosun-Task: 3\n\nbody\n"),
            [SESSION, TASK]
        );
        assert_eq!(missing("x\n\nBosun-Session: \nBosun-Task: 3\n"), [SESSION]);
    }

    #[test]
    fn stamp_appends_in_a_new_paragraph_and_skips_empty_values() {
        assert_eq!(
            stamp("feat: x\n", &want()),
            "feat: x\n\nBosun-Session: s1\nBosun-Task: 7\n"
        );
    }

    #[test]
    fn stamp_joins_an_existing_trailer_block_and_is_idempotent() {
        let m = "feat: x\n\nbody\n\nCo-Authored-By: C <c@c>\n";
        let once = stamp(m, &want());
        assert_eq!(
            once,
            "feat: x\n\nbody\n\nCo-Authored-By: C <c@c>\nBosun-Session: s1\nBosun-Task: 7\n"
        );
        assert_eq!(stamp(&once, &want()), once);
        let part = "x\n\nBosun-Task: 9\n";
        assert_eq!(
            stamp(part, &want()),
            "x\n\nBosun-Task: 9\nBosun-Session: s1\n"
        );
    }

    #[test]
    fn stamp_goes_above_editor_comments() {
        let m = "feat: x\n\n# Please enter\n# more\n";
        assert_eq!(
            stamp(m, &want()),
            "feat: x\n\nBosun-Session: s1\nBosun-Task: 7\n\n# Please enter\n# more\n"
        );
    }

    #[test]
    fn scissors_tail_is_kept_below_and_empty_values_are_filled() {
        let m =
            "feat: x\n\n# ------------------------ >8 ------------------------\ndiff --git a b\n";
        assert_eq!(
            stamp(m, &want()),
            "feat: x\n\nBosun-Session: s1\nBosun-Task: 7\n\n# ------------------------ >8 ------------------------\ndiff --git a b\n"
        );
        let e = "x\n\nBosun-Session: \n";
        let got = stamp(e, &want());
        assert!(
            got.contains("Bosun-Session: s1\nBosun-Task: 7\n"),
            "{got:?}"
        );
    }

    #[test]
    fn issue_ref_line_is_body_and_keys_ignore_case() {
        let m = "feat: x\n\nRefs\n#123\n";
        assert_eq!(
            stamp(m, &want()),
            "feat: x\n\nRefs\n#123\n\nBosun-Session: s1\nBosun-Task: 7\n"
        );
        // editor comments (`# text`, a lone `#`) still sit below the trailers
        assert_eq!(
            stamp("x\n\n#\n# hi\n", &want()),
            "x\n\nBosun-Session: s1\nBosun-Task: 7\n\n#\n# hi\n"
        );
        let c = "x\n\nbosun-session: s\nBOSUN-TASK: 3\n";
        assert!(missing(c).is_empty());
        assert_eq!(stamp(c, &want()), c);
    }

    #[test]
    fn empty_message_is_left_alone() {
        assert_eq!(stamp("", &want()), "");
    }
}
