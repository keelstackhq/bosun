//! The `bosun adopt` manifest (`.bosun/adopt.toml`): what adopt wrote, so `bosun uninstall` can
//! take exactly that back. No I/O.

use serde::Deserialize;

pub const MANIFEST_PATH: &str = ".bosun/adopt.toml";

/// FNV-1a 64 of `bytes`, as 16 lowercase hex digits. Detects edits; not a security hash.
pub fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub bosun_version: String,
    #[serde(default, rename = "file")]
    pub files: Vec<ManifestFile>,
    /// Labels adopt created (they did not exist before).
    #[serde(default)]
    pub labels: Vec<String>,
    /// The registry issue, only when adopt created it.
    #[serde(default)]
    pub registry_issue: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    pub hash: String,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        toml::from_str(text).map_err(|e| e.message().to_string())
    }

    /// Record a file; replaces an entry of the same path. Entries stay sorted by path.
    pub fn set_file(&mut self, path: &str, hash: String) {
        self.files.retain(|f| f.path != path);
        self.files.push(ManifestFile {
            path: path.to_string(),
            hash,
        });
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
    }

    pub fn add_label(&mut self, name: &str) {
        if !self.labels.iter().any(|l| l == name) {
            self.labels.push(name.to_string());
        }
    }

    /// Deterministic text: same manifest, same bytes.
    pub fn to_toml(&self) -> String {
        let q = |s: &str| serde_json::to_string(s).expect("string");
        let mut out = String::from(
            "# Written by `bosun adopt`; `bosun uninstall` reads it. Edit by hand only to drop a file from adopt's care.\n",
        );
        out.push_str(&format!("bosun_version = {}\n", q(&self.bosun_version)));
        if !self.labels.is_empty() {
            let l: Vec<String> = self.labels.iter().map(|s| q(s)).collect();
            out.push_str(&format!("labels = [{}]\n", l.join(", ")));
        }
        if let Some(n) = self.registry_issue {
            out.push_str(&format!("registry_issue = {n}\n"));
        }
        for f in &self.files {
            out.push_str(&format!(
                "\n[[file]]\npath = {}\nhash = {}\n",
                q(&f.path),
                q(&f.hash)
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_known_values() {
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"a"), "af63dc4c8601ec8c");
    }

    #[test]
    fn manifest_round_trips_and_sorts() {
        let mut m = Manifest {
            bosun_version: "0.7.0".into(),
            labels: vec!["bosun:halt".into()],
            registry_issue: Some(3),
            ..Manifest::default()
        };
        m.set_file("b", "2".into());
        m.set_file("a", "1".into());
        m.set_file("b", "3".into());
        m.add_label("bosun:halt");
        let text = m.to_toml();
        assert_eq!(Manifest::parse(&text).unwrap(), m);
        assert_eq!(m.files[0].path, "a");
        assert_eq!(m.files[1].hash, "3");
        assert_eq!(m.labels.len(), 1);
    }

    #[test]
    fn unknown_key_is_an_error() {
        assert!(Manifest::parse("nope = 1").is_err());
    }
}
