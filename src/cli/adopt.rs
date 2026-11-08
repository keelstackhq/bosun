//! `bosun adopt`: write bosun's files into a plain repo (no hull, splice or hoist) and make the
//! labels and registry issue. Files only land in the worktree; no commit, no push, no PR.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::AdoptArgs;
use super::init::{has_ci_ok_job_except, is_semver};
use crate::app::Result;
use crate::core::adopt::{MANIFEST_PATH, Manifest, fnv1a_hex};
use crate::core::render::{Vars, render};
use crate::core::state::State;
use crate::ctx::{RepoId, git_out, parse_remote_url, parse_repo};
use crate::github::{Github, LiveGithub};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

struct Tpl {
    path: &'static str,
    src: &'static str,
    /// Only `.jinja` files are templates; the others are copied as they are.
    jinja: bool,
    exec: bool,
}

macro_rules! tpl {
    ($path:literal, $file:literal, $jinja:literal) => {
        Tpl {
            path: $path,
            src: include_str!(concat!("../../channel/pieces/core/files/", $file)),
            jinja: $jinja,
            exec: false,
        }
    };
}

const APPROVERS: &str = "# Enrolled approval keys (spec §8). Add one `[[key]]` per approver:\n\
# id = \"alice\", ed25519 = \"<base64 raw public key>\", role = \"owner\" (or \"delegate\").\n\
# risk-check reads this file from the default branch.\n";

const OWNERS: Tpl = tpl!(
    "governance/owners.toml",
    "governance/owners.toml.jinja",
    true
);

/// The files adopt writes from the channel's core piece (rendered with the subset renderer).
fn templates() -> Vec<Tpl> {
    vec![
        tpl!(".bosun/bosun.toml", ".bosun/bosun.toml.jinja", true),
        tpl!(".bosun/version", ".bosun/version.jinja", true),
        tpl!(".bosun/limits.toml", ".bosun/limits.toml", false),
        tpl!(".bosun/risk.toml", ".bosun/risk.toml.jinja", true),
        tpl!(
            ".bosun/session-rules.toml",
            ".bosun/session-rules.toml",
            false
        ),
        Tpl {
            path: ".bosun/approvers.toml",
            src: APPROVERS,
            jinja: false,
            exec: false,
        },
        Tpl {
            exec: true,
            ..tpl!(
                ".bosun/fold-fragments.sh",
                ".bosun/fold-fragments.sh",
                false
            )
        },
        tpl!(
            ".github/workflows/bosun.yml",
            ".github/workflows/bosun.yml.jinja",
            true
        ),
        tpl!(
            ".github/workflows/risk.yml",
            ".github/workflows/risk.yml",
            false
        ),
        tpl!(
            ".github/workflows/verdict.yml",
            ".github/workflows/verdict.yml",
            false
        ),
        tpl!(
            ".github/workflows/main-open.yml",
            ".github/workflows/main-open.yml",
            false
        ),
        tpl!(
            ".github/workflows/main-health.yml",
            ".github/workflows/main-health.yml.jinja",
            true
        ),
        tpl!(".claude/settings.json", ".claude/settings.json", false),
        tpl!("docs/setup-script.sh", "docs/setup-script.sh.jinja", true),
    ]
}

impl Tpl {
    fn text(&self, vars: &Vars) -> std::result::Result<String, String> {
        if self.jinja {
            render(self.src, vars)
        } else {
            Ok(self.src.to_string())
        }
    }
}

/// Every file adopt can write (with the owners file), rendered; for the render tests.
pub fn render_all(vars: &Vars) -> std::result::Result<Vec<(&'static str, String)>, String> {
    let mut all = templates();
    all.push(OWNERS);
    all.iter()
        .map(|t| t.text(vars).map(|s| (t.path, s)))
        .collect()
}

/// The variables of a render (the channel's `[vars]`; `intake` stays off).
pub fn vars_for(
    project: &str,
    owner: &str,
    version: &str,
    stacks: &[String],
    own_ci: bool,
) -> Vars {
    let mut v = Vars {
        pieces: stacks.to_vec(),
        ..Vars::default()
    };
    v.set("project", project);
    v.set("owner", owner);
    v.set("bosun_version", version);
    v.set("template.version", version);
    v.set("rust_path", ".");
    v.set("ts_path", ".");
    v.set("own_ci", if own_ci { "1" } else { "" });
    v.set("intake", "");
    v
}

fn detect(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if root.join("Cargo.toml").is_file() {
        v.push("rust".to_string());
    }
    if root.join("package.json").is_file() && root.join("pnpm-lock.yaml").is_file() {
        v.push("ts-pnpm".to_string());
    }
    v
}

fn labels_wanted() -> Vec<(String, &'static str, &'static str)> {
    let mut v = vec![
        ("bosun:halt".to_string(), "B60205", "kill switch"),
        (
            registry::LABEL.to_string(),
            "5319e7",
            "bosun session registry",
        ),
    ];
    v.extend(
        State::ALL
            .iter()
            .map(|s| (s.label(), "ededed", "bosun task state")),
    );
    v
}

/// What the owner still has to do. `owner_listed`: governance/owners.toml names a login (just
/// written, or already there); `kept_owners`: the file exists but names nobody (with the
/// `--owner` login, maybe empty); `has_key`: .bosun/approvers.toml holds a `[[key]]`.
fn owner_steps(
    own_ci: bool,
    owner_listed: bool,
    kept_owners: Option<&str>,
    has_key: bool,
    settings_url: Option<&str>,
) -> Vec<String> {
    // With a ci.yml of its own, bosun's aggregate job is `bosun-ok` and `ci-ok` stays yours.
    let first = if own_ci { "bosun-ok" } else { "ci-ok" };
    let mut v = Vec::new();
    if let (false, Some(login)) = (owner_listed, kept_owners) {
        let login = if login.is_empty() {
            "<your-github-login>"
        } else {
            login
        };
        v.push(format!("Add yourself to governance/owners.toml (adopt keeps an existing file): [[owner]] github = \"{login}\"."));
    } else if !owner_listed {
        v.push("Rerun with `bosun adopt --owner <your-github-login>` (it writes governance/owners.toml).".to_string());
    }
    if !has_key {
        v.push("Make your approval key: `bosun keygen --enroll` (writes the private key outside the repo and adds the public key to .bosun/approvers.toml).".to_string());
    }
    let place = settings_url.map_or_else(
        || "the default branch".to_string(),
        |u| format!("the default branch at {u}"),
    );
    v.push(format!("Set branch protection on {place} with the required checks {first}, risk-check, review-verdict and main-open."));
    v.push("Cloud sessions: paste `bosun setup-snippet` once as the setup script of your Claude Code environment (claude.ai/code > Environments); it never needs a change.".to_string());
    v.push("Run the commands in `data.commands`, then open the setup PR and merge it yourself. The checks cannot run on main before it merges.".to_string());
    v.push("Run `bosun doctor --remote` to see what is still missing.".to_string());
    v
}

/// The shell commands that publish the setup branch; `git add` lists exactly the paths adopt
/// wrote (a dry run wrote nothing, so it gets none), plus the file `bosun keygen --enroll` will
/// change when the keygen step is still shown.
fn commands(changes: &[Value], enroll_target: Option<&str>) -> Vec<String> {
    let mut add: Vec<String> = changes
        .iter()
        .filter(|c| matches!(c["action"].as_str(), Some("create" | "update")))
        .filter_map(|c| c["path"].as_str())
        .map(str::to_string)
        .chain(enroll_target.map(str::to_string))
        .collect();
    add.sort();
    add.dedup();
    if add.is_empty() {
        // Nothing new to commit (a rerun): only the check is left.
        return vec!["bosun doctor".to_string()];
    }
    vec![
        "git switch bosun-setup 2>/dev/null || git switch -c bosun-setup".to_string(),
        format!("git add {}", add.join(" ")),
        r#"git commit -m "chore: set up bosun""#.to_string(),
        "git push -u origin bosun-setup".to_string(),
        "bosun doctor".to_string(),
    ]
}

/// Every path adopt can write; `bosun uninstall` deletes nothing else.
pub fn known_paths() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = templates().iter().map(|t| t.path).collect();
    v.push(OWNERS.path);
    v
}

/// The first component of `rel` under `root` that is a symlink (dangling or not), if any.
/// bosun never reads, writes or deletes through one.
pub(crate) fn symlink_in(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Some(p);
        }
    }
    None
}

/// The bytes of a file; `None` only when it does not exist. Any other failure is an error.
pub(crate) fn read_opt(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err("read", path, e)),
    }
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> BosunError {
    BosunError::error(
        format!("cannot {what} {}: {e}", path.display()),
        "make the repo directory writable, then rerun `bosun adopt`",
    )
}

pub fn run(dir: Option<&Path>, a: &AdoptArgs) -> CmdResult {
    let start = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir().map_err(|e| io_err("read", Path::new("."), e))?,
    };
    let root: PathBuf = git_out(&start, &["rev-parse", "--show-toplevel"])
        .map_or_else(|| start.clone(), PathBuf::from);
    let owner = a.owner.clone().unwrap_or_default();
    if !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(BosunError::usage(
            format!("--owner {owner:?} is not a GitHub login"),
            "pass letters, digits and hyphens only",
        ));
    }
    let version = a
        .bosun_version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    if !is_semver(&version) {
        return Err(BosunError::usage(
            format!("--bosun-version {version:?} is not X.Y.Z"),
            "pass a release such as 0.7.0",
        ));
    }
    let stacks: Vec<String> = if a.stack.is_empty() {
        detect(&root)
    } else {
        let mut v: Vec<String> = Vec::new();
        for s in a.stack.iter().filter(|s| *s != "other") {
            if !v.contains(s) {
                v.push(s.clone());
            }
        }
        v
    };
    let repo: Option<RepoId> = std::env::var("BOSUN_REPO")
        .ok()
        .and_then(|v| parse_repo(&v))
        .or_else(|| {
            git_out(&root, &["remote", "get-url", "origin"]).and_then(|u| parse_remote_url(&u))
        });
    if repo.is_none() && !a.no_remote {
        return Err(BosunError::error(
            "cannot tell which GitHub repo this is (no BOSUN_REPO and no parseable origin remote)",
            "set BOSUN_REPO=owner/name, or pass --no-remote to write only the files",
        ));
    }
    let project = repo.as_ref().map_or_else(
        || {
            let base = root
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into());
            let clean: String = base
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "._-".contains(c) {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            if clean.is_empty() {
                "app".to_string()
            } else {
                clean
            }
        },
        |r| r.name.clone(),
    );
    let own_ci = has_ci_ok_job_except(&root, "bosun.yml");
    let vars = vars_for(&project, &owner, &version, &stacks, own_ci);

    if let Some(link) = symlink_in(&root, MANIFEST_PATH) {
        return Err(BosunError::error(
            format!(
                "{} is a symlink; bosun does not write through symlinks",
                link.display()
            ),
            "replace it with a real directory or file, then rerun `bosun adopt`",
        ));
    }
    let old_manifest =
        read_opt(&root.join(MANIFEST_PATH))?.map(|b| String::from_utf8_lossy(&b).into_owned());
    let mut manifest = match &old_manifest {
        Some(t) => Manifest::parse(t).map_err(|e| {
            BosunError::error(
                format!("{MANIFEST_PATH}: {e}"),
                format!("fix or delete {MANIFEST_PATH}, then rerun `bosun adopt`"),
            )
        })?,
        None => Manifest::default(),
    };
    manifest.bosun_version.clone_from(&version);

    // Plan the files.
    let mut tpls = templates();
    if !owner.is_empty() {
        tpls.push(OWNERS);
    }
    let mut changes: Vec<Value> = Vec::new();
    let mut writes: Vec<(&Tpl, String)> = Vec::new();
    for t in &tpls {
        let text = t.text(&vars).map_err(|e| {
            BosunError::error(
                format!("template {}: {e}", t.path),
                "this is a bosun bug; report it",
            )
        })?;
        if let Some(link) = symlink_in(&root, t.path) {
            changes.push(json!({
                "path": t.path, "action": "kept",
                "fix": format!("{} is a symlink; bosun does not write through symlinks: replace it with a real file or directory, then rerun `bosun adopt`", link.display()),
            }));
            continue;
        }
        match read_opt(&root.join(t.path))? {
            Some(old) if old == text.as_bytes() => {
                changes.push(json!({"path": t.path, "action": "unchanged"}));
            }
            Some(_) => changes.push(json!({
                "path": t.path, "action": "kept",
                "fix": format!("{} exists and differs from bosun's version; edit it by hand, or delete it and rerun `bosun adopt`", t.path),
            })),
            None => {
                changes.push(json!({"path": t.path, "action": "create"}));
                writes.push((t, text));
            }
        }
    }

    // GitHub. What was created is recorded even when a later step fails.
    let (mut labels, mut registry_out) = (Vec::new(), json!({"action": "skipped"}));
    let mut last = old_manifest.clone();
    if !a.no_remote {
        let r = repo.as_ref().expect("checked above");
        match LiveGithub::new(&r.owner, &r.name) {
            Ok(gh) => {
                let res = remote(
                    &gh,
                    a.dry_run,
                    &mut manifest,
                    &mut labels,
                    &mut registry_out,
                );
                if !a.dry_run {
                    persist(&root, &mut manifest, &mut last)?;
                }
                res?;
            }
            Err(e) if a.dry_run => {
                labels = labels_wanted()
                    .iter()
                    .map(|(n, _, _)| json!({"name": n, "action": "unknown"}))
                    .collect();
                registry_out = json!({"action": "unknown", "reason": e.message});
            }
            Err(e) => return Err(e),
        }
    }

    if !a.dry_run {
        for (t, text) in &writes {
            record_dirs(&root, t.path, &mut manifest);
            write_file(&root.join(t.path), text, t.exec)?;
            manifest.set_file(t.path, fnv1a_hex(text.as_bytes()));
            persist(&root, &mut manifest, &mut last)?;
        }
    }
    // The manifest, last of the changes.
    let mut planned = manifest.clone();
    for (t, text) in &writes {
        planned.set_file(t.path, fnv1a_hex(text.as_bytes()));
    }
    let manifest_action = match &old_manifest {
        Some(old) if *old == planned.to_toml() => "unchanged",
        Some(_) => "update",
        None => "create",
    };
    changes.push(json!({"path": MANIFEST_PATH, "action": manifest_action}));
    // A file lists `table` when it parses and holds at least one `[[table]]`.
    let listed = |rel: &str, table: &str| {
        read_opt(&root.join(rel)).ok().flatten().is_some_and(|b| {
            String::from_utf8_lossy(&b)
                .parse::<toml::Table>()
                .ok()
                .and_then(|t| t.get(table)?.as_array().map(|a| !a.is_empty()))
                .unwrap_or(false)
        })
    };
    let owner_listed = (!owner.is_empty() && action_of(&changes, OWNERS.path) == "create")
        || listed(OWNERS.path, "owner");
    // An owners file that exists but names nobody: adopt never overwrites it, so edit it.
    let owners_file = (!owner_listed && root.join(OWNERS.path).exists()).then_some(owner.as_str());
    let has_key = listed(".bosun/approvers.toml", "key");
    let settings_url = git_out(&root, &["remote", "get-url", "origin"])
        .filter(|u| u.contains("github.com"))
        .and_then(|u| parse_remote_url(&u))
        .map(|r| {
            format!(
                "https://github.com/{}/{}/settings/branches",
                r.owner, r.name
            )
        });
    Ok(Outcome::data(json!({
        "stacks": stacks,
        "dry_run": a.dry_run,
        "changes": changes,
        "labels": labels,
        "registry": registry_out,
        "commands": if a.dry_run { Vec::new() } else { commands(&changes, (!has_key).then_some(".bosun/approvers.toml").filter(|p| root.join(p).exists())) },
        "owner_steps": owner_steps(own_ci, owner_listed, owners_file, has_key, settings_url.as_deref()),
    })))
}

fn action_of<'a>(changes: &'a [Value], path: &str) -> &'a str {
    changes
        .iter()
        .find(|c| c["path"] == path)
        .and_then(|c| c["action"].as_str())
        .unwrap_or("")
}

/// Write the manifest when its text differs from what is on disk (`last`).
fn persist(root: &Path, m: &mut Manifest, last: &mut Option<String>) -> Result<()> {
    record_dirs(root, MANIFEST_PATH, m);
    let text = m.to_toml();
    if last.as_deref() != Some(text.as_str()) {
        write_file(&root.join(MANIFEST_PATH), &text, false)?;
        *last = Some(text);
    }
    Ok(())
}

/// Note in the manifest each directory above `rel` that does not exist yet (adopt creates it).
fn record_dirs(root: &Path, rel: &str, m: &mut Manifest) {
    let parts: Vec<&str> = rel.split('/').collect();
    for i in 1..parts.len() {
        let dir = parts[..i].join("/");
        if !root.join(&dir).exists() {
            m.add_dir(&dir);
        }
    }
}

fn write_file(path: &Path, text: &str, exec: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err("create", parent, e))?;
    }
    std::fs::write(path, text).map_err(|e| io_err("write", path, e))?;
    #[cfg(unix)]
    if exec {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| io_err("chmod", path, e))?;
    }
    #[cfg(not(unix))]
    let _ = exec;
    Ok(())
}

/// Ensure the labels and the registry issue; a dry run only reads.
fn remote(
    gh: &dyn Github,
    dry_run: bool,
    manifest: &mut Manifest,
    labels: &mut Vec<Value>,
    registry_out: &mut Value,
) -> Result<()> {
    for (name, color, desc) in labels_wanted() {
        if gh.label_exists(&name)? {
            labels.push(json!({"name": name, "action": "exists"}));
            continue;
        }
        if !dry_run {
            gh.ensure_label(&name, color, desc)?;
            manifest.add_label(&name);
        }
        labels.push(json!({"name": name, "action": "create"}));
    }
    *registry_out = match registry::load(gh)? {
        Some((n, _)) => json!({"action": "exists", "number": n}),
        None if dry_run => json!({"action": "create"}),
        None => {
            let (n, _) = registry::ensure(gh)?;
            manifest.registry_issue = Some(n);
            json!({"action": "create", "number": n})
        }
    };
    Ok(())
}
