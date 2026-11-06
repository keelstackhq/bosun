//! `bosun adopt`: write bosun's files into a plain repo (no hull, splice or hoist) and make the
//! labels and registry issue. Files only land in the worktree; no commit, no push, no PR.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::AdoptArgs;
use super::init::has_ci_ok_job_except;
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

const OWNER_STEPS: [&str; 4] = [
    "Add your approver key to .bosun/approvers.toml and your login to governance/owners.toml.",
    "Set branch protection on the default branch with the required checks ci-ok, risk-check, review-verdict and main-open.",
    "Review and merge the setup PR yourself: git add .bosun .github .claude governance, commit, push, open the PR. The checks cannot run on main before it merges.",
    "Run `bosun doctor --remote` to see what is still missing.",
];

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

    let mut manifest = match std::fs::read_to_string(root.join(MANIFEST_PATH)) {
        Ok(t) => Manifest::parse(&t).map_err(|e| {
            BosunError::error(
                format!("{MANIFEST_PATH}: {e}"),
                format!("fix or delete {MANIFEST_PATH}, then rerun `bosun adopt`"),
            )
        })?,
        Err(_) => Manifest::default(),
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
        match std::fs::read(root.join(t.path)) {
            Ok(old) if old == text.as_bytes() => {
                changes.push(json!({"path": t.path, "action": "unchanged"}));
            }
            Ok(_) => changes.push(json!({
                "path": t.path, "action": "kept",
                "fix": format!("{} exists and differs from bosun's version; edit it by hand, or delete it and rerun `bosun adopt`", t.path),
            })),
            Err(_) => {
                changes.push(json!({"path": t.path, "action": "create"}));
                manifest.set_file(t.path, fnv1a_hex(text.as_bytes()));
                writes.push((t, text));
            }
        }
    }

    // GitHub.
    let (mut labels, mut registry_out) = (Vec::new(), json!({"action": "skipped"}));
    if !a.no_remote {
        let r = repo.as_ref().expect("checked above");
        let gh = LiveGithub::new(&r.owner, &r.name)?;
        remote(
            &gh,
            a.dry_run,
            &mut manifest,
            &mut labels,
            &mut registry_out,
        )?;
    }

    // The manifest, last of the changes.
    let text = manifest.to_toml();
    let manifest_action = match std::fs::read_to_string(root.join(MANIFEST_PATH)) {
        Ok(old) if old == text => "unchanged",
        Ok(_) => "update",
        Err(_) => "create",
    };
    changes.push(json!({"path": MANIFEST_PATH, "action": manifest_action}));

    if !a.dry_run {
        for (t, text) in &writes {
            write_file(&root.join(t.path), text, t.exec)?;
        }
        if manifest_action != "unchanged" {
            write_file(&root.join(MANIFEST_PATH), &text, false)?;
        }
    }
    Ok(Outcome::data(json!({
        "stacks": stacks,
        "dry_run": a.dry_run,
        "changes": changes,
        "labels": labels,
        "registry": registry_out,
        "owner_steps": OWNER_STEPS,
    })))
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
