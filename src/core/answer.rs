//! The chat form of an owner's answer (spec section 10): the orchestrator relays the owner's
//! words with a signature of a key enrolled in `.bosun/approvers.toml`, bound to the repo, the
//! issue, the open ask and the exact text.

/// A parsed chat form. `sig` is the `(key id, base64 signature)` of its signature line.
#[derive(Debug, PartialEq, Eq)]
pub struct Chat<'a> {
    pub issue: u64,
    pub ask: u64,
    pub text: String,
    pub sig: Option<(&'a str, &'a str)>,
}

/// A plain decimal id: no sign, no leading zero.
fn id(s: &str) -> Option<u64> {
    let n: u64 = s.parse().ok()?;
    (n.to_string() == s).then_some(n)
}

/// The answer text as signed: trailing whitespace cut from each line, blank lines cut from both
/// ends, lines joined with `\n`. Pasting through a chat keeps it.
pub fn normalize(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let start = lines.iter().position(|l| !l.is_empty());
    let end = lines.iter().rposition(|l| !l.is_empty());
    match (start, end) {
        (Some(s), Some(e)) => lines[s..=e].join("\n"),
        _ => String::new(),
    }
}

/// Parse `/bosun answer <issue> ask <ask id>`, the answer lines, and the first
/// `bosun-sig ed25519 <key id> <sig>` line; text after that line is ignored. `None` when the
/// first non-blank line is not the header or the answer is empty.
pub fn parse_chat(input: &str) -> Option<Chat<'_>> {
    let mut lines = input.lines().skip_while(|l| l.trim().is_empty());
    let head: Vec<&str> = lines.next()?.trim().split(' ').collect();
    let ["/bosun", "answer", issue, "ask", ask] = head.as_slice() else {
        return None;
    };
    let (issue, ask) = (id(issue)?, id(ask)?);
    let mut body = Vec::new();
    let mut sig = None;
    for l in lines {
        let parts: Vec<&str> = l.trim().split(' ').collect();
        if let ["bosun-sig", "ed25519", k, s] = parts.as_slice() {
            sig = Some((*k, *s));
            break;
        }
        body.push(l);
    }
    let text = normalize(&body.join("\n"));
    (!text.is_empty()).then_some(Chat {
        issue,
        ask,
        text,
        sig,
    })
}

/// The exact bytes an owner key signs for an answer: owner and repo lowercased, and the
/// SHA-256 (lowercase hex) of the normalized text.
pub fn answer_message(owner: &str, repo: &str, issue: u64, ask: u64, text: &str) -> String {
    let hex = text_sha256(text);
    let (o, r) = (owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
    format!("bosun-answer v1 {o}/{r}#{issue} ask {ask} sha256 {hex}")
}

/// The lowercase hex SHA-256 of the normalized text, as the signed message and the record carry it.
pub fn text_sha256(text: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, normalize(text).as_bytes());
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The chat form the signer relays.
pub fn chat_form(issue: u64, ask: u64, text: &str, key_id: &str, sig_b64: &str) -> String {
    let text = normalize(text);
    format!("/bosun answer {issue} ask {ask}\n{text}\nbosun-sig ed25519 {key_id} {sig_b64}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_form_round_trips_and_ignores_a_footer() {
        let form = chat_form(4, 101, "  \nYes.  \n\nNo.\n", "k1", "c2ln");
        assert_eq!(
            form,
            "/bosun answer 4 ask 101\nYes.\n\nNo.\nbosun-sig ed25519 k1 c2ln"
        );
        let pasted = format!("\n{form}\n---\nfooter");
        let c = parse_chat(&pasted).unwrap();
        assert_eq!((c.issue, c.ask, c.text.as_str()), (4, 101, "Yes.\n\nNo."));
        assert_eq!(c.sig, Some(("k1", "c2ln")));
    }

    #[test]
    fn a_bad_header_or_empty_text_is_no_chat_form() {
        for bad in [
            "Yes.",
            "/bosun answer 4 ask 0101\nYes.",
            "/bosun answer +4 ask 1\nYes.",
            "/bosun answer 4\nYes.",
            "/bosun answer 4 ask 1\n\nbosun-sig ed25519 k1 s",
        ] {
            assert_eq!(parse_chat(bad), None, "{bad}");
        }
        assert_eq!(parse_chat("/bosun answer 4 ask 1\nYes.").unwrap().sig, None);
    }

    #[test]
    fn the_message_binds_repo_issue_ask_and_text() {
        let m = answer_message("Acme", "Widget", 4, 101, "Yes, drop it.\n");
        assert_eq!(
            m,
            "bosun-answer v1 acme/widget#4 ask 101 sha256 867f3ef4df6a78cc37edf9a83f7a38a55d0d5879f489417776fe7bad959b297e"
        );
        assert_ne!(m, answer_message("acme", "widget", 4, 102, "Yes, drop it."));
        assert_ne!(m, answer_message("acme", "widget", 5, 101, "Yes, drop it."));
        assert_ne!(m, answer_message("acme", "widget", 4, 101, "Yes, keep it."));
    }
}
