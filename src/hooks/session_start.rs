//! `bosun hook session-start`: join the registry, warm the lease cache and tell the session
//! where it stands (spec §4).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::cache;
use super::session_rules::rules_context;
use crate::app::{App, halt_reason, main_red};
use crate::cli::lesson;
use crate::core::lessons;
use crate::registry::{self, JoinSpec};

fn output(text: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}})
        .to_string()
}

/// `git` for the adopted check: never prompts, gives up on a stalled transfer (low speed) and
/// on one that takes over 20 seconds.
fn git_remote(dir: &Path, args: &[&str]) -> Option<String> {
    let mut child = std::process::Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["-c", "http.lowSpeedLimit=1000"])
        .args(["-c", "http.lowSpeedTime=15", "-C"])
        .arg(dir)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let end = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().ok()? {
            break s;
        }
        if Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    status.success().then(|| text.trim().to_string())
}

/// True when bosun was never set up in the repo at `cwd`: no `.bosun/version` in the worktree AND
/// none at the head of the remote's default branch. The remote answers (`git ls-remote`, then the
/// tree at that sha, fetched without blobs into a private repo if not local): local refs and files
/// can be edited. Fail closed: an unreachable remote counts as adopted, and so does no `origin`
/// with a `.bosun/` directory. The answer is cached per session for `BOSUN_ADOPTED_CACHE_SECS`
/// (default 600) in `<git common dir>/bosun/adopted`, so a burst of hooks asks the remote once.
pub fn unadopted(cwd: &Path) -> bool {
    let root = crate::ctx::git_out(cwd, &["rev-parse", "--show-toplevel"])
        .map_or_else(|| cwd.to_path_buf(), PathBuf::from);
    if root.join(".bosun/version").exists() {
        return false;
    }
    let common = super::git_out(&root, &["rev-parse", "--git-common-dir"]);
    let file = common.map(|c| root.join(c).join("bosun/adopted"));
    let session = std::env::var("BOSUN_SESSION").unwrap_or_default();
    let ttl = std::env::var("BOSUN_ADOPTED_CACHE_SECS").ok();
    let ttl: i64 = ttl.and_then(|v| v.trim().parse().ok()).unwrap_or(600);
    let now = crate::time::Timestamp::now().0;
    let cached = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok());
    if let Some(c) = &cached
        && let [at, answer, key] = c.splitn(3, ' ').collect::<Vec<_>>()[..]
        && at
            .parse::<i64>()
            .is_ok_and(|at| (0..ttl).contains(&(now - at)))
        && key == session
    {
        return answer == "1";
    }
    let answer = unadopted_remote(&root);
    if let Some(f) = file {
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(&root));
        let _ = std::fs::write(f, format!("{now} {} {session}", u8::from(answer)));
    }
    answer
}

/// A private bare repo under the temp dir, with an unpredictable name and mode 0700.
fn private_repo() -> Option<PathBuf> {
    let mut raw = [0u8; 12];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw).ok()?;
    let rnd = raw.iter().fold(String::new(), |a, b| format!("{a}{b:02x}"));
    let tmp = std::env::temp_dir().join(format!("bosun-adopted-{rnd}"));
    let mut dir = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(&tmp).ok()?;
    git_remote(&tmp, &["init", "-q", "--bare"]).map(|_| tmp)
}

fn unadopted_remote(root: &Path) -> bool {
    let Some(url) = git_remote(root, &["remote", "get-url", "origin"]) else {
        return !root.join(".bosun").is_dir();
    };
    let Some(heads) = git_remote(root, &["ls-remote", &url, "HEAD"]) else {
        return false;
    };
    let Some(sha) = heads.split_whitespace().next() else {
        return true; // reachable, no default branch yet
    };
    let spec = [format!("{sha}:.bosun/version"), format!("{sha}^{{commit}}")];
    if git_remote(root, &["cat-file", "-e", &spec[0]]).is_some() {
        return false;
    }
    if git_remote(root, &["cat-file", "-e", &spec[1]]).is_some() {
        return true;
    }
    let Some(tmp) = private_repo() else {
        return false;
    };
    let mut args: Vec<&str> = "fetch -q --depth=1 --filter=blob:none --no-tags"
        .split(' ')
        .collect();
    args.extend([&*url, sha]);
    let fetched = git_remote(&tmp, &args).is_some();
    let tree = git_remote(&tmp, &["ls-tree", "--name-only", sha, ".bosun/version"]);
    let _ = std::fs::remove_dir_all(&tmp);
    fetched && tree.is_some_and(|t| t.is_empty())
}

const NOT_SET_UP: &str = "bosun is not set up here: run `bosun adopt --policy autonomous \
    --owner <login>` (the owner's GitHub login) and open the setup PR. Until then bosun has no \
    registry, leases or approvals in this repo.";

pub fn run(cwd: Option<&Path>, input: &Value) -> String {
    let dir = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    if dir.as_deref().is_some_and(unadopted) {
        return output(NOT_SET_UP);
    }
    if let Some(d) = dir.as_deref()
        && let Err(e) = crate::cli::trailers::install(d)
    {
        eprintln!("bosun: trailers hook: {}", e.message);
    }
    let lessons = format!(
        "{}{}{}",
        skew_block(dir.as_deref()),
        lessons_block(dir.as_deref()),
        pin_block(dir.as_deref())
    );
    let registered = context(dir.as_deref());
    // After compaction the summary may have lost the state: add it back (spec §4). Built after
    // `context`, which refreshed the lease cache the block reads.
    let lessons = match dir.as_deref() {
        Some(d) if input.get("source").and_then(Value::as_str) == Some("compact") => {
            let block = super::compact::safe_block(d);
            format!(
                "{lessons}{}",
                block.map(|b| format!("\n{b}")).unwrap_or_default()
            )
        }
        _ => lessons,
    };
    match registered {
        Ok(text) => {
            // The janitor: expired leases go back to ready (spec §4); a failure is only a warning.
            if let Err(e) = crate::cli::reap::run(dir.as_deref()) {
                eprintln!("bosun: warning: reap: {}", e.message);
            }
            output(&format!(
                "{text}{lessons}\n{}",
                rules_context(dir.as_deref())
            ))
        }
        Err(why) if why.contains(".bosun/") => {
            eprintln!("bosun: warning: session start: {why}");
            output(&format!(
                "bosun cannot read its config, so this session is not registered: {why}. Fix the \
                 file, then run `bosun join`.{lessons}\n{}",
                rules_context(dir.as_deref())
            ))
        }
        Err(why) => {
            eprintln!("bosun: warning: session start: {why}");
            output(&format!(
                "bosun could not reach GitHub ({why}), so this session is not registered and \
                 edit locks may be stale. Set GH_TOKEN (or GITHUB_TOKEN) and BOSUN_REPO=owner/name, \
                 then run `bosun join`.{lessons}\n{}",
                rules_context(dir.as_deref())
            ))
        }
    }
}

/// The repo's global lessons (at most 8) as a block to append; empty when there are none or
/// `docs/lessons.toml` is missing or unreadable. Read before any GitHub call.
fn lessons_block(dir: Option<&Path>) -> String {
    let root = dir.map(|d| {
        crate::ctx::git_out(d, &["rev-parse", "--show-toplevel"])
            .map_or_else(|| d.to_path_buf(), PathBuf::from)
    });
    let Some(store) = root.and_then(|d| lesson::load(&d).ok()) else {
        return String::new();
    };
    let lines: Vec<String> = lessons::globals(&store)
        .into_iter()
        .take(lessons::MAX_GLOBAL)
        .map(lessons::line)
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("\nLessons:\n{}", lines.join("\n"))
    }
}

/// One line when the installed binary differs from the repo's `.bosun/version` pin.
fn skew_block(dir: Option<&Path>) -> String {
    let root = dir.map(|d| {
        crate::ctx::git_out(d, &["rev-parse", "--show-toplevel"])
            .map_or_else(|| d.to_path_buf(), PathBuf::from)
    });
    let pinned = root.and_then(|r| std::fs::read_to_string(r.join(".bosun/version")).ok());
    match pinned.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() && p != env!("CARGO_PKG_VERSION") => format!(
            "\ninstalled {}, repo pins {p}: run bash docs/setup-script.sh",
            env!("CARGO_PKG_VERSION")
        ),
        _ => String::new(),
    }
}

/// One line when a tool pin is behind npm (spec D35); empty otherwise, any error goes to stderr.
fn pin_block(dir: Option<&Path>) -> String {
    let Some(dir) = dir else {
        return String::new();
    };
    // BOSUN_PIN_CHECK=0 opts out of the lookup, so it must not read an old answer either.
    let store = (std::env::var("BOSUN_PIN_CHECK").as_deref() != Ok("0"))
        .then(|| cache::path(dir))
        .flatten()
        .map(|p| p.with_file_name("pin-notice.json"));
    let now = crate::time::Timestamp::now().0;
    if let Some(hit) = store.as_deref().and_then(|p| read_notice(p, now)) {
        return hit.map(|l| format!("\n{l}")).unwrap_or_default();
    }
    match crate::cli::pin::session_notice(dir) {
        Ok(line) => {
            if let Some(p) = store.as_deref() {
                let body = json!({"at": now, "line": line});
                let _ = std::fs::create_dir_all(p.parent().unwrap_or(dir))
                    .and_then(|()| std::fs::write(p, body.to_string()));
            }
            line.map(|l| format!("\n{l}")).unwrap_or_default()
        }
        Err(e) => {
            eprintln!("bosun: warning: pin check: {}", e.message);
            String::new()
        }
    }
}

/// npm is asked at most once a day per checkout: the answer is kept next to the lease cache.
const PIN_TTL_SECS: i64 = 86_400;

/// The kept answer (`Some(None)` is "nothing behind"); `None` when missing, old or unreadable.
fn read_notice(file: &Path, now: i64) -> Option<Option<String>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let at = v.get("at")?.as_i64()?;
    (0..PIN_TTL_SECS)
        .contains(&(now - at))
        .then(|| v.get("line").and_then(Value::as_str).map(String::from))
}

fn context(dir: Option<&Path>) -> Result<String, String> {
    let app = App::load(dir).map_err(|e| e.message)?;
    let (_, member) = registry::join(&app, JoinSpec::default()).map_err(|e| e.message)?;
    let file = dir.and_then(cache::path);
    let leases = cache::refresh(&app, file.as_deref()).map_err(|e| e.message)?;
    let mine: Vec<String> = leases
        .iter()
        .filter(|l| l.session == app.ctx.session)
        .map(|l| format!("#{}", l.issue))
        .collect();
    let own = if mine.is_empty() {
        "none".to_string()
    } else {
        mine.join(", ")
    };
    let halt = !matches!(halt_reason(app.gh.as_ref()), Ok(None));
    let red = match main_red(app.gh.as_ref()).ok().flatten() {
        Some(n) => format!("Main is red: open main-red issue #{n}."),
        None => "No open main-red issue.".to_string(),
    };
    Ok(format!(
        "bosun: you are session {} (origin {}). Own live leases: {own}. Kill switch: {}. {red} \
         Run `bosun status` for the full picture.",
        member.session,
        member.origin,
        if halt {
            "on or unreadable (work commands exit 4)"
        } else {
            "off"
        },
    ))
}
