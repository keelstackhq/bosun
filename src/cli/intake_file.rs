//! `bosun intake check|sign|verify`: the intake file `.bosun/intake.toml` (spec section 3) and
//! its detached signature `.bosun/intake.sig`. No GitHub access: everything is read from local
//! files, so `verify` can run in provision.yml against its own checked-out main.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{IntakeCheckArgs, IntakeSignArgs, IntakeVerifyArgs};
use crate::cli::approve::read_key;
use crate::core::approve::{APPROVERS_PATH, ApproverKey, Role, parse_approvers, verify_ed25519};
use crate::core::intake::{INTAKE_PATH, Intake, SigFile, intake_sha256, parse, signed_message};
use crate::github::b64;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::{Timestamp, parse_rfc3339};

/// The longest a signature may live.
const MAX_EXPIRES_SECS: u64 = 3600;

fn at(dir: Option<&Path>, p: &Path) -> PathBuf {
    dir.map_or_else(|| p.to_path_buf(), |d| d.join(p))
}

fn read(dir: Option<&Path>, p: &Path, what: &str) -> Result<String, BosunError> {
    std::fs::read_to_string(at(dir, p)).map_err(|e| {
        BosunError::usage(
            format!("cannot read the {what} {}: {}", p.display(), e.kind()),
            format!("pass the path of the {what}"),
        )
    })
}

fn schema(text: &str) -> Result<Intake, BosunError> {
    parse(text).map_err(|e| {
        let data = json!({"field": e.field});
        let err = if e.missing {
            BosunError::needs_human(e.message, e.fix)
        } else {
            BosunError::usage(e.message, e.fix)
        };
        err.with_data(data)
    })
}

fn view(i: &Intake) -> Value {
    serde_json::to_value(i).expect("intake serializes")
}

fn repo_of(i: &Intake) -> String {
    format!("{}/{}", i.repo.owner, i.repo.name).to_ascii_lowercase()
}

pub fn check(dir: Option<&Path>, a: &IntakeCheckArgs) -> CmdResult {
    let text = if a.from.is_some() {
        let mut t = String::new();
        let mut stdin = std::io::stdin();
        stdin.read_to_string(&mut t).map_err(|e| {
            BosunError::usage(
                format!("cannot read stdin: {}", e.kind()),
                "pipe the file in",
            )
        })?;
        t
    } else if let Some(f) = &a.file {
        read(dir, f, "intake file")?
    } else if at(dir, Path::new(INTAKE_PATH)).exists() {
        read(dir, Path::new(INTAKE_PATH), "intake file")?
    } else {
        return Err(BosunError::needs_human(
            format!("{INTAKE_PATH} does not exist"),
            format!("write {INTAKE_PATH} (`bosun init --intake`)"),
        ));
    };
    Ok(Outcome::data(view(&schema(&text)?)))
}

/// The enrolled keys of an approvers file.
fn keys(dir: Option<&Path>, p: &Path) -> Result<Vec<ApproverKey>, BosunError> {
    parse_approvers(&read(dir, p, "approvers file")?).map_err(|e| {
        BosunError::error(
            format!("{} is malformed: {e}", p.display()),
            "fix the approvers file on the default branch with a PR the owner approves",
        )
    })
}

pub fn sign(dir: Option<&Path>, a: &IntakeSignArgs) -> CmdResult {
    let text = read(dir, &a.file, "intake file")?;
    let intake = schema(&text)?;
    if a.repo.to_ascii_lowercase() != repo_of(&intake) {
        return Err(BosunError::usage(
            format!(
                "--repo {} is not the repo of the intake ({})",
                a.repo,
                repo_of(&intake)
            ),
            "pass --repo owner/name as repo.owner and repo.name of the file",
        ));
    }
    if a.expires_in == 0 || a.expires_in > MAX_EXPIRES_SECS {
        return Err(BosunError::usage(
            format!(
                "--expires-in {} is not between 1 and {MAX_EXPIRES_SECS}",
                a.expires_in
            ),
            "pass at most 3600 seconds; sign again when it runs out",
        ));
    }
    let pair = read_key(&a.sign_key)?;
    let public = b64::encode(ring::signature::KeyPair::public_key(&pair).as_ref());
    let key = match &a.key_id {
        Some(id) => id.clone(),
        None => {
            let path = a
                .approvers
                .clone()
                .unwrap_or_else(|| PathBuf::from(APPROVERS_PATH));
            let found = keys(dir, &path).ok().and_then(|ks| {
                ks.into_iter().find(|k| {
                    b64::decode(&k.ed25519).map(|b| b64::encode(&b)).as_deref() == Some(&public)
                })
            });
            found.map(|k| k.id).ok_or_else(|| {
                BosunError::error(
                    format!(
                        "the key is not enrolled in {}; nothing was signed",
                        path.display()
                    ),
                    format!("enroll the public key {public} in {APPROVERS_PATH}, or pass --key-id"),
                )
            })?
        }
    };
    let mut raw = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw)
        .map_err(|_| BosunError::error("no random bytes for the nonce", "retry"))?;
    let expires = Timestamp::now().plus_secs(a.expires_in as i64).to_string();
    let mut f = SigFile {
        key,
        repo: repo_of(&intake),
        mode: a.mode.clone(),
        nonce: raw.iter().map(|b| format!("{b:02x}")).collect(),
        expires,
        intake_sha256: intake_sha256(text.as_bytes()),
        sig: String::new(),
    };
    f.sig = b64::encode(pair.sign(f.message().as_bytes()).as_ref());
    Ok(Outcome::data(
        json!({"sig": f.to_toml(), "key": f.key, "nonce": f.nonce, "expires": f.expires, "mode": f.mode}),
    ))
}

fn refuse(reason: &str, why: &str, fix: &str) -> BosunError {
    BosunError::needs_human(format!("the intake signature does not count: {why}"), fix)
        .with_data(json!({"reason": reason}))
}

/// Only an owner key signs an intake (the orchestrator's key is enrolled as an owner key, as
/// for `bosun answer --chat`); a delegate key never does.
fn may_sign(role: Role) -> bool {
    role == Role::Owner
}

pub fn verify(dir: Option<&Path>, a: &IntakeVerifyArgs) -> CmdResult {
    let text = read(dir, &a.file, "intake file")?;
    let intake = schema(&text)?;
    let fix = "ask the orchestrator to sign the intake again (`bosun intake sign`) and start the workflow with the new signature";
    let sf = std::fs::read_to_string(at(dir, &a.sig))
        .map_err(|e| e.to_string())
        .and_then(|t| SigFile::parse(&t))
        .map_err(|e| {
            refuse(
                "bad_sig_file",
                &format!("the signature file is unusable ({e})"),
                fix,
            )
        })?;
    let nonce_ok = sf.nonce.len() == 32 && sf.nonce.bytes().all(|b| b.is_ascii_hexdigit());
    let expires = parse_rfc3339(&sf.expires);
    let (true, Some(expires)) = (nonce_ok, expires) else {
        return Err(refuse(
            "bad_sig_file",
            "the nonce or the expiry is malformed",
            fix,
        ));
    };
    if sf.intake_sha256 != intake_sha256(text.as_bytes()) {
        return Err(refuse(
            "intake_changed",
            "the intake differs from the one that was signed",
            fix,
        ));
    }
    if sf.repo.to_ascii_lowercase() != repo_of(&intake) {
        return Err(refuse(
            "repo_mismatch",
            "it was signed for another repo",
            fix,
        ));
    }
    if Timestamp::now() >= expires {
        return Err(refuse("expired", "it has expired", fix));
    }
    let ks = keys(dir, &a.approvers)?;
    let key = ks.iter().find(|k| k.id == sf.key).ok_or_else(|| {
        refuse(
            "unknown_key",
            "its key is not enrolled",
            "enroll the key in the approvers file with a PR the owner approves",
        )
    })?;
    if !may_sign(key.role) {
        return Err(refuse(
            "role_not_allowed",
            "its key has a role that may not sign intakes",
            "sign with an owner key",
        ));
    }
    let msg = signed_message(
        &sf.repo,
        &sf.mode,
        &sf.nonce,
        &sf.expires,
        &sf.intake_sha256,
    );
    let ok = match (b64::decode(&key.ed25519), b64::decode(&sf.sig)) {
        (Some(public), Some(raw)) => verify_ed25519(&public, msg.as_bytes(), &raw),
        _ => false,
    };
    if !ok {
        return Err(refuse(
            "bad_signature",
            "the signature does not verify",
            fix,
        ));
    }
    Ok(Outcome::data(
        json!({"verified": true, "key": sf.key, "repo": sf.repo, "mode": sf.mode,
        "nonce": sf.nonce, "expires": sf.expires, "intake": view(&intake)}),
    ))
}
