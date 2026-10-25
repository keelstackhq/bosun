//! `bosun init`: add bosun's splice channel to a repo and open one setup PR (spec D16, §12 M7).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use super::InitArgs;
use crate::app::{App, default_branch};
use crate::core::checks;
use crate::output::{BosunError, CmdResult, Outcome};

/// Files the channel seeds (written once, never overwritten); init excludes those that exist.
const SEEDS: [&str; 9] = [
    ".bosun/limits.toml",
    ".bosun/risk.toml",
    "governance/owners.toml",
    ".github/CODEOWNERS",
    ".github/workflows/splice.yml",
    "hoist.toml",
    "deny.toml",
    "rust-toolchain.toml",
    ".npmrc",
];
/// The channel's shared file; excluded when the repo has it, which leaves the hooks uninstalled.
const SETTINGS: &str = ".claude/settings.json";
const HOOKS_STEP: &str = "The bosun hooks are missing from .claude/settings.json, which another template owns: take them from that template (hull adds them in hull#22, then `splice update`) instead of editing the file.";
const HOIST_CHECKS: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];
const BRANCH: &str = "bosun/init";

const OWNER_STEPS: [&str; 6] = [
    "Fill governance/owners.toml with the owners' GitHub logins (the channel seeds it from --owner).",
    "Run `hoist apply` for branch protection, auto-merge and the required checks ci-ok, risk-check, review-verdict, main-open (hoist.toml).",
    "Create the kill-switch label: `gh label create bosun:halt --color B60205`.",
    "Add the secret SPLICE_BOT_TOKEN (a fine-grained or app token) so the splice update bot's PRs run CI.",
    "Review this setup PR; it is R-gov, so approve its head SHA (`/bosun approve <sha>`). While main has no bosun workflows, risk-check, review-verdict and main-open cannot run on it: merge it yourself, then apply the required checks.",
    "Create the Routines (janitor, curator, notify); `bosun doctor` prints their prompts.",
];

fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn git(root: &Path, args: &[&str]) -> Result<String, BosunError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| BosunError::error(format!("cannot run git: {e}"), "install git"))?;
    if !out.status.success() {
        return Err(BosunError::error(
            format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "fix the git error above, then rerun `bosun init`",
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run splice; its stdout is one JSON object `{ok, changes, messages}`.
fn run_splice(argv: &[String]) -> Result<Value, BosunError> {
    let bin = std::env::var("BOSUN_SPLICE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "splice".into());
    let out = Command::new(&bin)
        .args(argv)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| {
            BosunError::error(
                format!("cannot start {bin}: {e}"),
                "install splice (`npm install -g @keelstackhq/splice`) or set BOSUN_SPLICE",
            )
        })?;
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    if out.status.success() && parsed["ok"] == true {
        return Ok(parsed);
    }
    let detail = match parsed["messages"].as_array() {
        Some(m) => m
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("; "),
        None => parsed.to_string(),
    };
    Err(BosunError::error(
        format!("splice failed (exit {:?}): {detail}", out.status.code()),
        "fix the splice error above, then rerun `bosun init`",
    )
    .with_data(json!({ "splice": parsed })))
}

/// Does a splice lock (`.splice/lock.toml`, or a channel's `.splice/channels/*.toml`) list
/// CLAUDE.md under `[files]`? Then a template owns it. Unreadable files count as no.
fn main_owns_claude_md(root: &Path) -> bool {
    let mut paths = vec![root.join(".splice/lock.toml")];
    if let Ok(dir) = std::fs::read_dir(root.join(".splice/channels")) {
        paths.extend(
            dir.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml")),
        );
    }
    paths.iter().any(|p| {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|t| t.get("files").and_then(|f| f.as_table().cloned()))
            .is_some_and(|f| f.contains_key("CLAUDE.md"))
    })
}

/// Does a workflow under `.github/workflows` report the check `ci-ok` (a job named so, or keyed
/// so without a `name:`)? Then the repo has its own required-check gate.
fn has_ci_ok_job(root: &Path) -> bool {
    let Ok(dir) = std::fs::read_dir(root.join(".github/workflows")) else {
        return false;
    };
    dir.flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "yml" || x == "yaml")
        })
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .any(|t| checks::workflow_checks(&t).iter().any(|c| c == "ci-ok"))
}

/// Span of the `[...]` array value found at or after `from`: `(open, close, end of last element)`.
/// Skips strings and comments.
fn array_span(text: &str, from: usize) -> Option<(usize, usize, Option<usize>)> {
    let open = from + text[from..].find('[')?;
    let (mut depth, mut last_end) = (0usize, None);
    let mut it = text[open..].char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, open + i, last_end));
                }
            }
            '#' => while it.next_if(|&(_, n)| n != '\n').is_some() {},
            '"' | '\'' => {
                while let Some((_, n)) = it.next() {
                    if n == '\\' && c == '"' {
                        it.next();
                    } else if n == c {
                        break;
                    }
                }
                if depth == 1 {
                    last_end = Some(it.peek().map_or(text.len(), |&(j, _)| open + j));
                }
            }
            _ => {}
        }
    }
    None
}

/// Why hoist.toml cannot get the checks: not TOML, or a form init cannot edit safely.
#[derive(Debug, PartialEq)]
enum HoistErr {
    Invalid(String),
    ByHand(Vec<String>),
}

impl HoistErr {
    fn into_error(self) -> BosunError {
        match self {
            HoistErr::Invalid(m) => BosunError::error(
                format!("hoist.toml is not valid TOML: {m}"),
                "make hoist.toml valid TOML, then rerun `bosun init`",
            ),
            HoistErr::ByHand(add) => BosunError::error(
                "init cannot edit [github] required_checks in hoist.toml safely",
                format!(
                    "add {} to [github] required_checks in hoist.toml by hand, then rerun `bosun init`",
                    add.join(", ")
                ),
            ),
        }
    }
}

fn checks_in(doc: &toml::Table) -> Option<Vec<&str>> {
    let rc = doc.get("github")?.get("required_checks")?.as_array()?;
    Some(rc.iter().filter_map(toml::Value::as_str).collect())
}

/// The text edit alone: `None` when the layout is not one it knows how to edit.
fn edit_checks(text: &str, quoted: &[String], has_github: bool) -> Option<String> {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    // Where the [github] header ends and where the required_checks line starts.
    let (mut header, mut key, mut pos) = (None, None, 0);
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if header.is_some() && t.starts_with('[') {
            break;
        }
        let is_github = t
            .strip_prefix("[github]")
            .is_some_and(|r| r.trim().is_empty() || r.trim().starts_with('#'));
        if header.is_none() && is_github {
            header = Some(pos + line.len());
        } else if header.is_some()
            && key.is_none()
            && t.strip_prefix("required_checks")
                .is_some_and(|r| r.trim_start().starts_with('='))
        {
            key = Some(pos);
        }
        pos += line.len();
    }
    let mut out = text.to_string();
    match (header, key) {
        (_, Some(k)) => {
            let (open, close, last) = array_span(text, k)?;
            let line_end = last.and_then(|e| text[e..].find('\n').map(|n| e + n));
            let multi_tail = line_end.is_some_and(|le| close > le);
            if let (true, Some(e), Some(le)) = (multi_tail, last, line_end) {
                // Items go on their own lines after the last one; its comment stays on its line.
                let from = text[..e].rfind('\n').map_or(0, |n| n + 1);
                let indent: String = text[from..]
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                let rest = text[e..].trim_start_matches([' ', '\t']);
                let comma = rest.starts_with(',');
                let at = if text[..le].ends_with('\r') {
                    le - 1
                } else {
                    le
                };
                let items: Vec<String> =
                    quoted.iter().map(|q| format!("{nl}{indent}{q}")).collect();
                out.insert_str(
                    at,
                    &format!("{}{}", items.join(","), if comma { "," } else { "" }),
                );
                if !comma {
                    out.insert(e, ',');
                }
            } else if last.is_none() && text[open..close].contains('\n') {
                let items = quoted.join(&format!(",{nl}  "));
                out.insert_str(open + 1, &format!("{nl}  {items},"));
            } else {
                let at = last.unwrap_or(open + 1);
                let sep = if last.is_some() { ", " } else { "" };
                out.insert_str(at, &format!("{sep}{}", quoted.join(", ")));
            }
        }
        (Some(h), None) => {
            let lead = if text[..h].ends_with('\n') { "" } else { nl };
            out.insert_str(
                h,
                &format!("{lead}required_checks = [{}]{nl}", quoted.join(", ")),
            );
        }
        (None, None) => {
            if has_github {
                return None;
            }
            if !out.is_empty() {
                out += if out.ends_with('\n') { nl } else { "\n\n" };
            }
            out += &format!("[github]{nl}required_checks = [{}]{nl}", quoted.join(", "));
        }
    }
    Some(out)
}

/// Pure: make `[github] required_checks` in hoist.toml `text` contain every name in `want`,
/// keeping the entries there and appending the missing ones in order. A minimal text edit, so
/// comments and layout stay; the result is parsed again and must hold every wanted check, else
/// the edit is refused. Returns the new text and the names added.
fn add_required_checks(text: &str, want: &[&str]) -> Result<(String, Vec<String>), HoistErr> {
    let doc: toml::Table = text
        .parse()
        .map_err(|e| HoistErr::Invalid(format!("{e}")))?;
    let rc = doc.get("github").and_then(|g| g.get("required_checks"));
    let have = checks_in(&doc);
    let add: Vec<String> = want
        .iter()
        .filter(|w| !have.as_deref().unwrap_or_default().contains(w))
        .map(|w| (*w).to_string())
        .collect();
    if add.is_empty() {
        return Ok((text.to_string(), add));
    }
    if rc.is_some_and(|c| !c.is_array()) {
        return Err(HoistErr::ByHand(add));
    }
    let quoted: Vec<String> = add.iter().map(|a| format!("\"{a}\"")).collect();
    let out = edit_checks(text, &quoted, doc.contains_key("github"))
        .filter(|o| {
            o.parse::<toml::Table>()
                .is_ok_and(|d| checks_in(&d).is_some_and(|c| want.iter().all(|w| c.contains(w))))
        })
        .ok_or_else(|| HoistErr::ByHand(add.clone()))?;
    Ok((out, add))
}

fn pr_body(pieces: &[String], changes: &Value, kept: &[String], steps: &[String]) -> String {
    let files: Vec<&str> = changes
        .as_array()
        .map(|c| c.iter().filter_map(|x| x["path"].as_str()).collect())
        .unwrap_or_default();
    let mut s = format!(
        "Sets up bosun (`bosun init`): the splice channel `bosun` with the pieces {}.\n\nFiles: `{}`.\n",
        pieces.join(", "),
        files.join("`, `")
    );
    if !kept.is_empty() {
        s += &format!("\nKept the repo's own `{}`.\n", kept.join("`, `"));
    }
    s += "\nRisk: high (R-gov). It needs the owner's approval on the head SHA.\n\nOwner steps:\n";
    for (i, step) in steps.iter().enumerate() {
        s += &format!("{}. {step}\n", i + 1);
    }
    s
}

/// Splice, the plan and the commit on the init branch: `(splice changes, plan path)`.
fn setup(
    root: &Path,
    args: &InitArgs,
    argv: &[String],
    hoist: &Option<(String, Vec<String>)>,
) -> Result<(Value, Option<String>, Vec<String>), BosunError> {
    let splice = run_splice(argv)?;
    let mut added = Vec::new();
    if let Some((text, new)) = hoist {
        if !new.is_empty() {
            std::fs::write(root.join("hoist.toml"), text).map_err(|e| {
                BosunError::error(
                    format!("cannot write hoist.toml: {e}"),
                    "check that hoist.toml is writable",
                )
            })?;
        }
        added.clone_from(new);
    }
    let changes = splice.get("changes").cloned().unwrap_or_else(|| json!([]));
    let mut plan = None;
    if let Some(src) = &args.plan {
        let rel = format!(
            "plans/{}",
            src.file_name().unwrap_or_default().to_string_lossy()
        );
        let dest = root.join(&rel);
        // A plan already at plans/<name> stays as it is (copying a file onto itself empties it).
        let same =
            dest.exists() && std::fs::canonicalize(src).ok() == std::fs::canonicalize(&dest).ok();
        if !same {
            std::fs::create_dir_all(root.join("plans"))
                .and_then(|()| std::fs::copy(src, &dest))
                .map_err(|e| {
                    BosunError::error(
                        format!("cannot copy the plan to {rel}: {e}"),
                        "check that plans/ is writable",
                    )
                })?;
        }
        plan = Some(rel);
    }
    git(root, &["add", "-A"])?;
    // A runner with no git identity still commits, as bosun.
    let mut commit: Vec<&str> = Vec::new();
    if git(root, &["config", "user.email"]).is_err() {
        commit.extend([
            "-c",
            "user.name=bosun",
            "-c",
            "user.email=bosun@users.noreply.github.com",
        ]);
    }
    commit.extend(["commit", "-q", "-m", "chore: set up bosun (bosun init)"]);
    git(root, &commit)?;
    Ok((changes, plan, added))
}

/// Undo a failed init: drop what splice wrote and the branch, and go back to where it started
/// (a branch, or a commit when HEAD was detached). The tree was clean before, so nothing of the
/// user's is lost. False when a step failed.
fn rollback(root: &Path, start: &str, detached: bool) -> bool {
    let back = if detached { "--detach" } else { "--no-guess" };
    [
        vec!["reset", "-q", "--hard"],
        vec!["clean", "-q", "-fd"],
        vec!["switch", "-q", back, start],
        vec!["branch", "-q", "-D", BRANCH],
    ]
    .iter()
    .all(|args| git(root, args).is_ok())
}

pub fn run(dir: Option<&Path>, args: &InitArgs) -> CmdResult {
    for (i, s) in args.stack.iter().enumerate() {
        if args.stack[..i].contains(s) {
            return Err(BosunError::usage(
                format!("--stack {s} is given twice"),
                "give each --stack once",
            ));
        }
    }
    if let Some(v) = args.bosun_version.as_deref().filter(|v| !is_semver(v)) {
        return Err(BosunError::usage(
            format!("--bosun-version {v:?} is not X.Y.Z"),
            "pass an exact release, such as --bosun-version 0.1.0",
        ));
    }
    if let Some(p) = args.plan.as_ref().filter(|p| !p.is_file()) {
        return Err(BosunError::usage(
            format!("plan file {} does not exist", p.display()),
            "pass --plan with the path of an existing file",
        ));
    }
    if let Some(o) = args
        .owner
        .as_deref()
        .filter(|o| o.is_empty() || !o.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    {
        return Err(BosunError::usage(
            format!("--owner {o:?} is not a GitHub login"),
            "pass the owner's login, such as --owner octocat",
        ));
    }
    if args.source.starts_with('-') {
        return Err(BosunError::usage(
            format!("--source {:?} looks like a flag", args.source),
            "pass a splice source, such as gh:keelstackhq/bosun//channel",
        ));
    }
    let root =
        crate::cli::gate::repo_root(&dir.map_or_else(|| PathBuf::from("."), Path::to_path_buf))?;
    if root.join(".splice/channels/bosun.toml").exists() {
        return Err(BosunError::conflict(
            "this repo already has the bosun channel",
            "the splice update bot keeps it fresh; `splice update --channel bosun` updates it by hand",
        ));
    }
    if !git(&root, &["status", "--porcelain"])?.is_empty() {
        return Err(BosunError::error(
            "the working tree is not clean",
            "commit or stash your changes, then rerun `bosun init`",
        ));
    }
    if git(
        &root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{BRANCH}"),
        ],
    )
    .is_ok()
    {
        return Err(BosunError::conflict(
            format!("branch {BRANCH} already exists"),
            "finish that setup, or delete the branch (`git branch -D bosun/init`) and rerun",
        ));
    }

    let version = env!("CARGO_PKG_VERSION");
    let git_ref = args
        .git_ref
        .clone()
        .unwrap_or_else(|| format!("v{version}"));
    // The repo name needs no token: BOSUN_REPO or the origin remote, else the directory name.
    let project = crate::ctx::Ctx::load(Some(&root))
        .map(|c| c.repo.name)
        .unwrap_or_else(|_| {
            root.file_name()
                .map_or("project".into(), |n| n.to_string_lossy().into_owned())
        });
    let kept: Vec<String> = SEEDS
        .into_iter()
        .chain([SETTINGS])
        .filter(|p| root.join(p).exists())
        .map(String::from)
        .collect();
    let own_ci = has_ci_ok_job(&root);
    let claude_md = !main_owns_claude_md(&root);
    let mut checks = HOIST_CHECKS.to_vec();
    if own_ci {
        checks.push("bosun-ok");
    }
    // A dry run of the hoist.toml edit, so a bad file fails before anything is rendered.
    let hoist = if kept.iter().any(|p| p == "hoist.toml") {
        let text = std::fs::read_to_string(root.join("hoist.toml")).map_err(|e| {
            BosunError::error(
                format!("cannot read hoist.toml: {e}"),
                "make hoist.toml a readable file, then rerun `bosun init`",
            )
        })?;
        Some(add_required_checks(&text, &checks).map_err(HoistErr::into_error)?)
    } else {
        None
    };
    let mut pieces = args.stack.clone();
    if claude_md {
        pieces.push("claude-md".into());
    }
    let mut steps: Vec<String> = OWNER_STEPS.iter().map(|s| (*s).to_string()).collect();
    if own_ci {
        steps[1] = steps[1].replace("main-open (hoist.toml)", "main-open, bosun-ok (hoist.toml)");
    }
    let hooks_present = std::fs::read_to_string(root.join(SETTINGS)).is_ok_and(|t| {
        t.contains("bosun hook pre-tool-use") && t.contains("bosun hook session-start")
    });
    if kept.iter().any(|p| p == SETTINGS) && !hooks_present {
        steps.push(HOOKS_STEP.into());
    }

    let mut argv: Vec<String> = vec!["channel".into(), "add".into(), args.source.clone()];
    argv.extend([
        "--as".into(),
        "bosun".into(),
        "--ref".into(),
        git_ref.clone(),
    ]);
    for s in &pieces {
        argv.extend(["--piece".into(), s.clone()]);
    }
    let mut data = vec![format!("project={project}")];
    if own_ci {
        data.push("own_ci=1".into());
    }
    data.extend(args.owner.as_ref().map(|o| format!("owner={o}")));
    // splice versions a branch or sha ref as 0.0.0+g<sha>, which is no valid pin: name one.
    let release_ref = git_ref.strip_prefix('v').is_some_and(is_semver);
    if args.bosun_version.is_some() || !release_ref {
        data.push(format!(
            "bosun_version={}",
            args.bosun_version.as_deref().unwrap_or(version)
        ));
    }
    for d in data {
        argv.extend(["--data".into(), d]);
    }
    for p in &kept {
        argv.extend(["--exclude".into(), p.clone()]);
    }
    argv.extend(["--yes", "--json", "-C"].map(String::from));
    argv.push(root.display().to_string());

    // Read the token and the base branch before changing anything, so a missing token fails
    // before the branch exists. No kill-switch check: a fresh repo has no labels yet.
    let github = if args.no_pr {
        None
    } else {
        let app = App::load(Some(&root))?;
        let base = default_branch(app.gh.as_ref())?;
        Some((app, base))
    };
    let start = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let detached = start == "HEAD";
    let start = if detached {
        git(&root, &["rev-parse", "HEAD"])?
    } else {
        start
    };

    git(&root, &["switch", "-q", "-c", BRANCH])?;
    let local = setup(&root, args, &argv, &hoist).and_then(|(changes, plan, added)| {
        if github.is_some() {
            git(&root, &["push", "-q", "-u", "origin", BRANCH])?;
        }
        Ok((changes, plan, added))
    });
    let (changes, plan, added) = match local {
        Ok(v) => v,
        Err(e) => {
            let fix = if rollback(&root, &start, detached) {
                format!("bosun init rolled back to {start}; {}", e.fix)
            } else {
                format!(
                    "bosun init could not roll back fully: check `git status`, switch to {start} and \
                     delete bosun/init; then: {}",
                    e.fix
                )
            };
            return Err(BosunError { fix, ..e });
        }
    };
    let commit = git(&root, &["rev-parse", "HEAD"])?;

    let mut pr = None;
    if let Some((app, base)) = &github {
        let body = pr_body(&pieces, &changes, &kept, &steps);
        let pull = app
            .gh
            .create_pull("chore: set up bosun", BRANCH, base, &body)
            .map_err(|e| BosunError {
                fix: format!(
                    "bosun/init is pushed: open its PR by hand with \
                     `gh pr create --head bosun/init --base {base} --title \"chore: set up bosun\"`"
                ),
                ..e
            })?;
        pr = Some(json!({"number": pull.number, "head_sha": pull.head_sha}));
    }

    Ok(Outcome::data(json!({
        "channel": "bosun", "source": args.source, "ref": git_ref, "pieces": pieces,
        "changes": changes, "excluded": kept, "plan": plan, "branch": BRANCH,
        "commit": commit, "pr": pr, "risk": "high", "owner_steps": steps,
        "own_ci": own_ci, "claude_md": claude_md, "hoist_checks_added": added,
    })))
}

#[cfg(test)]
mod tests {
    use super::add_required_checks;

    const ALL: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];

    fn edit(t: &str) -> (String, Vec<String>) {
        let (out, added) = add_required_checks(t, &ALL).unwrap();
        out.parse::<toml::Table>().unwrap();
        (out, added)
    }

    #[test]
    fn single_line_appends_in_order_and_keeps_comments() {
        let (out, added) = edit("# top\n[github]\nrequired_checks = [\"ci-ok\"] # why\nx = 1\n");
        assert_eq!(
            out,
            "# top\n[github]\nrequired_checks = [\"ci-ok\", \"risk-check\", \"review-verdict\", \"main-open\"] # why\nx = 1\n"
        );
        assert_eq!(added, ["risk-check", "review-verdict", "main-open"]);
    }

    #[test]
    fn multi_line_appends_with_the_same_indent() {
        let (out, _) =
            edit("[github]\nrequired_checks = [\n    \"ci-ok\", # first\n    \"risk-check\"\n]\n");
        assert_eq!(
            out,
            "[github]\nrequired_checks = [\n    \"ci-ok\", # first\n    \"risk-check\",\n    \"review-verdict\",\n    \"main-open\"\n]\n"
        );
    }

    #[test]
    fn missing_key_and_missing_table_are_added() {
        let (out, added) = edit("[github]\nauto_merge = true\n");
        assert!(
            out.starts_with("[github]\nrequired_checks = [\"ci-ok\""),
            "{out}"
        );
        assert!(out.contains("auto_merge = true"));
        assert_eq!(added.len(), 4);
        let (out, _) = edit("# c\n[other]\na = 1\n");
        assert!(
            out.starts_with("# c\n[other]\na = 1\n\n[github]\nrequired_checks = ["),
            "{out}"
        );
    }

    #[test]
    fn complete_is_unchanged_and_invalid_is_an_error() {
        let t = "[github]\nrequired_checks = [\"main-open\", \"ci-ok\", \"risk-check\", \"review-verdict\"]\n";
        assert_eq!(edit(t), (t.to_string(), vec![]));
        assert!(matches!(
            add_required_checks("[github\n", &ALL),
            Err(super::HoistErr::Invalid(_))
        ));
    }

    /// Whatever the layout, the result is valid TOML holding every wanted check, or a refusal.
    fn ok_or_refused(t: &str) {
        match add_required_checks(t, &ALL) {
            Ok((out, _)) => {
                let d: toml::Table = out.parse().unwrap();
                let c = super::checks_in(&d).unwrap();
                assert!(ALL.iter().all(|w| c.contains(w)), "{out}");
            }
            Err(super::HoistErr::ByHand(add)) => assert!(!add.is_empty()),
            Err(e) => panic!("{e:?}"),
        }
    }

    #[test]
    fn odd_layouts_are_edited_correctly_or_refused() {
        // A quoted key.
        ok_or_refused("[github]\n\"required_checks\" = [\"ci-ok\"]\n");
        // A [foo] line inside a multi-line string, before the real table.
        ok_or_refused("note = \"\"\"\n[github]\n\"\"\"\n[foo]\ns = '''\n[bar]\n'''\n");
        // required_checks is a string, followed by another table.
        ok_or_refused("[github]\nrequired_checks = \"ci-ok\"\n[release]\nx = 1\n");
        // Dotted key, inline table, spaced header.
        ok_or_refused("github.required_checks = [\"ci-ok\"]\n");
        ok_or_refused("github = { required_checks = [\"ci-ok\"] }\n");
        ok_or_refused("[ github ]\nrequired_checks = [\"ci-ok\"]\n");
        for t in [
            "[github]\nrequired_checks = \"ci-ok\"\n[release]\nx = 1\n",
            "[ github ]\nrequired_checks = [\"ci-ok\"]\n",
        ] {
            assert!(matches!(
                add_required_checks(t, &ALL),
                Err(super::HoistErr::ByHand(_))
            ));
        }
    }

    #[test]
    fn comments_stay_on_their_line_and_crlf_is_kept() {
        let (out, _) = edit("[github]\nrequired_checks = [\n  \"ci-ok\", # keep\n]\n");
        assert_eq!(
            out,
            "[github]\nrequired_checks = [\n  \"ci-ok\", # keep\n  \"risk-check\",\n  \"review-verdict\",\n  \"main-open\",\n]\n"
        );
        let (out, _) = edit("[github]\r\nrequired_checks = [\"ci-ok\"]\r\n");
        assert!(
            out.ends_with("\"main-open\"]\r\n") && !out.contains("\"\n"),
            "{out:?}"
        );
        let (out, _) = edit("[other]\r\na = 1\r\n");
        assert!(
            out.ends_with("]\r\n") && out.contains("\r\n[github]\r\n"),
            "{out:?}"
        );
    }
}
