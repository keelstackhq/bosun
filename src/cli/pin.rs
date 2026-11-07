//! `bosun pin --latest`: move the tool pins in this repo to the newest npm release of splice,
//! hoist, berth and bosun, after checking every tarball against npm's `dist.integrity` (spec D35).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::PinArgs;
use crate::core::pin::{Pins, apply, get_line, keys, parse_semver, sha512_of};
use crate::ctx::git_out;
use crate::github::b64;
use crate::output::{BosunError, CmdResult, Outcome};

const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";
const SCRIPT: &str = "docs/setup-script.sh";
const TEMPLATE: &str = "channel/pieces/core/files/docs/setup-script.sh.jinja";
const VERSION_FILE: &str = ".bosun/version";
const CHANNEL: &str = ".splice/channels/bosun.toml";

/// What npm says about one package version.
struct Meta {
    version: String,
    integrity: String,
    tarball: String,
}

struct Npm {
    base: String,
    agent: ureq::Agent,
}

impl Npm {
    fn new() -> Npm {
        let base = std::env::var("BOSUN_NPM_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.into());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .into();
        Npm {
            base: base.trim_end_matches('/').into(),
            agent,
        }
    }

    fn get(&self, url: &str, what: &str, limit: u64) -> Result<Vec<u8>, BosunError> {
        let fail = |d: String| {
            BosunError::error(
                format!("npm {what}: {d}"),
                "check the network and BOSUN_NPM_REGISTRY, then run `bosun pin --latest` again",
            )
        };
        let mut resp = self
            .agent
            .get(url)
            .call()
            .map_err(|e| fail(e.to_string()))?;
        if resp.status().as_u16() != 200 {
            return Err(fail(format!("HTTP {}", resp.status().as_u16())));
        }
        let body = resp.body_mut().with_config().limit(limit).read_to_vec();
        body.map_err(|e| fail(e.to_string()))
    }

    /// Metadata of `pkg` at `version` (`latest` or X.Y.Z).
    fn meta(&self, pkg: &str, version: &str) -> Result<Meta, BosunError> {
        let what = format!("@keelstackhq/{pkg}@{version}");
        let url = format!("{}/@keelstackhq/{pkg}/{version}", self.base);
        let v: Value = serde_json::from_slice(&self.get(&url, &what, 1 << 22)?)
            .map_err(|e| BosunError::error(format!("npm {what}: bad JSON: {e}"), "retry later"))?;
        let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(String::from);
        match (s("/version"), s("/dist/integrity"), s("/dist/tarball")) {
            (Some(version), Some(integrity), Some(tarball)) => Ok(Meta {
                version,
                integrity,
                tarball,
            }),
            _ => Err(BosunError::error(
                format!("npm {what}: no version, dist.integrity or dist.tarball"),
                "run `bosun pin --latest` again; if it persists, pin by hand",
            )),
        }
    }

    /// The sha512 (base64) of `m`'s tarball, after downloading and checking it against npm's integrity.
    fn verified(&self, pkg: &str, m: &Meta) -> Result<String, BosunError> {
        let name = format!("@keelstackhq/{pkg}@{}", m.version);
        let bad = |msg: String| {
            BosunError::error(
                msg,
                "do not pin this release; tell the owner, then rerun later",
            )
        };
        if !m.tarball.starts_with(&format!("{}/", self.base)) {
            return Err(bad(format!(
                "{name}: tarball URL {} is outside the registry",
                m.tarball
            )));
        }
        let want = sha512_of(&m.integrity).ok_or_else(|| {
            bad(format!(
                "{name}: integrity {:?} is not sha512-",
                m.integrity
            ))
        })?;
        let bytes = self.get(&m.tarball, &name, 1 << 29)?;
        let got = b64::encode(ring::digest::digest(&ring::digest::SHA512, &bytes).as_ref());
        if got != want {
            return Err(bad(format!(
                "{name}: tarball sha512 does not match dist.integrity"
            )));
        }
        Ok(got)
    }
}

fn io_err(what: &str, p: &Path, e: std::io::Error) -> BosunError {
    BosunError::error(
        format!("cannot {what} {}: {e}", p.display()),
        "make the path accessible, then rerun",
    )
}

pub fn run(dir: Option<&Path>, a: &PinArgs) -> CmdResult {
    let start = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir().map_err(|e| io_err("read", Path::new("."), e))?,
    };
    let root: PathBuf =
        git_out(&start, &["rev-parse", "--show-toplevel"]).map_or(start, PathBuf::from);
    if root.join(CHANNEL).exists() {
        return Err(BosunError::error(
            "the bosun channel owns the tool pins in this repo",
            "run `splice update --channel bosun` (the splice update bot opens that PR); `bosun pin` is for the bosun repo",
        ));
    }
    let script = std::fs::read_to_string(root.join(SCRIPT))
        .map_err(|e| io_err("read", &root.join(SCRIPT), e))?;
    let tools: Vec<&str> = ["splice", "hoist", "berth", "bosun"]
        .into_iter()
        .filter(|t| a.tool.is_empty() || a.tool.iter().any(|x| x == t))
        .collect();
    let npm = Npm::new();

    // Compare every tool first; any refusal happens before a tarball is fetched or a file written.
    let (mut changed, mut unchanged, mut latest) = (Vec::new(), Vec::new(), Vec::new());
    for t in &tools {
        let key = &keys(t)[0];
        let cur = get_line(&script, key).ok_or_else(|| {
            BosunError::error(
                format!("{SCRIPT}: {key}= must appear exactly once"),
                format!("fix {SCRIPT} by hand"),
            )
        })?;
        let m = npm.meta(t, "latest")?;
        let sv = |v: &str, what: &str| {
            parse_semver(v).ok_or_else(|| {
                BosunError::error(
                    format!("{t}: {what} version {v:?} is not X.Y.Z"),
                    "pin this tool by hand",
                )
            })
        };
        let (from, to) = (sv(cur, "pinned")?, sv(&m.version, "latest")?);
        if to < from {
            return Err(BosunError::conflict(
                format!("{t}: npm latest {} is older than the pin {cur}", m.version),
                "pins only move up; check the npm dist-tag, or pin by hand",
            ));
        }
        if to == from {
            unchanged.push(json!(t));
        } else {
            changed.push(json!({"tool": t, "from": cur, "to": m.version}));
            latest.push((*t, m));
        }
    }
    if a.check {
        return Ok(Outcome::data(
            json!({"behind": changed, "unchanged": unchanged}),
        ));
    }

    // Verify and rewrite in memory; write only when everything passed.
    let tpl_path = root.join(TEMPLATE);
    let mut tpl = match std::fs::read_to_string(&tpl_path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err("read", &tpl_path, e)),
    };
    let (mut script_new, mut version_new) = (script.clone(), None);
    for (t, m) in &latest {
        let launcher = npm.verified(t, m)?;
        let arch = |suffix: &str| -> Result<String, BosunError> {
            let pkg = format!("{t}-linux-{suffix}");
            let am = npm.meta(&pkg, &m.version)?;
            if am.version != m.version {
                return Err(BosunError::error(
                    format!(
                        "npm @keelstackhq/{pkg}@{}: answered version {}",
                        m.version, am.version
                    ),
                    "do not pin this release; tell the owner, then rerun later",
                ));
            }
            npm.verified(&pkg, &am)
        };
        let pins = Pins {
            version: m.version.clone(),
            launcher,
            x64: arch("x64")?,
            arm64: arch("arm64")?,
        };
        let edit = |text: &str, file: &str| {
            apply(text, t, &pins).map_err(|e| {
                BosunError::error(format!("{file}: {e}"), format!("fix {file} by hand"))
            })
        };
        script_new = edit(&script_new, SCRIPT)?;
        if *t == "bosun" {
            version_new = Some(format!("{}\n", m.version));
        } else if let Some(text) = &tpl {
            tpl = Some(edit(text, TEMPLATE)?);
        }
    }
    let mut files = Vec::new();
    let mut outs = vec![
        (SCRIPT, Some(script_new)),
        (TEMPLATE, tpl.filter(|_| !latest.is_empty())),
    ];
    if root.join(VERSION_FILE).exists() {
        outs.push((VERSION_FILE, version_new));
    }
    for (rel, text) in outs {
        if let Some(text) = text {
            let p = root.join(rel);
            if std::fs::read_to_string(&p).ok().as_deref() != Some(&text) {
                std::fs::write(&p, text).map_err(|e| io_err("write", &p, e))?;
                files.push(rel);
            }
        }
    }
    let list: Vec<String> = latest
        .iter()
        .map(|(t, m)| format!("{t} {}", m.version))
        .collect();
    let title = if list.is_empty() {
        String::new()
    } else {
        format!("chore: pin {}", list.join(", "))
    };
    Ok(Outcome::data(json!({
        "changed": changed, "unchanged": unchanged, "files": files, "branch": "bosun/pin", "title": title,
    })))
}
