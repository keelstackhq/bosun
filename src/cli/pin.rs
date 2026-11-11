//! `bosun pin --latest`: move the tool pins in this repo to the newest npm release of splice,
//! hoist, berth and bosun, after checking every tarball against npm's `dist.integrity` (spec D35).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::PinArgs;
use crate::app::{App, default_branch};
use crate::core::pin::{Pins, apply, get_line, keys, parse_semver, sha512_of};
use crate::ctx::{RepoId, git_out};
use crate::github::b64;
use crate::output::{BosunError, CmdResult, Outcome};

const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";
const SCRIPT: &str = "docs/setup-script.sh";
const TEMPLATE: &str = "channel/pieces/core/files/docs/setup-script.sh.jinja";
const VERSION_FILE: &str = ".bosun/version";
const CHANNEL: &str = ".splice/channels/bosun.toml";
const BRANCH: &str = "bosun/pin";

/// What npm says about one package version.
struct Meta {
    version: String,
    integrity: String,
    tarball: String,
}

struct Npm {
    base: String,
    agent: ureq::Agent,
    /// Every request must end before this; `None` leaves each request its own 120 s.
    deadline: Option<Instant>,
}

impl Npm {
    fn new(total: Option<Duration>) -> Npm {
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
            deadline: total.map(|t| Instant::now() + t),
        }
    }

    fn get(&self, url: &str, what: &str, limit: u64) -> Result<Vec<u8>, BosunError> {
        let fail = |d: String| {
            BosunError::error(
                format!("npm {what}: {d}"),
                "check the network and BOSUN_NPM_REGISTRY, then run `bosun pin --latest` again",
            )
        };
        let req = self.agent.get(url);
        let resp = match self.deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(fail("timed out".into()));
                }
                req.config().timeout_global(Some(left)).build().call()
            }
            None => req.call(),
        };
        let mut resp = resp.map_err(|e| fail(e.to_string()))?;
        if resp.status().as_u16() != 200 {
            return Err(fail(format!("HTTP {}", resp.status().as_u16())));
        }
        let body = resp.body_mut().with_config().limit(limit).read_to_vec();
        body.map_err(|e| fail(e.to_string()))
    }

    /// Metadata of `pkg` at `version` (`latest` or X.Y.Z).
    fn meta(&self, pkg: &str, version: &str) -> Result<Meta, BosunError> {
        // Every fetch starts here; tarballs must sit under this base.
        if !crate::anthropic::https_or_loopback(&self.base) {
            return Err(BosunError::error(
                "BOSUN_NPM_REGISTRY must be an https:// URL (http:// only for 127.0.0.1 or localhost)",
                "unset BOSUN_NPM_REGISTRY to use https://registry.npmjs.org, or set an https:// URL",
            ));
        }
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

/// What `inspect` found: the script text, the tools behind, the tools current, and npm's metadata
/// for those behind.
type Found<'a> = (
    String,
    Vec<Value>,
    Vec<Value>,
    Vec<(&'a str, Meta)>,
    Option<String>,
);

/// Read the pins of `root` and compare them with npm latest (nothing is written or downloaded).
fn inspect<'a>(root: &Path, only: &'a [String], npm: &Npm) -> Result<Found<'a>, BosunError> {
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
        .filter(|t| only.is_empty() || only.iter().any(|x| x == t))
        .collect();
    // Compare every tool first; any refusal happens before a tarball is fetched or a file written.
    let (mut changed, mut unchanged, mut latest) = (Vec::new(), Vec::new(), Vec::new());
    let mut bosun_latest = None;
    for t in &tools {
        let key = &keys(t)[0];
        let cur = get_line(&script, key).ok_or_else(|| {
            BosunError::error(
                format!("{SCRIPT}: {key}= must appear exactly once"),
                format!("fix {SCRIPT} by hand"),
            )
        })?;
        let m = npm.meta(t, "latest")?;
        if *t == "bosun" {
            bosun_latest = Some(m.version.clone());
        }
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
    Ok((script, changed, unchanged, latest, bosun_latest))
}

pub fn run(dir: Option<&Path>, a: &PinArgs) -> CmdResult {
    let start = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir().map_err(|e| io_err("read", Path::new("."), e))?,
    };
    let root: PathBuf =
        git_out(&start, &["rev-parse", "--show-toplevel"]).map_or(start, PathBuf::from);
    let npm = Npm::new(None);
    let (script, changed, unchanged, latest, _) = inspect(&root, &a.tool, &npm)?;
    if a.check {
        return Ok(Outcome::data(
            json!({"behind": changed, "unchanged": unchanged}),
        ));
    }

    let pre = if a.pr && !changed.is_empty() {
        Some(super::pin_pr::preflight(&root)?)
    } else {
        None
    };
    if let Some(super::pin_pr::Pre::Open(..)) = pre {
        let data = json!({"changed": changed, "unchanged": unchanged, "files": [], "branch": BRANCH, "title": ""});
        return super::pin_pr::publish(&root, pre.unwrap(), &[], "", "", data, None);
    }

    // Plain path: the open pin PR and the free branch name, before anything is written (both
    // names taken refuses here, dropping nothing).
    let plain = if pre.is_none() && !changed.is_empty() {
        let found = open_pin_pr(&root);
        let free = super::pin_pr::free_branch(&root);
        if !matches!(&found, Ok((_, Some(_), _))) {
            free.clone()?;
        }
        Some((found, free))
    } else {
        None
    };

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
    let mut data = json!({
        "changed": changed, "unchanged": unchanged, "files": files, "branch": BRANCH, "title": title,
    });
    if files.is_empty() {
        return Ok(Outcome::data(data));
    }
    let moves: Vec<String> = changed
        .iter()
        .map(|c| {
            format!(
                "- {} {} -> {}",
                c["tool"].as_str().unwrap_or(""),
                c["from"],
                c["to"]
            )
            .replace('"', "")
        })
        .collect();
    let body = format!(
        "Tool pins moved by `bosun pin --latest` (tarballs checked against npm dist.integrity):\n{}\n\nGated R-gov: needs owner approval.",
        moves.join("\n")
    );
    if let Some(pre) = pre {
        let to = latest
            .iter()
            .find(|(t, _)| *t == "bosun")
            .map(|(_, m)| m.version.as_str());
        return super::pin_pr::publish(&root, pre, &files, &title, &body, data, to);
    }
    let (found, free) = plain.expect("plain path");
    let d = data.as_object_mut().expect("object");
    let mut call =
        json!({"head": BRANCH, "base": "main", "draft": true, "title": title, "body": body});
    match found {
        Ok((_, Some((n, head)), _)) => {
            d.insert("branch".into(), json!(head));
            d.insert("pr".into(), json!(n));
            d.insert("steps".into(), json!([
                format!("PR #{n} is the open pin PR: do not open another."),
                format!("Commit the files on branch {head}, merge origin/<default branch> into it first (never force-push), then push to refresh the PR."),
            ]));
            return Ok(Outcome::data(data));
        }
        Ok((repo, None, base)) => {
            // A stale `bosun/pin` left on origin or here gets the dated name, as with --pr.
            let head = free?;
            d.insert("branch".into(), json!(head));
            call["head"] = json!(head);
            call["owner"] = json!(repo.owner);
            call["repo"] = json!(repo.name);
            call["base"] = json!(base);
            d.insert("pr".into(), Value::Null);
        }
        Err(e) => {
            d.insert("pr".into(), Value::Null);
            d.insert("pr_lookup".into(), json!(e.message));
            let head = free?;
            d.insert("branch".into(), json!(head));
            call["head"] = json!(head);
            if let Ok(ctx) = crate::ctx::Ctx::load(Some(&root)) {
                call["owner"] = json!(ctx.repo.owner);
                call["repo"] = json!(ctx.repo.name);
            }
        }
    }
    Ok(Outcome::data(data).with_next("mcp__github__create_pull_request", call))
}

/// The repo, the number of the open PR from `bosun/pin` (if any) and the default branch.
type OpenPin = (RepoId, Option<(u64, String)>, String);

fn open_pin_pr(root: &Path) -> Result<OpenPin, BosunError> {
    let app = App::load(Some(root))?;
    // An open PR needs no base: a failing base lookup must not hide it.
    if let Some(found) = super::pin_pr::find_open(app.gh.as_ref(), &app.ctx.repo.to_string())? {
        return Ok((app.ctx.repo.clone(), Some(found), String::new()));
    }
    let base = default_branch(app.gh.as_ref())?;
    Ok((app.ctx.repo.clone(), None, base))
}

/// What `bosun doctor` and the SessionStart notice report: the tool pins behind npm latest, and
/// the newest bosun when this binary is older.
pub struct Update {
    pub moves: Vec<String>,
    pub cli: Option<String>,
    /// A bosun channel repo: its pins move with `splice update --channel bosun`, not `bosun pin`.
    pub channel: bool,
}

/// The result of the update check.
pub enum Check {
    /// Not checked on purpose: why, and the fix.
    Skipped(String, String),
    /// The check itself failed: why, and the fix.
    Failed(String, String),
    /// Pins and binary match npm latest.
    Current,
    Behind(Update),
}

/// `latest` when it is newer than this binary. A prerelease binary is never flagged: it has no X.Y.Z.
fn newer(latest: Option<&str>) -> Option<String> {
    let have = parse_semver(env!("CARGO_PKG_VERSION"))?;
    let v = latest?;
    (parse_semver(v)? > have).then(|| v.to_string())
}

const NPM_FIX: &str = "check the network and BOSUN_NPM_REGISTRY, then rerun";

/// Waits 5 s at most; never fails (an error is `Skipped`).
pub fn update_check(dir: &Path) -> Check {
    if std::env::var("BOSUN_PIN_CHECK").is_ok_and(|v| v == "0") {
        return Check::Skipped(
            "BOSUN_PIN_CHECK=0".into(),
            "unset BOSUN_PIN_CHECK to check for updates".into(),
        );
    }
    let root: PathBuf =
        git_out(dir, &["rev-parse", "--show-toplevel"]).map_or(dir.to_path_buf(), PathBuf::from);
    let channel = root.join(CHANNEL).exists();
    if !channel && !root.join(SCRIPT).exists() {
        return Check::Skipped(
            format!("no {SCRIPT}: not a repo with tool pins"),
            format!("add {SCRIPT} (bosun adopt does), or ignore"),
        );
    }
    let npm = Npm::new(Some(Duration::from_secs(5)));
    // Npm's own errors start with "npm ": the network or the registry; any other is the repo's.
    let failed = |e: &BosunError| {
        if e.message.starts_with("npm ") {
            Check::Failed(format!("npm unreachable: {}", e.message), NPM_FIX.into())
        } else {
            Check::Failed(format!("pin check failed: {}", e.message), e.fix.clone())
        }
    };
    let (moves, latest) = if channel {
        // The pins are the channel's; only the binary is compared.
        match npm.meta("bosun", "latest") {
            Ok(m) => (Vec::new(), Some(m.version)),
            Err(e) => return failed(&e),
        }
    } else {
        match inspect(&root, &[], &npm) {
            Ok((_, behind, _, _, latest)) => {
                let moves = behind
                    .iter()
                    .map(|c| {
                        format!(
                            "{} {} -> {}",
                            c["tool"].as_str().unwrap_or(""),
                            c["from"],
                            c["to"]
                        )
                        .replace('"', "")
                    })
                    .collect();
                (moves, latest)
            }
            Err(e) => {
                // The binary check does not depend on the pin compare: try it on its own.
                let latest = npm.meta("bosun", "latest").ok().map(|m| m.version);
                if newer(latest.as_deref()).is_none() {
                    return failed(&e);
                }
                eprintln!("bosun: warning: pin check: {}", e.message);
                (Vec::new(), latest)
            }
        }
    };
    // A prerelease binary is never flagged: it has no X.Y.Z to compare.
    let cli = newer(latest.as_deref());
    if moves.is_empty() && cli.is_none() {
        if channel {
            return Check::Skipped(
                "bosun channel repo: its tool pins move with the channel".into(),
                "splice update --channel bosun".into(),
            );
        }
        return Check::Current;
    }
    Check::Behind(Update {
        moves,
        cli,
        channel,
    })
}

/// The SessionStart notice (spec D35): `Some(line)` when a tool pin is behind npm latest; `None`
/// when the hook should stay silent (opted out, not a pin repo, nothing behind).
pub fn session_notice(dir: &Path) -> Result<Option<String>, BosunError> {
    // No npm call in a bosun channel repo: its pins are the channel's (spec 04, SessionStart).
    let root: PathBuf =
        git_out(dir, &["rev-parse", "--show-toplevel"]).map_or(dir.to_path_buf(), PathBuf::from);
    if root.join(CHANNEL).exists() {
        return Ok(None);
    }
    Ok(match update_check(dir) {
        Check::Behind(u) if !u.moves.is_empty() => Some(format!(
            "tool pins behind: {}. The orchestrator moves them: on the default branch run `bosun pin --latest --pr` (branch {BRANCH}, commit, push, draft PR; one pin PR per repo).",
            u.moves.join(", ")
        )),
        Check::Failed(why, fix) => return Err(BosunError::error(why, fix)),
        _ => None,
    })
}
