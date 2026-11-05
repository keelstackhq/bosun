//! The record of the last passing local `bosun gate` run, and the stale-gate test the Stop hook
//! uses (spec §7). Pure: the caller runs git and reads the file.

use serde_json::{Value, json};

/// Where the work tree stood when the gates last passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRecord {
    pub head: String,
    pub fingerprint: String,
}

/// Lowercase hex SHA-256 of `git status --porcelain=v1` and `git diff HEAD` output.
pub fn fingerprint(status: &str, diff: &str) -> String {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    for part in [status, "\0", diff] {
        ctx.update(part.as_bytes());
    }
    ctx.finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl GateRecord {
    pub fn to_json(&self) -> Value {
        json!({"head": self.head, "fingerprint": self.fingerprint})
    }

    pub fn from_json(v: &Value) -> Option<GateRecord> {
        Some(GateRecord {
            head: v.get("head")?.as_str()?.into(),
            fingerprint: v.get("fingerprint")?.as_str()?.into(),
        })
    }
}

/// True when the tree is not what the last passing gate run saw. Without a record only a dirty
/// tree counts; with one, any other head or fingerprint does.
pub fn is_stale(record: Option<&GateRecord>, now: &GateRecord, dirty: bool) -> bool {
    match record {
        None => dirty,
        Some(r) => r != now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(head: &str, fp: &str) -> GateRecord {
        GateRecord {
            head: head.into(),
            fingerprint: fp.into(),
        }
    }

    #[test]
    fn fingerprint_depends_on_both_parts() {
        let a = fingerprint("M a", "d1");
        assert_eq!(a.len(), 64);
        assert_eq!(a, fingerprint("M a", "d1"));
        assert_ne!(a, fingerprint("M a", "d2"));
        assert_ne!(a, fingerprint("M b", "d1"));
        assert_ne!(fingerprint("ab", "c"), fingerprint("a", "bc"));
    }

    #[test]
    fn stale_rules() {
        let r = rec("h1", "f1");
        assert!(!is_stale(Some(&r), &rec("h1", "f1"), false));
        assert!(!is_stale(Some(&r), &rec("h1", "f1"), true));
        assert!(is_stale(Some(&r), &rec("h1", "f2"), true));
        assert!(is_stale(Some(&r), &rec("h2", "f1"), false));
        assert!(!is_stale(None, &rec("h1", "f1"), false));
        assert!(is_stale(None, &rec("h1", "f1"), true));
    }

    #[test]
    fn json_round_trip() {
        let r = rec("h", "f");
        assert_eq!(GateRecord::from_json(&r.to_json()), Some(r));
        assert_eq!(GateRecord::from_json(&json!({"head": "h"})), None);
    }
}
