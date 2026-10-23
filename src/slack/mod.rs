//! The pure parts of the Slack stream (spec §10, plan M6 S1-S4, S8). bosun never calls Slack
//! and holds no Slack token: the caller supplies the channel facts and makes the posts.

use serde::Deserialize;

/// The notification kinds `bosun notify` knows (thread-result comes with M6/T4).
pub const KINDS: &[&str] = &["halt", "main-red", "budget-halt", "owner-question"];

/// The channel facts the caller read with its Slack tool (S2).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelInfo {
    pub id: String,
    pub is_ext_shared: bool,
    pub is_org_shared: bool,
    pub is_shared: bool,
    pub guests: u64,
}

impl ChannelInfo {
    pub fn parse(text: &str) -> Result<ChannelInfo, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// `Err(reason)` unless this is the configured channel, not shared, without guests.
    pub fn check(&self, configured: &str) -> Result<(), &'static str> {
        if self.id != configured {
            Err("wrong channel")
        } else if self.is_ext_shared || self.is_org_shared || self.is_shared {
            Err("channel shared")
        } else if self.guests > 0 {
            Err("channel has guests")
        } else {
            Ok(())
        }
    }
}

/// One post waiting for the caller: a stable key, its kind and the text.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub key: String,
    pub kind: &'static str,
    pub text: String,
}

/// A notification key: `^[a-z-]+:[0-9a-z:-]+$`.
pub fn valid_key(key: &str) -> bool {
    let Some((head, tail)) = key.split_once(':') else {
        return false;
    };
    !head.is_empty()
        && head.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        && !tail.is_empty()
        && tail
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b':' || b == b'-')
}

/// Make GitHub-sourced text safe to post (S8): Slack's own escapes for `&`, `<` and `>` (so no
/// `<!channel>` or link syntax survives) and no `@` (so no `@here` or user mention). One line,
/// at most 300 characters.
pub fn neutralise(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for c in flat.chars().take(300) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '@' => out.push_str("(at)"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        for ok in ["halt:7", "ask:12:345", "budget-halt:2026-10-22"] {
            assert!(valid_key(ok), "{ok}");
        }
        for bad in [
            "", "halt", "halt:", ":7", "Halt:7", "halt:7 x", "a_b:1", "a:B",
        ] {
            assert!(!valid_key(bad), "{bad}");
        }
    }

    #[test]
    fn neutralised_text_has_no_mentions() {
        let t = neutralise("hi <!channel> @here @everyone <@U1> a&b\nnext");
        for bad in ['<', '>', '@'] {
            assert!(!t.contains(bad), "{t}");
        }
        assert!(t.contains("a&amp;b next"));
    }
}
