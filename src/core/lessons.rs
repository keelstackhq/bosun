//! The lessons store `docs/lessons.toml` (spec §6): entries, caps, ids, hits and scope match.

use serde::{Deserialize, Serialize};

use super::glob::Glob;

/// At most this many entries.
pub const MAX: usize = 40;
/// At most this many entries without a scope.
pub const MAX_GLOBAL: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub rule: String,
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Hand-written entries may omit it.
    #[serde(default)]
    pub added: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hit: Option<String>,
    #[serde(default)]
    pub hits: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub lesson: Vec<Lesson>,
}

/// Which cap stopped an add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapError {
    /// True when the global cap (8) stopped it, false for the total cap (40).
    pub global: bool,
}

impl Store {
    pub fn global_count(&self) -> usize {
        self.lesson.iter().filter(|l| l.scope.is_empty()).count()
    }
}

/// The store of a `lessons.toml` text; no text means no lessons.
pub fn parse(text: Option<&str>) -> Result<Store, String> {
    match text {
        None => Ok(Store::default()),
        Some(t) => toml::from_str(t).map_err(|e| format!("docs/lessons.toml is not valid: {e}")),
    }
}

/// The next free `L-NNN`: highest number plus one.
pub fn next_id(store: &Store) -> String {
    let max = store
        .lesson
        .iter()
        .filter_map(|l| l.id.strip_prefix("L-")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("L-{:03}", max + 1)
}

/// Add a lesson and return its id; refuse at a cap.
pub fn add(
    store: &mut Store,
    text: &str,
    scope: Vec<String>,
    why: &str,
    evidence: Vec<String>,
    today: &str,
) -> Result<String, CapError> {
    if store.lesson.len() >= MAX {
        return Err(CapError { global: false });
    }
    if scope.is_empty() && store.global_count() >= MAX_GLOBAL {
        return Err(CapError { global: true });
    }
    let id = next_id(store);
    store.lesson.push(Lesson {
        id: id.clone(),
        rule: text.to_string(),
        why: why.to_string(),
        scope,
        evidence,
        added: today.to_string(),
        last_hit: None,
        hits: 0,
    });
    Ok(id)
}

/// Record a hit; false when the id is unknown.
pub fn hit(store: &mut Store, id: &str, today: &str) -> bool {
    match store.lesson.iter_mut().find(|l| l.id == id) {
        Some(l) => {
            l.hits += 1;
            l.last_hit = Some(today.to_string());
            true
        }
        None => false,
    }
}

/// Lessons for a task that touches `paths`: global ones plus those whose scope matches any path.
pub fn matching<'a>(store: &'a Store, paths: &[String]) -> Vec<&'a Lesson> {
    store
        .lesson
        .iter()
        .filter(|l| {
            l.scope.is_empty()
                || l.scope
                    .iter()
                    .filter_map(|g| Glob::parse(g).ok())
                    .any(|g| paths.iter().any(|p| g.matches(p)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_n(s: &mut Store, scope: &[&str]) -> Result<String, CapError> {
        add(
            s,
            "rule",
            scope.iter().map(|x| x.to_string()).collect(),
            "why",
            vec![],
            "2026-10-22",
        )
    }

    #[test]
    fn hand_written_entries_without_added_load() {
        let s = parse(Some("[[lesson]]\nid = \"L-001\"\nrule = \"r\"\n")).unwrap();
        assert_eq!(s.lesson[0].added, "");
        assert!(parse(None).unwrap().lesson.is_empty());
        assert!(parse(Some("[[lesson]]\nid = 1")).is_err());
    }

    #[test]
    fn ids_count_up_from_the_highest() {
        let mut s = Store::default();
        assert_eq!(add_n(&mut s, &["a/**"]).unwrap(), "L-001");
        assert_eq!(add_n(&mut s, &["a/**"]).unwrap(), "L-002");
        s.lesson.remove(0);
        assert_eq!(next_id(&s), "L-003");
    }

    #[test]
    fn total_cap_is_40_and_global_cap_is_8() {
        let mut s = Store::default();
        for _ in 0..8 {
            add_n(&mut s, &[]).unwrap();
        }
        assert_eq!(add_n(&mut s, &[]), Err(CapError { global: true }));
        for _ in 0..32 {
            add_n(&mut s, &["x/**"]).unwrap();
        }
        assert_eq!(add_n(&mut s, &["x/**"]), Err(CapError { global: false }));
    }

    #[test]
    fn hit_counts_and_unknown_id_is_false() {
        let mut s = Store::default();
        add_n(&mut s, &[]).unwrap();
        assert!(hit(&mut s, "L-001", "2026-10-23"));
        assert_eq!(s.lesson[0].hits, 1);
        assert_eq!(s.lesson[0].last_hit.as_deref(), Some("2026-10-23"));
        assert!(!hit(&mut s, "L-009", "2026-10-23"));
    }

    #[test]
    fn matching_takes_global_and_scoped() {
        let mut s = Store::default();
        add_n(&mut s, &[]).unwrap();
        add_n(&mut s, &["src/gates/**"]).unwrap();
        add_n(&mut s, &["docs/**"]).unwrap();
        let ids: Vec<_> = matching(&s, &["src/gates/mod.rs".into()])
            .iter()
            .map(|l| l.id.clone())
            .collect();
        assert_eq!(ids, ["L-001", "L-002"]);
    }
}
