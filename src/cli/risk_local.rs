//! `bosun risk-check --local` (spec §8): judge `git diff origin/main...HEAD` before a push,
//! offline. Path rules and R-weaken as CI's risk-check reads them; nothing is posted, and what
//! needs a PR (linked tasks, approvals, reverts, one-off size) is left to CI.

use std::path::Path;
use std::process::Command;

use serde_json::json;

use super::risk_check::{Files, is_manifest, rules_from, weaken_from};
use crate::app::Result;
use crate::core::risk::{self, Approval, ChangedFile, added_lines, evaluate, new_dependencies};
use crate::core::weaken;
use crate::github::PrFile;
use crate::output::{BosunError, CmdResult, Outcome};

struct Local<'a>(&'a Path);

impl Local<'_> {
    fn git(&self, args: &[&str]) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.0)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl Files for Local<'_> {
    fn read(&self, path: &str, at: &str) -> Result<Option<String>> {
        let obj = format!("{at}:{path}");
        if self.git(&["cat-file", "-e", &obj]).is_none() {
            return Ok(None);
        }
        let text = self.git(&["cat-file", "blob", &obj]);
        text.map(Some).ok_or_else(|| {
            BosunError::error(format!("cannot read {obj}"), "check the git checkout")
        })
    }
    fn paths(&self, at: &str) -> Option<Vec<String>> {
        Some(
            self.git(&["ls-tree", "-r", "--name-only", at])?
                .lines()
                .map(String::from)
                .collect(),
        )
    }
}

pub fn run(dir: Option<&Path>) -> CmdResult {
    // Paths are from the repo root, so git runs there, whatever directory the command ran in.
    let top = Local(dir.unwrap_or(Path::new("."))).git(&["rev-parse", "--show-toplevel"]);
    let top = top.unwrap_or_else(|| ".".into());
    let src = Local(Path::new(top.trim()));
    let fail = |what: &str| {
        BosunError::error(
            format!("risk-check --local: cannot read {what}"),
            "run it in a git checkout whose `origin/main` is fetched: `git fetch origin main`",
        )
    };
    let base = src
        .git(&["merge-base", "origin/main", "HEAD"])
        .ok_or_else(|| fail("origin/main"))?;
    let head = src
        .git(&["rev-parse", "HEAD"])
        .ok_or_else(|| fail("HEAD"))?;
    let (base, head) = (base.trim(), head.trim());
    let names = src.git(&[
        "diff",
        "--no-ext-diff",
        "-z",
        "--name-status",
        "-M",
        base,
        head,
    ]);
    let names = names.ok_or_else(|| fail("the diff"))?;
    let mut parts = names.split('\0').filter(|s| !s.is_empty());
    let mut pr_files = Vec::new();
    while let Some(st) = parts.next() {
        let renamed = st.starts_with('R') || st.starts_with('C');
        let prev = if renamed { parts.next() } else { None };
        let Some(path) = parts.next() else { break };
        let status = match &st[..1] {
            "A" => "added",
            "D" => "removed",
            "R" => "renamed",
            _ => "modified",
        };
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-textconv",
            "--no-ext-diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-M",
            base,
            head,
            "--",
        ];
        args.extend(prev);
        args.push(path);
        pr_files.push(PrFile {
            filename: path.into(),
            status: status.into(),
            additions: 0,
            deletions: 0,
            patch: src.git(&args).filter(|p| !p.is_empty()),
            previous_filename: prev.map(String::from),
            sha: None,
        });
    }
    let rules = rules_from(&src, base)?;
    let files: Vec<ChangedFile> = (pr_files.iter())
        .map(|f| ChangedFile {
            path: f.filename.clone(),
            previous_path: f.previous_filename.clone(),
            added_lines: f.patch.as_deref().map(added_lines).unwrap_or_default(),
        })
        .collect();
    let mut manifests = Vec::new();
    for f in pr_files.iter().filter(|f| is_manifest(&f.filename)) {
        let old = f.previous_filename.as_deref().unwrap_or(&f.filename);
        manifests.push((
            f.filename.clone(),
            src.read(old, base)?,
            src.read(&f.filename, head)?,
        ));
    }
    let findings = weaken_from(&src, &pr_files, base, head)?;
    let caps: Vec<String> = match findings.is_empty() {
        true => Vec::new(),
        false => vec![weaken::CAPABILITY.into()],
    };
    let mut matches = evaluate(&rules, &files, &new_dependencies(&manifests), &caps);
    if !findings.is_empty() {
        matches.retain(|m| m.rule != weaken::RULE);
        matches.push(risk::Match {
            rule: weaken::RULE.into(),
            why: weaken::WHY.into(),
            approval: Approval::Owner,
            evidence: findings,
        });
    }
    let level = risk::level(&matches);
    let data =
        json!({"local": true, "base": base, "head": head, "level": level, "matches": matches});
    if matches.is_empty() {
        return Ok(Outcome::data(data));
    }
    let rules: Vec<&str> = matches.iter().map(|m| m.rule.as_str()).collect();
    Err(BosunError::needs_human(
        format!("risk-check --local: high risk ({})", rules.join(", ")),
        "push if the change is meant; the PR then needs the approval CI's risk-check names \
         (R-weaken: the owner's), else drop the matching change first",
    )
    .with_data(data))
}
