//! Lockfile merges for approval carry-over (spec §8, F2). After a merge of the default branch the
//! regenerated `Cargo.lock` or `pnpm-lock.yaml` equals neither side, but it is safe to carry an
//! approval when it holds nothing new: every whole entry of the merged file is, byte for byte, an
//! entry of the approved head's file or of the default branch's file. Entries are compared whole
//! (a Cargo `[[package]]` block with name, version, source, checksum and dependencies; a pnpm
//! package key with everything under it), so one package's name cannot be stitched to another's
//! checksum. Pure: texts in, a yes or no out.

use std::collections::BTreeSet;

/// The lockfile kind of a path by its file name, if any.
pub fn lockfile_kind(path: &str) -> Option<Lock> {
    match path.rsplit('/').next() {
        Some("Cargo.lock") => Some(Lock::Cargo),
        Some("pnpm-lock.yaml") => Some(Lock::Pnpm),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Cargo,
    Pnpm,
}

/// The units of a lockfile: whole entries, each with its section, trailing blanks dropped.
/// `None` when the text has the same unit twice (no tool writes that; fail closed).
fn units(kind: Lock, text: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut cur: Option<String> = None;
    let flush = |cur: &mut Option<String>, out: &mut Vec<String>| {
        if let Some(u) = cur.take() {
            out.push(u.trim_end().to_string());
        }
    };
    for line in text.lines() {
        let starts = match kind {
            // `[[package]]`, `[metadata]`, ...: a new block. Text before the first is one unit.
            Lock::Cargo => line.starts_with('['),
            // A top-level key, or an entry two spaces in under one.
            Lock::Pnpm => {
                let top = !line.is_empty() && !line.starts_with([' ', '#']);
                let entry = line.starts_with("  ") && !line[2..].starts_with([' ', '-']);
                if top {
                    section = line.to_string();
                }
                top || entry
            }
        };
        if starts {
            flush(&mut cur, &mut out);
            let head = match kind {
                Lock::Pnpm if line.starts_with(' ') => format!("{section}\n"),
                _ => String::new(),
            };
            cur = Some(head);
        }
        let u = cur.get_or_insert_with(String::new);
        u.push_str(line);
        u.push('\n');
    }
    flush(&mut cur, &mut out);
    let mut seen = BTreeSet::new();
    out.retain(|u| !u.trim().is_empty());
    out.iter().all(|u| seen.insert(u.clone())).then_some(out)
}

/// Whether `merged` holds nothing that neither `approved` nor `main` holds, entry by entry.
/// Entries may be dropped (a merge prunes them); none may be new or changed.
pub fn merged_lock_ok(kind: Lock, approved: &str, main: &str, merged: &str) -> bool {
    let (Some(a), Some(m), Some(x)) = (
        units(kind, approved),
        units(kind, main),
        units(kind, merged),
    ) else {
        return false;
    };
    let known: BTreeSet<&String> = a.iter().chain(&m).collect();
    x.iter().all(|u| known.contains(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, version: &str, sum: &str) -> String {
        format!(
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
             source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
             checksum = \"{sum}\"\n"
        )
    }

    fn member(name: &str, deps: &[&str]) -> String {
        let list: Vec<String> = deps.iter().map(|d| format!(" \"{d}\",\n")).collect();
        format!(
            "[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\ndependencies = [\n{}]\n",
            list.concat()
        )
    }

    fn lock(pkgs: &[String]) -> String {
        format!("# auto\nversion = 4\n\n{}", pkgs.join("\n"))
    }

    fn cargo(h: &str, m: &str, merged: &str) -> bool {
        merged_lock_ok(Lock::Cargo, h, m, merged)
    }

    #[test]
    fn a_union_of_both_sides_carries() {
        let serde = pkg("serde", "1.0.0", "aa");
        let anyhow = pkg("anyhow", "1.0.0", "bb");
        let tokio = pkg("tokio", "1.0.0", "cc");
        let h = lock(&[serde.clone(), anyhow.clone(), member("app", &["anyhow"])]);
        let m = lock(&[serde.clone(), tokio.clone(), member("srv", &["tokio"])]);
        let merged = lock(&[
            anyhow.clone(),
            member("app", &["anyhow"]),
            serde,
            member("srv", &["tokio"]),
            tokio,
        ]);
        assert!(cargo(&h, &m, &merged));
        // A dropped entry is fine: the merge pruned it.
        assert!(cargo(&h, &m, &lock(&[member("app", &["anyhow"]), anyhow])));
    }

    #[test]
    fn a_stitched_or_new_entry_is_refused() {
        let h = lock(&[pkg("anyhow", "1.0.0", "bb")]);
        let m = lock(&[pkg("tokio", "1.0.0", "cc")]);
        assert!(cargo(&h, &m, &lock(&[pkg("anyhow", "1.0.0", "bb")])));
        // anyhow's name and version with tokio's checksum: every line exists on one side.
        assert!(!cargo(&h, &m, &lock(&[pkg("anyhow", "1.0.0", "cc")])));
        let new = lock(&[pkg("anyhow", "1.0.0", "bb"), pkg("evil", "0.1.0", "dd")]);
        assert!(!cargo(&h, &m, &new));
        assert!(!cargo(&h, &m, &lock(&[pkg("anyhow", "1.0.1", "bb")])));
        // A member whose dependency list mixes both sides is new.
        let (hm, mm) = (
            lock(&[member("app", &["a"])]),
            lock(&[member("app", &["t"])]),
        );
        assert!(!cargo(&hm, &mm, &lock(&[member("app", &["a", "t"])])));
        // So is a new preamble, and a duplicate entry is refused.
        let v3 = lock(&[pkg("anyhow", "1.0.0", "bb")]).replace("version = 4", "version = 3");
        assert!(!cargo(&h, &m, &v3));
        let dup = lock(&[pkg("anyhow", "1.0.0", "bb"), pkg("anyhow", "1.0.0", "bb")]);
        assert!(!cargo(&h, &m, &dup));
    }

    const PNPM_H: &str = "lockfileVersion: '9.0'

importers:

  apps/a:
    dependencies:
      left-pad:
        specifier: ^1.3.0
        version: 1.3.0

packages:

  left-pad@1.3.0:
    resolution: {integrity: sha512-AAA}

snapshots:

  left-pad@1.3.0: {}
";

    const PNPM_M: &str = "lockfileVersion: '9.0'

importers:

  apps/b:
    dependencies:
      is-odd:
        specifier: ^3.0.1
        version: 3.0.1

packages:

  is-odd@3.0.1:
    resolution: {integrity: sha512-BBB}

snapshots:

  is-odd@3.0.1: {}
";

    const PNPM_MERGED: &str = "lockfileVersion: '9.0'

importers:

  apps/a:
    dependencies:
      left-pad:
        specifier: ^1.3.0
        version: 1.3.0

  apps/b:
    dependencies:
      is-odd:
        specifier: ^3.0.1
        version: 3.0.1

packages:

  is-odd@3.0.1:
    resolution: {integrity: sha512-BBB}

  left-pad@1.3.0:
    resolution: {integrity: sha512-AAA}

snapshots:

  is-odd@3.0.1: {}

  left-pad@1.3.0: {}
";

    fn pnpm(merged: &str) -> bool {
        merged_lock_ok(Lock::Pnpm, PNPM_H, PNPM_M, merged)
    }

    #[test]
    fn pnpm_entries_are_compared_whole() {
        assert!(pnpm(PNPM_MERGED));
        // left-pad with is-odd's integrity: each line exists on one side, the entry does not.
        let stitched = PNPM_MERGED.replace("sha512-AAA", "sha512-BBB");
        assert!(!pnpm(&stitched));
        // An entry moved to another section is new there.
        let moved = PNPM_MERGED
            .replace("snapshots:\n\n  is-odd@3.0.1: {}\n", "snapshots:\n")
            .replace("packages:\n", "packages:\n\n  is-odd@3.0.1: {}\n");
        assert!(!pnpm(&moved));
        let tarball = PNPM_MERGED.replace("AAA}", "AAA, tarball: https://evil.test/x.tgz}");
        assert!(!pnpm(&tarball));
        assert!(!pnpm(&PNPM_MERGED.replace("'9.0'", "'6.0'")));
    }

    #[test]
    fn lockfile_kinds_by_name() {
        assert_eq!(lockfile_kind("Cargo.lock"), Some(Lock::Cargo));
        assert_eq!(lockfile_kind("web/pnpm-lock.yaml"), Some(Lock::Pnpm));
        assert_eq!(lockfile_kind("Cargo.lock.bak"), None);
        assert_eq!(lockfile_kind("package-lock.json"), None);
    }
}
