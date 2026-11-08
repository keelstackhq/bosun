//! `bosun keygen`: make an ed25519 approval key (PKCS#8 PEM, mode 0600, outside the repo) and
//! print, or with `--enroll` append, its `.bosun/approvers.toml` block (spec section 8).

use std::io::Write;
use std::path::{Path, PathBuf};

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;

use super::KeygenArgs;
use crate::github::b64;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::Timestamp;

const APPROVERS: &str = ".bosun/approvers.toml";
const GOOD_OUT: &str = "bosun keygen --out ~/.bosun/approver.pem";

fn io_err(what: &str, path: &Path, e: &std::io::Error) -> BosunError {
    BosunError::error(
        format!("cannot {what} {}: {}", path.display(), e.kind()),
        "check the path and its permissions, then rerun `bosun keygen`",
    )
}

/// The git worktree root that holds `base`, if any.
fn toplevel(base: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (out.status.success() && !s.is_empty()).then(|| PathBuf::from(s))
}

/// `path` with symlinks resolved as far as it exists (the rest is appended unchanged).
fn resolve(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = path;
    while let Some(parent) = cur.parent() {
        if let Ok(real) = std::fs::canonicalize(cur) {
            return rest.iter().rev().fold(real, |p, c| p.join(c));
        }
        rest.extend(cur.file_name());
        cur = parent;
    }
    path.to_path_buf()
}

fn write_new(path: &Path, pem: &str) -> Result<(), BosunError> {
    use std::fs::{DirBuilder, OpenOptions};
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut b = DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(parent).map_err(|e| io_err("create", parent, &e))?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    match opts.open(path) {
        Ok(mut f) => f
            .write_all(pem.as_bytes())
            .map_err(|e| io_err("write", path, &e)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(BosunError::conflict(
            format!(
                "{} already exists; bosun never overwrites a key",
                path.display()
            ),
            "pick another path with `bosun keygen --out <new.pem>`",
        )),
        Err(e) => Err(io_err("create", path, &e)),
    }
}

pub fn run(dir: Option<&Path>, a: &KeygenArgs) -> CmdResult {
    let base = dir.unwrap_or_else(|| Path::new("."));
    let role = a.role.as_deref().unwrap_or("owner");
    let id =
        a.id.clone()
            .unwrap_or_else(|| format!("{role}-{}", &Timestamp::now().to_string()[..7]));
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(BosunError::usage(
            format!("key id {id:?} must be 1-64 characters of A-Z a-z 0-9 . _ -"),
            "pass a plain name, e.g. `--id owner-2026-11`",
        ));
    }
    let out = match &a.out {
        Some(p) => base.join(p),
        None => match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            Some(h) => PathBuf::from(h).join(".bosun/approver.pem"),
            None => {
                return Err(BosunError::usage(
                    "HOME is not set",
                    format!("run `{GOOD_OUT}`"),
                ));
            }
        },
    };
    let root = toplevel(base).map(|r| resolve(&r));
    if root.as_ref().is_some_and(|r| resolve(&out).starts_with(r)) {
        return Err(BosunError::usage(
            format!(
                "{} is inside this git worktree; a private key must never land in the repo",
                out.display()
            ),
            GOOD_OUT,
        ));
    }
    let approvers = match (a.enroll, &root) {
        (true, Some(r)) => Some(r.join(APPROVERS)),
        (true, None) => {
            return Err(BosunError::usage(
                "--enroll needs a git worktree",
                "run `bosun keygen --enroll` from inside the repo, or leave --enroll out",
            ));
        }
        _ => None,
    };
    let existing = match &approvers {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(io_err("read", p, &e)),
        },
        None => String::new(),
    };
    let table: toml::Table = existing.parse().map_err(|_| {
        BosunError::error(
            format!("{APPROVERS} is not valid TOML"),
            format!("fix {APPROVERS}, then rerun"),
        )
    })?;
    let ids = table
        .get("key")
        .and_then(|k| k.as_array())
        .into_iter()
        .flatten();
    if ids.filter_map(|k| k.get("id")?.as_str()).any(|i| i == id) {
        return Err(BosunError::conflict(
            format!("key id {id} is already in {APPROVERS}"),
            "pick a new name with `bosun keygen --id <name> --enroll`",
        ));
    }

    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| {
        BosunError::error("the system random source failed", "rerun `bosun keygen`")
    })?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| BosunError::error("ring made an unreadable key", "rerun `bosun keygen`"))?;
    let public = b64::encode(pair.public_key().as_ref());
    let body = b64::encode(pkcs8.as_ref())
        .as_bytes()
        .chunks(64)
        .map(|c| String::from_utf8_lossy(c).into_owned() + "\n")
        .collect::<String>();
    write_new(
        &out,
        &format!("-----BEGIN PRIVATE KEY-----\n{body}-----END PRIVATE KEY-----\n"),
    )?;

    let block = format!("[[key]]\nid = \"{id}\"\ned25519 = \"{public}\"\nrole = \"{role}\"\n");
    let shown = out.display();
    let steps = if let Some(p) = &approvers {
        let sep = if existing.is_empty() {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create", parent, &e))?;
        }
        std::fs::write(p, format!("{existing}{sep}{block}")).map_err(|e| io_err("write", p, &e))?;
        vec![
            "Commit .bosun/approvers.toml and open a PR; approve it yourself on GitHub (a PR cannot enroll its own key: risk-check reads the file from the default branch).".to_string(),
            format!("Keep {shown} on the machine that signs approvals; sign with `bosun approve <pr>@<head> --sign-key {shown}`."),
        ]
    } else {
        vec![
            "Add the toml block to .bosun/approvers.toml (or rerun with --enroll), commit it and open a PR; approve it yourself on GitHub.".to_string(),
            format!(
                "Keep {shown} on the machine that signs approvals; sign with `bosun approve <pr>@<head> --sign-key {shown}`."
            ),
        ]
    };
    Ok(Outcome::data(json!({
        "path": shown.to_string(), "public_key": public, "id": id, "role": role,
        "toml": block, "enrolled": approvers.is_some(), "steps": steps,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pem_reads_back_through_approve_read_key() {
        let d = tempfile::tempdir().unwrap();
        let pem = d.path().join("k.pem");
        let args = KeygenArgs {
            out: Some(pem.clone()),
            id: None,
            role: None,
            enroll: false,
        };
        let out = run(Some(d.path()), &args).unwrap();
        let v = out.data;
        let pair = crate::cli::approve::read_key(&pem).unwrap();
        assert_eq!(b64::encode(pair.public_key().as_ref()), v["public_key"]);
    }
}
