//! `bosun digest` logic (spec §14, §18): the daily owner digest as one D-012 chat form, and the
//! parser of the owner's reply ("approve 12 15", "raise 7 +20"). No I/O.

use crate::core::approve::message_id;
use crate::core::text::inert;

/// Longest title in the digest, in characters.
const TITLE: usize = 50;

/// One owner-waiting PR at the head the digest shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub pr: u64,
    pub head: String,
    pub rules: Vec<String>,
    pub title: String,
}

/// The digest: its id, the owner's text and the header line the reply must carry.
pub struct Digest {
    pub id: String,
    pub text: String,
    pub header: String,
}

/// A rule id as risk-check names it (`R-gov`); anything else never reaches the owner's form.
fn rule_ok(r: &str) -> bool {
    r.len() > 2
        && r.starts_with("R-")
        && r[2..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The digest for `repo` (`owner/name`) of `items` (owner-waiting PRs) and `asks` (`(issue,
/// title)` of open asks).
pub fn digest(repo: &str, items: &[Item], asks: &[(u64, String)]) -> Digest {
    let pairs: Vec<String> = items
        .iter()
        .map(|i| format!("{}@{}", i.pr, i.head))
        .collect();
    let id = message_id(&pairs);
    let mut text = format!("Daily digest {repo} {id}");
    text.push_str(&format!("\nApprovals ({}):", items.len()));
    for i in items {
        let short: String = i.head.chars().take(7).collect();
        let rules: Vec<&str> = i
            .rules
            .iter()
            .map(String::as_str)
            .filter(|r| rule_ok(r))
            .collect();
        let rules = if rules.is_empty() {
            "owner".to_string()
        } else {
            rules.join(", ")
        };
        let title = inert(&i.title, TITLE);
        text.push_str(&format!("\n- #{}@{short} {rules}: {title}", i.pr));
    }
    text.push_str(&format!("\nOpen asks ({}):", asks.len()));
    for (n, title) in asks {
        text.push_str(&format!("\n- #{n} {}", inert(title, TITLE)));
    }
    text.push_str("\nBudget asks (0):");
    let nums: Vec<String> = items.iter().map(|i| i.pr.to_string()).collect();
    let reply = if nums.is_empty() {
        "raise <epic> +<usd>".to_string()
    } else {
        format!("approve {} / raise <epic> +<usd>", nums.join(" "))
    };
    text.push_str(&format!("\nReply: {id} {reply}"));
    let header = format!("/bosun digest {repo} {id} {}", pairs.join(" "))
        .trim_end()
        .to_string();
    Digest { id, text, header }
}

/// A plain decimal number: no sign, no leading zero.
fn num(s: &str) -> Option<u64> {
    let n: u64 = s.parse().ok()?;
    (n.to_string() == s).then_some(n)
}

/// A parsed digest header: repo, id and the shown `(pr, head)` pairs.
pub type Header = (String, String, Vec<(u64, String)>);

/// The header line `/bosun digest <owner/repo> <id> <pr>@<sha>...`, when the id is the pairs'
/// message id. The id is an unkeyed checksum: it catches copy errors, not forgery.
pub fn parse_header(line: &str) -> Option<Header> {
    let mut w = line.split_whitespace();
    if (w.next(), w.next()) != (Some("/bosun"), Some("digest")) {
        return None;
    }
    let repo = w.next()?.to_string();
    let id = w.next()?.to_string();
    let (mut pairs, mut raw) = (Vec::new(), Vec::new());
    for p in w {
        let (pr, sha) = p.split_once('@')?;
        let hex = |b: u8| matches!(b, b'0'..=b'9' | b'a'..=b'f');
        (sha.len() == 40 && sha.bytes().all(hex)).then_some(())?;
        pairs.push((num(pr)?, sha.to_string()));
        raw.push(p.to_string());
    }
    (message_id(&raw) == id).then_some((repo, id, pairs))
}

/// The owner's reply: PRs to approve, `(epic, usd)` raises, and the lines that are neither.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// The digest id the owner quoted, if any.
    pub id: Option<String>,
    pub approve: Vec<u64>,
    pub raise: Vec<(u64, u64)>,
    pub unknown: Vec<String>,
}

/// Parse the reply: lines (or `;`/`/` parts) `approve 12 15` (`#12` and commas allowed) and
/// `raise <epic> +<usd>`. Anything else is `unknown`, never guessed.
pub fn parse_reply(text: &str) -> Reply {
    let mut r = Reply::default();
    for part in text.split(['\n', ';', '/']) {
        let mut words: Vec<String> = part
            .replace(',', " ")
            .split_whitespace()
            .map(|w| w.trim_start_matches('#').to_ascii_lowercase())
            .collect();
        // A leading digest id (8 hex) is the owner quoting the message they answer.
        if words
            .first()
            .is_some_and(|w| w.len() == 8 && w.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            r.id = Some(words.remove(0));
        }
        let ok = match words
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            [] => true,
            ["approve", prs @ ..] if !prs.is_empty() => {
                let n: Option<Vec<u64>> = prs.iter().map(|p| num(p)).collect();
                n.map(|n| r.approve.extend(n)).is_some()
            }
            ["raise", epic, usd] => match (num(epic), usd.strip_prefix('+').and_then(num)) {
                (Some(e), Some(u)) if u > 0 => {
                    r.raise.push((e, u));
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if !ok {
            r.unknown.push(part.trim().to_string());
        }
    }
    r.approve.sort_unstable();
    r.approve.dedup();
    r
}

/// The digest's head of a PR the owner named, or why it is not approved.
pub fn pick(
    shown: &[(u64, String)],
    now: &[(u64, String)],
    pr: u64,
) -> Result<String, &'static str> {
    let Some((_, head)) = shown.iter().find(|(n, _)| *n == pr) else {
        return Err("not in this digest");
    };
    match now.iter().find(|(n, _)| *n == pr) {
        Some((_, h)) if h == head => Ok(head.clone()),
        Some(_) => Err("head moved since the digest; listed again in the next digest"),
        None => Err("no longer waits for an owner at the digest's head"),
    }
}

/// The exact bytes an owner key signs for a raise: repo lowercased, `(digest id, day, SHA-256
/// hex of the normalized reply)` so one reply is never replayed as a second raise, epic, amount.
pub fn raise_message(
    owner: &str,
    repo: &str,
    at: (&str, &str, &str),
    epic: u64,
    usd: u64,
) -> String {
    let (o, r) = (owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
    let (id, day, sha) = at;
    format!("bosun-raise v1 {o}/{r} digest {id} day {day} reply {sha} epic {epic} usd +{usd}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> String {
        c.to_string().repeat(40)
    }

    fn item(pr: u64, c: char) -> Item {
        let (rules, title) = (vec!["R-gate".into()], format!("change {pr}"));
        Item {
            pr,
            head: sha(c),
            rules,
            title,
        }
    }

    #[test]
    fn the_digest_lists_prs_asks_and_the_reply() {
        let mut odd = item(15, 'b');
        odd.rules = vec!["R-gov".into(), "R-ci: `docs only`".into()];
        let d = digest(
            "acme/widget",
            &[item(12, 'a'), odd],
            &[(30, "which db?".into())],
        );
        let id = &d.id;
        let want = format!(
            "Daily digest acme/widget {id}\nApprovals (2):\n- #12@aaaaaaa R-gate: `change 12`\n- #15@bbbbbbb R-gov: `change 15`\nOpen asks (1):\n- #30 `which db?`\nBudget asks (0):\nReply: {id} approve 12 15 / raise <epic> +<usd>"
        );
        assert_eq!(d.text, want);
        let pairs = vec![(12, sha('a')), (15, sha('b'))];
        assert_eq!(
            parse_header(&d.header),
            Some(("acme/widget".into(), id.clone(), pairs))
        );
        let empty = digest("acme/widget", &[], &[]);
        assert_eq!(parse_header(&empty.header).unwrap().2, vec![]);
        assert!(
            empty
                .text
                .ends_with(&format!("Reply: {} raise <epic> +<usd>", empty.id))
        );
    }

    #[test]
    fn an_edited_header_does_not_parse() {
        let d = digest("acme/widget", &[item(12, 'a'), item(15, 'b')], &[]);
        let edited = d.header.replace(&sha('b'), &sha('c'));
        assert_eq!(parse_header(&edited), None);
        assert_eq!(parse_header(&d.header.replace("15@", "015@")), None);
        assert_eq!(parse_header("/bosun answer 1 ask 2"), None);
    }

    #[test]
    fn the_reply_parses_approvals_and_raises_and_keeps_the_rest() {
        let r = parse_reply("Approve #15, 12 12\nraise 7 +20; ok thanks\nraise 7 20\nraise 7 +0");
        assert_eq!(r.approve, vec![12, 15]);
        assert_eq!(r.raise, vec![(7, 20)]);
        assert_eq!(r.unknown, vec!["ok thanks", "raise 7 20", "raise 7 +0"]);
        assert_eq!(r.id, None);
        assert_eq!(parse_reply("approve 12 x").approve, Vec::<u64>::new());
        assert_eq!(parse_reply("approve").unknown, vec!["approve"]);
        let q = parse_reply("1A2B3C4D approve 12");
        assert_eq!((q.id.as_deref(), q.approve), (Some("1a2b3c4d"), vec![12]));
    }

    #[test]
    fn a_moved_head_or_an_unlisted_pr_drops_out() {
        let shown = vec![(12, sha('a')), (15, sha('b'))];
        let now = vec![(12, sha('a')), (15, sha('c'))];
        assert_eq!(pick(&shown, &now, 12), Ok(sha('a')));
        assert!(
            pick(&shown, &now, 15)
                .unwrap_err()
                .starts_with("head moved")
        );
        assert_eq!(pick(&shown, &now, 16), Err("not in this digest"));
        assert!(pick(&shown, &[], 12).unwrap_err().starts_with("no longer"));
    }

    #[test]
    fn the_raise_message_binds_repo_digest_day_reply_epic_and_amount() {
        let m = raise_message("Acme", "Widget", ("1a2b3c4d", "2026-11-12", "ab"), 7, 20);
        assert_eq!(
            m,
            "bosun-raise v1 acme/widget digest 1a2b3c4d day 2026-11-12 reply ab epic 7 usd +20"
        );
    }
}
