//! `bosun risk-check <pr>`: judge a PR with the risk rules of its base branch (spec section 8).
//! The rules, the owners and the manifests at base are read at the base SHA, never from the head.

use std::path::Path;

use serde_json::json;

use super::PrArg;
use super::carry_over::{candidates, carry_approval, carry_verdict, has_verdict, is_verdict};
use super::verdict::CONTEXT as VERDICT;
use crate::app::{App, Result, default_branch, default_ref, halt_reason, task_meta, trusted_notes};
use crate::config::{CONFIG_FILE, Config};
use crate::core::approve::{
    APPROVERS_PATH, ApproverKey, Form, OrchestratorCase, Policy, Role, orchestrator_counts,
    parse_approvers, signature_lines, signed_message, signed_message_autonomous,
    signed_message_bootstrap, signed_message_v2, trust_files_changed, verify_ed25519,
};
use crate::core::lease::leases_of;
use crate::core::meta::{closing_issues_in, linked_issues_in};
use crate::core::risk::{
    self, Approval, ChangedFile, FileView, Rules, VERDICTS_NEEDED, added_lines, apply_floor,
    capability_labels, clean_verdicts, comment_approves, evaluate, lease_sessions,
    new_dependencies, parse_owners, plain, replace_note, requirement, risk_note, two_verdicts,
    unseen_diff,
};
use crate::core::touch::{Changed, outside};
use crate::core::weaken;
use crate::ctx::RepoId;
use crate::gates::{adapters, ts};
use crate::github::gate::GateWriter;
use crate::github::{Comment, Github, Issue, PrFile, Pull, Review, b64};
use crate::output::{BosunError, CmdResult, Exit, Outcome};
use crate::time::{Timestamp, parse_rfc3339};

const RULES_PATH: &str = ".bosun/risk.toml";
pub const OWNERS_PATH: &str = "governance/owners.toml";
const CONTEXT: &str = "risk-check";
/// A base with this file carries bosun's own approval code, which is then critical too.
const BOSUN_CODE_MARKER: &str = "src/cli/risk_check.rs";

/// Owner logins from `governance/owners.toml` at `git_ref`; empty when the file is missing.
pub fn owner_logins(gh: &dyn Github, git_ref: &str) -> Result<Vec<String>> {
    let Some(text) = gh.file_at(OWNERS_PATH, git_ref)? else {
        return Ok(Vec::new());
    };
    let owners = parse_owners(&text).map_err(|e| {
        BosunError::error(
            e,
            format!("fix {OWNERS_PATH}: one `[[owner]]` table with `github = \"login\"` per owner"),
        )
    })?;
    Ok(owners.into_iter().map(|o| o.github).collect())
}

/// Slack IDs of the owners in `governance/owners.toml` at `git_ref`; empty when none is set.
pub fn owner_slack_ids(gh: &dyn Github, git_ref: &str) -> Result<Vec<String>> {
    let Some(text) = gh.file_at(OWNERS_PATH, git_ref)? else {
        return Ok(Vec::new());
    };
    let owners = parse_owners(&text).map_err(|e| {
        BosunError::error(
            e,
            format!("fix {OWNERS_PATH}: one `[[owner]]` table with `github = \"login\"` per owner"),
        )
    })?;
    Ok(owners.into_iter().filter_map(|o| o.slack).collect())
}

fn is_manifest(path: &str) -> bool {
    matches!(
        path.rsplit('/').next(),
        Some("Cargo.toml" | "package.json" | "pnpm-workspace.yaml")
    )
}

pub fn run(dir: Option<&Path>, args: &PrArg) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let n = args.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` lists the open PRs",
        )
    })?;
    let w = GateWriter::for_ctx(gh, &app.ctx);
    // In flight, a required check is pending, so auto-merge waits for this run's verdict.
    if let Err(e) = w.post(&pull.head_sha, CONTEXT, "pending", PENDING) {
        eprintln!(
            "bosun: risk-check: cannot post the pending status: {}",
            e.message
        );
    }
    let out = judge(&app, &w, pull.clone());
    // A run that cannot finish must not leave `pending` behind (exit 5 posted its verdict).
    if out.as_ref().is_err_and(|e| e.exit != Exit::NeedsHuman) {
        _ = w.post(&pull.head_sha, CONTEXT, "error", UNFINISHED);
    }
    out
}

const PENDING: &str = "risk-check is judging this head";
const UNFINISHED: &str = "risk-check could not finish; fix the error and re-run the risk workflow";

/// R-weaken evidence (spec §8): gate files compared at base and head, diffs, deleted or thinned
/// tests. Deduplicated, in file order.
fn weaken_findings(
    gh: &dyn Github,
    pr_files: &[PrFile],
    base: &str,
    head: &str,
) -> Result<Vec<String>> {
    let patches = || {
        pr_files
            .iter()
            .filter_map(|f| Some((&f.filename, f.patch.as_deref()?)))
    };
    let added: Vec<String> = patches()
        .flat_map(|(n, p)| weaken::ci_commands(n, p).1)
        .collect();
    let mut out = Vec::new();
    for f in pr_files {
        let p = f.patch.as_deref();
        out.extend(
            p.map(|p| weaken::patch_findings(&f.filename, p, &added))
                .unwrap_or_default(),
        );
        // Each side of a rename on its own: the path left behind falls back to its built-in (or
        // nothing), the new path is judged against what it replaces, never the moved file.
        let prev = f.previous_filename.as_deref().filter(|p| *p != f.filename);
        for (path, here) in [
            (f.filename.as_str(), true),
            (prev.unwrap_or_default(), false),
        ] {
            if path.is_empty() || !weaken::reads_text(path) {
                continue;
            }
            let at_base = f.status != "added" && !(here && prev.is_some());
            let old = match at_base {
                true => gh.file_at(path, base)?,
                false => builtin_base(path).map(str::to_string),
            };
            let new = match here && f.status != "removed" {
                true => gh.file_at(path, head)?,
                false => builtin_base(path).map(str::to_string),
            };
            out.extend(weaken::text_findings(path, old.as_deref(), new.as_deref()));
        }
    }
    out.extend(test_findings(gh, pr_files, base, head)?);
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.clone()));
    Ok(out)
}

/// The built-in file a new `.bosun/risk.toml` or `.bosun/profiles/<name>.toml` replaces.
fn builtin_base(path: &str) -> Option<&'static str> {
    if path == RULES_PATH {
        return Some(risk::DEFAULT_RULES);
    }
    let name = path
        .strip_prefix(".bosun/profiles/")?
        .strip_suffix(".toml")?;
    crate::gates::profile::builtin(name)
}

/// Test counts over whole files at base and head, totalled over the PR so moving tests between
/// files is no loss: tests, assertions, skips, `.only` and `cfg` switch-offs, as test-integrity
/// counts them. A file counts only where it runs: a Rust test file under a `src/` or `tests/`
/// root, a TS/JS test file. Also every deleted test file.
fn test_findings(gh: &dyn Github, files: &[PrFile], base: &str, head: &str) -> Result<Vec<String>> {
    let (mut out, mut c) = (Vec::new(), [[0usize; 5]; 2]);
    let mut lists: [std::collections::BTreeMap<String, Vec<String>>; 2] = Default::default();
    // A cap on reads: past it the owner judges, no quota is burnt and nothing errors.
    let many = files.iter().filter(|f| code_file(&f.filename)).count();
    if many > 100 {
        return Ok(vec![format!(
            "tests not counted: {many} code files; the owner judges"
        )]);
    }
    for f in files {
        let n = &f.filename;
        let prev = f.previous_filename.as_deref().unwrap_or(n);
        if f.status == "removed" && code_file(n) && risk::test_path(n) {
            out.push(format!("{n}: test file present → (deleted)"));
        }
        let sides = [
            (prev, base, f.status != "added"),
            (n.as_str(), head, f.status != "removed"),
        ];
        for (side, (path, at, exists)) in sides.into_iter().enumerate() {
            if !exists || !code_file(path) {
                continue;
            }
            match gh.file_at(path, at) {
                Ok(Some(t)) if t.len() <= 1 << 20 => {
                    let r = test_counts(path, &t);
                    (0..5).for_each(|i| c[side][i] += r[i]);
                    if path.to_ascii_lowercase().ends_with(".rs") {
                        for (k, v) in weaken::allowed_lists(&t) {
                            lists[side].entry(k).or_default().extend(v);
                        }
                    }
                }
                Ok(None) => {}
                _ => out.push(format!(
                    "{path}: tests not counted (unreadable or over 1 MB); the owner judges"
                )),
            }
        }
    }
    out.extend(weaken::allowed_findings(&lists[0], &lists[1]));
    let names = ["tests", "assertions", "skips", "only", "cfg switch-offs"];
    for (i, what) in names.iter().enumerate() {
        let (o, n) = (c[0][i], c[1][i]);
        if if i < 2 { n < o } else { n > o } {
            out.push(format!("{what} (all files) {o} → {n}"));
        }
    }
    Ok(out)
}

fn code_file(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let exts = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts",
    ];
    exts.iter().any(|e| p.ends_with(e))
}

/// `[tests, assertions, skips, only, cfg switch-offs]` of a file that runs, else zeros.
fn test_counts(path: &str, text: &str) -> [usize; 5] {
    let p = path.to_ascii_lowercase();
    if p.ends_with(".rs") {
        // What Cargo builds: inline tests under `src/`, `tests/*.rs` and `tests/<dir>/main.rs`.
        let segs: Vec<&str> = p.split('/').collect();
        // `src/` of a crate, not one under tests, examples, fixtures or vendored code.
        let src = segs.iter().position(|s| *s == "src").is_some_and(|i| {
            let not = [
                "tests", "examples", "benches", "fixtures", "vendor", "target", "docs",
            ];
            !segs[..i].iter().any(|s| not.contains(s))
        });
        let built = src
            || (segs.iter().rposition(|s| *s == "tests"))
                .is_some_and(|i| matches!(&segs[i + 1..], [_] | [_, "main.rs"]));
        let text = weaken::strip_block_comments(text);
        if !built || !adapters::is_test_file(&p, &text) {
            return [0; 5];
        }
        let off = text.lines().filter(|l| weaken::cfg_off(l)).count();
        return [
            adapters::test_count(&text),
            adapters::assertions(&text),
            adapters::pending_markers(&text).len(),
            0,
            off,
        ];
    }
    if !ts::is_test_file(&p) {
        return [0; 5];
    }
    [
        ts::test_count(text),
        ts::assertions(text),
        ts::pending_markers(text).len(),
        ts::only_count(text),
        0,
    ]
}

fn judge(app: &App, w: &GateWriter, pull: Pull) -> CmdResult {
    let gh = app.gh.as_ref();
    let n = pull.number;
    let (base, head) = (pull.base_sha.as_str(), pull.head_sha.as_str());

    let rules = base_rules(gh, base)?;

    let pr_files = gh.pull_files(n)?;
    let files: Vec<ChangedFile> = pr_files
        .iter()
        .map(|f| ChangedFile {
            path: f.filename.clone(),
            previous_path: f.previous_filename.clone(),
            added_lines: f.patch.as_deref().map(added_lines).unwrap_or_default(),
        })
        .collect();

    let mut manifests = Vec::new();
    for f in pr_files.iter().filter(|f| is_manifest(&f.filename)) {
        let old_path = f.previous_filename.as_deref().unwrap_or(&f.filename);
        let old = if f.status == "added" {
            None
        } else {
            gh.file_at(old_path, base)?
        };
        let new = if f.status == "removed" {
            None
        } else {
            gh.file_at(&f.filename, head)?
        };
        manifests.push((f.filename.clone(), old, new));
    }
    let new_deps = new_dependencies(&manifests);
    let findings = weaken_findings(gh, &pr_files, base, head)?;

    let mut capabilities = capability_labels(&pull.labels);
    if !findings.is_empty() {
        capabilities.push(weaken::CAPABILITY.into());
    }
    // Every linked issue counts: the agent writes the PR body, so a decoy first link must not
    // hide the real task's capabilities.
    let mut task = None;
    let mut plan_high = Vec::new();
    for i in linked_issues_in(&pull.body, &app.ctx.repo.to_string()) {
        if let Some(issue) = gh.get_issue(i)?
            && let Some(meta) = task_meta(&issue)
        {
            capabilities.extend(meta.capabilities);
            if meta.risk == "high" {
                plan_high.push(format!("#{i} is `risk: high` in the plan"));
            }
            task.get_or_insert(i);
        }
    }

    let mut matches = evaluate(&rules, &files, &new_deps, &capabilities);
    // The plan rates a task high on more than paths (D-016); the PR never rates lower.
    if !plan_high.is_empty() {
        matches.push(risk::Match {
            rule: "R-plan".into(),
            why: "the linked task is `risk: high` in the plan, so the owner decides".into(),
            approval: Approval::Owner,
            evidence: plan_high,
        });
    }
    // Built in: a weakening is R-weaken and the owner's, whatever the repo's rules say (a repo
    // rule with the `weaken` capability only adds its own match, made owner-class too).
    let mut findings = findings;
    for m in &mut matches {
        let weakens = (rules.rules.iter())
            .any(|r| r.id == m.rule && r.capability.as_deref() == Some(weaken::CAPABILITY));
        if weakens || m.rule == weaken::RULE {
            m.approval = Approval::Owner;
        }
        // A repo rule with the capability under another id also counts as R-weaken, so no
        // signature that keys on the id can pass it.
        if weakens && m.rule != weaken::RULE && findings.is_empty() {
            findings.extend(m.evidence.iter().map(|e| format!("{}: {e}", m.rule)));
        }
    }
    if !findings.is_empty() {
        matches.retain(|m| m.rule != weaken::RULE);
        matches.push(risk::Match {
            rule: weaken::RULE.into(),
            why: weaken::WHY.into(),
            approval: Approval::Owner,
            evidence: findings.clone(),
        });
    }
    let views: Vec<FileView> = pr_files
        .iter()
        .map(|f| FileView {
            path: f.filename.clone(),
            status: f.status.clone(),
            has_patch: f.patch.is_some(),
            changes: f.additions + f.deletions,
        })
        .collect();
    matches.extend(unseen_diff(&views, pull.changed_files));
    matches.extend(risk::bundle_gated(
        crate::cli::pr_bundle::closed_by_pr(gh, &pull, &app.ctx.repo.to_string())?.count(),
        &files,
        &rules,
    ));
    if let Some((_, evidence)) = revert_findings(gh, &rules, &pull, &pr_files)
        && !evidence.is_empty()
    {
        matches.push(risk::Match {
            rule: "R-revert".into(),
            why: "a revert of an owner-class PR, or not its exact inverse, needs the owner".into(),
            approval: Approval::Owner,
            evidence,
        });
    }
    // The critical floor: a delegate never approves the approval system, secrets or prod.
    let critical = if matches.iter().any(|m| m.approval == Approval::Delegate) {
        let bosun_code = gh.file_at(BOSUN_CODE_MARKER, base)?.is_some();
        risk::critical(&matches, &files, bosun_code)
    } else {
        Vec::new()
    };
    apply_floor(&mut matches, &critical);
    let level = risk::level(&matches);
    let high = level == "high";
    let out_of_touch = out_of_touch(app, task, &pr_files, pull.changed_files);

    let warning = out_of_touch.as_deref().and_then(touch_warning);
    if let Some(w) = &warning {
        eprintln!("bosun: risk-check: {w}");
    }
    // A warning only: the status description, the level and the state stay as they are. The
    // PR body carries it where it already carries a Risk note (high risk).
    let note = high.then(|| {
        let note = risk_note(&matches, head);
        match &warning {
            Some(w) => note.replacen(
                "## Risk: high\n\n",
                &format!("## Risk: high\n\n- **warning:** {w}\n"),
                1,
            ),
            None => note,
        }
    });
    let body = replace_note(&pull.body, note.as_deref());
    if body != pull.body {
        gh.update_pull_body(n, &body)?;
    }

    let owners = if high {
        owner_logins(gh, base)?
    } else {
        Vec::new()
    };
    // Key and owner changes need the owner's own GitHub approval, never a signature.
    let trust = trust_files_changed(
        files
            .iter()
            .map(|f| (f.path.as_str(), f.previous_path.as_deref())),
    );
    if !trust.is_empty() {
        eprintln!(
            "bosun: risk-check: the PR changes {}: signed approvals do not count",
            trust.join(", ")
        );
    }
    // An unseen diff may hide a trust file: no signature counts then either.
    let unseen = matches.iter().any(|m| m.rule == "R-unseen");
    if unseen {
        eprintln!(
            "bosun: risk-check: bosun could not see the whole diff: signed approvals do not count"
        );
    }
    let signed_ok = trust.is_empty() && !unseen;
    let requirement = requirement(&matches);
    let (mut bootstrap, policy_source) = if high && signed_ok {
        let bosun_code = gh.file_at(BOSUN_CODE_MARKER, base)?.is_some();
        let excluded = bootstrap_excluded(&pr_files, &matches, bosun_code);
        let today = app.ctx.now.to_string();
        let pr = (&pr_files[..], &matches[..], requirement);
        orchestrator_window(gh, base, &today[..10], pr, (&excluded, &capabilities))?
    } else {
        (Err("signed approvals do not count on this PR".into()), None)
    };
    let repo = app.ctx.repo.to_string();
    let linked = linked_issues_in(&pull.body, &repo);
    // D-017: the signature counts only with a clean verdict and a task the author leased.
    let auto = if policy_source.is_some() && bootstrap.is_ok() {
        autonomous_inputs(gh, &pull, &repo)?
    } else {
        None
    };
    if policy_source.is_some() && bootstrap.is_ok() && auto.is_none() {
        bootstrap =
            Err("autonomous mode (D-017) needs an open linked task leased by the author".into());
    }
    let autonomous = auto.is_some();
    let unlinked =
        linked.is_empty() && matches!(requirement, Approval::TwoVerdicts | Approval::Delegate);
    let models = review_models(gh, base)?;
    let need = Need {
        repo: &app.ctx.repo,
        pr: n,
        base_ref: &pull.base_ref,
        owners: &owners,
        signed_ok,
        bootstrap: bootstrap.as_deref().ok(),
        requirement,
        linked: &linked,
        unlinked,
        autonomous: auto.as_ref(),
        models: models.as_deref(),
    };
    let found = if high {
        approval_on(gh, &need, head)?
    } else {
        Found::default()
    };
    if let Some(why) = &found.delegate_blocked {
        eprintln!("bosun: risk-check: a delegate cannot approve: {why}");
    }
    let passed_on_head = !high || found.passed();
    // An older head's approval carries over a clean merge of main (design §3 C7), judged by `need`.
    let carried = if passed_on_head {
        None
    } else {
        default_branch(gh).ok().and_then(|branch| {
            let approve = |h: &str| approval_on(gh, &need, h).ok();
            carry_approval(gh, &pull, &branch, &owners, &approve)
        })
    };
    let Found {
        mut approved_by,
        mut verdicts,
        mut delegate_verdicts,
        delegate_blocked,
        autonomous_verdicts,
    } = found;
    let carried_from = carried.as_ref().map(|(h, ..)| h.clone());
    if let Some((_, who, counted)) = carried {
        approved_by = Some(who);
        // The sessions counted on h, shown as the head path shows them.
        match requirement {
            Approval::TwoVerdicts => verdicts = counted,
            Approval::Delegate => delegate_verdicts = counted,
            _ => {}
        }
    }
    let by_verdicts = verdicts.len() >= VERDICTS_NEEDED;
    let passed = passed_on_head || carried_from.is_some();

    let ids: Vec<&str> = matches.iter().map(|m| m.rule.as_str()).collect();
    let tag = |h: &String| format!(" (carried from {})", &h[..7]); // a full sha (`candidates`)
    let from: String = carried_from.iter().map(tag).collect();
    let description = match (&approved_by, high) {
        (_, false) => "normal risk".to_string(),
        (Some(who), _) => format!("high risk ({}), approved by {who}{from}", ids.join(", ")),
        (None, _) if by_verdicts => format!(
            "high risk ({}), approved by two verdicts: {}",
            ids.join(", "),
            verdicts.join(", ")
        ),
        (None, _) if autonomous => format!(
            "high risk, autonomous mode (D-017): clean verdict and orchestrator signature needed: {}",
            ids.join(", ")
        ),
        (None, _) if unlinked => format!(
            "high risk, owner approval needed (links no task, so verdicts cannot count): {}",
            ids.join(", ")
        ),
        (None, _) if !critical.is_empty() => format!(
            "high risk, owner approval needed (critical: {}): {}",
            critical.join(", "),
            ids.join(", ")
        ),
        (None, _) if requirement == Approval::Delegate => format!(
            "high risk, owner approval needed (or a delegate): {}",
            ids.join(", ")
        ),
        (None, _) if requirement == Approval::TwoVerdicts => format!(
            "high risk, two verdicts needed ({}/{VERDICTS_NEEDED}): {}",
            verdicts.len(),
            ids.join(", ")
        ),
        (None, _) => format!("high risk, owner approval needed: {}", ids.join(", ")),
    };
    let state = if passed { "success" } else { "failure" };
    let rules: Vec<String> = matches.iter().map(|m| format!("- {}", m.rule)).collect();
    let summary = format!("{description}\n\nMatched rules:\n{}", rules.join("\n"));
    w.post_full(head, CONTEXT, state, &description, &summary)?;
    // After the check is posted, and never an error: a doubt or a failed read is no carry.
    let carried = carry_status(gh, w, &pull, &linked);

    let mut data = json!({"pr": n, "head_sha": head, "level": level, "matches": matches,
        "approved_by": approved_by, "out_of_touch": out_of_touch,
        "verdict_carried_from": carried, "approval_carried_from": carried_from});
    if w.checks_skipped() {
        data["checks_skipped"] = json!(true);
    }
    if high {
        data["requirement"] = json!(requirement.name());
        if requirement == Approval::TwoVerdicts {
            data["verdicts"] = json!(verdicts);
        }
        if requirement == Approval::Delegate {
            data["delegate_verdicts"] = json!(delegate_verdicts);
            data["delegate_blocked"] = json!(delegate_blocked);
        }
        if !critical.is_empty() {
            data["critical"] = json!(critical);
        }
        if autonomous {
            data["policy"] = json!("autonomous");
            data["policy_source"] = json!(policy_source);
            data["autonomous_verdicts"] = json!(autonomous_verdicts);
        }
    }
    if passed {
        return Ok(Outcome::data(data));
    }
    let owner_fix = if owners.is_empty() {
        format!("fill {OWNERS_PATH} on the base branch, then an owner must approve head {head}")
    } else {
        format!(
            "an owner in {OWNERS_PATH} must approve head {head}: the session asks the owner with a form in the Claude Code chat and the orchestrator posts the signed comment (`bosun approve {n}@{head} --sign-key <pem>`); fallback: an owner's GitHub review (then comment `/bosun recheck` (or re-run the risk workflow)), or the owner's own PR comment `/bosun approve {head}`"
        )
    };
    // A bootstrap-mode signature that does not count says why (D-012).
    let boot = match &bootstrap {
        Err(why) if bootstrap_signed(gh, n, head)? => {
            eprintln!("bosun: risk-check: a bootstrap-mode approval does not count: {why}");
            format!("a bootstrap-mode approval (D-012) does not count: {why}; ")
        }
        _ => String::new(),
    };
    let verdict_class =
        autonomous || matches!(requirement, Approval::TwoVerdicts | Approval::Delegate);
    let model_note = match &models {
        Some(m) if verdict_class => format!(
            "; only verdicts by a model in `[approvals] review_models` ({}) count, a verdict with another or no model does not",
            m.join(", ")
        ),
        _ => String::new(),
    };
    let fix = if autonomous {
        format!(
            "get one independent clean verdict (approve, confidence >= {}, no major finding) on head {head} from a reviewer session (`bosun verdict comment --from -`), then the orchestrator signs `bosun approve {n}@{head} --sign-key <pem> --bootstrap`; or an owner approval: {owner_fix}",
            risk::VERDICT_MIN_CONFIDENCE
        )
    } else if unlinked {
        format!(
            "the PR links no task (`Closes #N`), so verdicts cannot be checked for independence: link the task, or get an owner approval: {owner_fix}"
        )
    } else if requirement == Approval::Delegate {
        format!(
            "get one independent review verdict (approve, confidence >= {}, no major finding) on head {head} and a delegate's signed approval (`bosun approve {n}@{head} --sign-key <pem> --as delegate`), or an owner approval: {owner_fix}",
            risk::VERDICT_MIN_CONFIDENCE
        )
    } else if requirement == Approval::TwoVerdicts {
        format!(
            "get a second review verdict from a different session (approve, confidence >= {}) on head {head}, or an owner approval: {owner_fix}",
            risk::VERDICT_MIN_CONFIDENCE
        )
    } else {
        owner_fix
    };
    Err(BosunError::needs_human(
        format!(
            "{boot}PR #{n} is high risk ({}) and not approved on head {head}",
            ids.join(", ")
        ),
        format!("{boot}{fix}{model_note}"),
    )
    .with_data(data))
}

/// The risk rules at `base` (the built-in defaults when it has none).
pub(crate) fn base_rules(gh: &dyn Github, base: &str) -> Result<Rules> {
    Ok(match gh.file_at(RULES_PATH, base)? {
        Some(text) => Rules::parse(&text).map_err(|e| {
            BosunError::error(
                format!("{RULES_PATH} at base {base}: {e}"),
                "fix the file on the base branch in a PR; a PR cannot change the rules that judge it",
            )
        })?,
        None => Rules::defaults(),
    })
}

fn patch_files(files: &[PrFile]) -> Vec<risk::PatchFile<'_>> {
    (files.iter())
        .map(|f| risk::PatchFile {
            path: &f.filename,
            previous_path: f.previous_filename.as_deref(),
            status: &f.status,
            patch: f.patch.as_deref(),
            changes: f.additions + f.deletions,
        })
        .collect()
}

/// Built-in R-revert (§8): `None` without a revert marker, else `(N, evidence)`; no evidence =
/// #N needed no owner and this is its exact inverse. A failed read is evidence, never an error.
pub(crate) fn revert_findings(
    gh: &dyn Github,
    rules: &Rules,
    pull: &Pull,
    pr_files: &[PrFile],
) -> Option<(u64, Vec<String>)> {
    let t = risk::revert_targets(&pull.body);
    let n = *t.first()?;
    let ev = if t.len() > 1 || n == pull.number {
        vec!["the marker names several PRs or this PR".into()]
    } else {
        revert_evidence(gh, rules, pull, pr_files, n)
            .unwrap_or_else(|e| vec![format!("cannot judge #{n}: {}", e.message)])
    };
    Some((n, ev))
}

fn revert_evidence(
    gh: &dyn Github,
    rules: &Rules,
    pull: &Pull,
    pr_files: &[PrFile],
    n: u64,
) -> Result<Vec<String>> {
    let Some(old) = gh.get_pull(n)?.filter(|p| p.merged) else {
        return Ok(vec![format!("#{n} does not exist or is not merged")]);
    };
    let old_files = gh.pull_files(n)?;
    let whole =
        pull.changed_files == pr_files.len() as u64 && old.changed_files == old_files.len() as u64;
    if !whole || old.base_ref != pull.base_ref {
        let why = "the file lists are not whole, or the bases differ";
        return Ok(vec![format!("#{n}: {why}")]);
    }
    let mut ev = Vec::new();
    let problems = risk::inverse_problems(&patch_files(&old_files), &patch_files(pr_files));
    if !problems.is_empty() {
        ev.push(format!(
            "not the exact inverse of #{n}: {}",
            problems.join("; ")
        ));
    }
    let changed: Vec<ChangedFile> = (old_files.iter())
        .map(|f| ChangedFile {
            path: f.filename.clone(),
            previous_path: f.previous_filename.clone(),
            added_lines: f.patch.as_deref().map(added_lines).unwrap_or_default(),
        })
        .collect();
    // #N's labels and links are editable after the merge: its own trusted risk-check outcome
    // counts instead, and only `normal risk` is no owner class (fail closed).
    let trusted = |s: &&crate::github::Status| {
        s.context == CONTEXT && s.creator == crate::core::approve::TRUSTED_CREATOR
    };
    let statuses = crate::github::gate::gate_statuses(gh, &old.head_sha)?; // newest first
    if statuses
        .iter()
        .find(trusted)
        .map(|s| s.description.as_str())
        != Some("normal risk")
    {
        ev.push(format!(
            "#{n} needed the owner (its risk-check was not normal risk)"
        ));
    }
    // The files under today's rules too; a changed manifest counts as a new dependency.
    let deps: Vec<String> = (old_files.iter())
        .filter(|f| is_manifest(&f.filename))
        .map(|f| f.filename.clone())
        .collect();
    let mut ms = evaluate(rules, &changed, &deps, &[]);
    if !weaken_findings(gh, &old_files, &old.base_sha, &old.head_sha)?.is_empty() {
        ms.push(risk::Match {
            rule: weaken::RULE.into(),
            why: weaken::WHY.into(),
            approval: Approval::Owner,
            evidence: Vec::new(),
        });
    }
    let bosun_code = gh.file_at(BOSUN_CODE_MARKER, &pull.base_sha)?.is_some();
    let critical = risk::critical(&ms, &changed, bosun_code);
    apply_floor(&mut ms, &critical);
    let owner = |m: &&risk::Match| {
        m.approval == Approval::Owner || ["R-weaken", "R-gate", "R-gov"].contains(&m.rule.as_str())
    };
    let mut ids: Vec<&str> = ms.iter().filter(owner).map(|m| m.rule.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    ev.extend(ids.iter().map(|id| format!("#{n} needed the owner ({id})")));
    Ok(ev)
}

/// What an approval is judged by: the checked head's own diff, the rules and owners at base.
struct Need<'a> {
    repo: &'a RepoId,
    pr: u64,
    base_ref: &'a str,
    owners: &'a [String],
    signed_ok: bool,
    /// The label under which an orchestrator signature counts as the owner (D-012, D-014): see
    /// `orchestrator_window`.
    bootstrap: Option<&'a str>,
    requirement: Approval,
    linked: &'a [u64],
    unlinked: bool,
    /// Autonomous policy (D-017), nothing excluded: the PR comments and the lease sessions the
    /// signature's clean verdict is judged by.
    autonomous: Option<&'a (Vec<Comment>, Vec<String>)>,
    /// `[approvals] review_models` at base: when set, only verdicts by these models count.
    models: Option<&'a [String]>,
}

/// How a head is approved: by an owner or delegate (`approved_by`), or by two verdicts.
#[derive(Default)]
pub(crate) struct Found {
    pub(crate) approved_by: Option<String>,
    pub(crate) verdicts: Vec<String>,
    delegate_verdicts: Vec<String>,
    delegate_blocked: Option<String>,
    autonomous_verdicts: Vec<String>,
}

impl Found {
    pub(crate) fn passed(&self) -> bool {
        self.approved_by.is_some() || self.verdicts.len() >= VERDICTS_NEEDED
    }

    /// The sessions whose verdicts count in this approval; `None` when an owner approved.
    pub(crate) fn counted(&self) -> Option<&[String]> {
        let delegate = (!self.delegate_verdicts.is_empty()).then_some(&self.delegate_verdicts[..]);
        if self.approved_by.is_none() {
            Some(&self.verdicts)
        } else {
            delegate
        }
    }
}

/// The approval of a high-risk PR on `sha`: an owner, two verdicts, or a delegate.
fn approval_on(gh: &dyn Github, need: &Need, sha: &str) -> Result<Found> {
    let Need {
        repo, pr, owners, ..
    } = *need;
    let mut found = Found::default();
    let mut bootstrap = need.bootstrap;
    if let Some((comments, excluded)) = need.autonomous {
        found.autonomous_verdicts = autonomous_clean(comments, sha, excluded, need.models);
        if found.autonomous_verdicts.is_empty() {
            bootstrap = None;
        }
    }
    // Under the autonomous policy only its own form counts, else only the bootstrap form.
    let form = if need.autonomous.is_some() {
        Form::Autonomous
    } else {
        Form::Bootstrap
    };
    let bootstrap = bootstrap.map(|l| (l, form));
    found.approved_by = approver(gh, repo, pr, sha, owners, need.signed_ok, bootstrap)?;
    let open = found.approved_by.is_none() && !need.unlinked;
    // Two verdicts only matter when the owner has not approved and the class allows them.
    if open && need.requirement == Approval::TwoVerdicts {
        let (comments, excluded) = verdict_inputs(gh, pr, need.linked)?;
        found.verdicts = two_verdicts(
            comments.iter().map(|c| (c.user.as_str(), c.body.as_str())),
            sha,
            &excluded,
            need.models,
        );
    }
    // A delegate decides only with one clean independent verdict on the head (spec §8).
    if open && need.requirement == Approval::Delegate {
        found.delegate_blocked = delegation_blocked(gh, need.base_ref, need.linked)?;
        if found.delegate_blocked.is_none() && need.signed_ok {
            let (comments, excluded) = verdict_inputs(gh, pr, need.linked)?;
            found.delegate_verdicts = clean_verdicts(
                comments.iter().map(|c| (c.user.as_str(), c.body.as_str())),
                sha,
                &excluded,
                need.models,
            );
            if !found.delegate_verdicts.is_empty() && !owners_withdrawn(gh, pr, sha, owners)? {
                let v1 = |f: Form| f == Form::V1;
                let key = signed_approver(gh, repo, pr, sha, &comments, Role::Delegate, &v1)?;
                found.approved_by = key.map(|id| format!("delegate {id}"));
            }
        }
    }
    Ok(found)
}

/// Carries `review-verdict` of an older head onto exactly `pull.head_sha` (design §3 C7), checking
/// its statuses before and after posting (verdict.yml is not serialized with us). Never an error.
fn carry_status(gh: &dyn Github, w: &GateWriter, pull: &Pull, linked: &[u64]) -> Option<String> {
    let head = pull.head_sha.as_str();
    // A carried status on top of a real verdict (a run was cancelled mid-post) is repaired first.
    let top = crate::github::gate::gate_statuses(gh, head)
        .ok()
        .and_then(|l| l.into_iter().find(is_verdict));
    if let Some(s) = top.filter(|s| s.description.starts_with(CARRIED)) {
        settle(gh, w, head, &s.description);
        return None;
    }
    let branch = default_branch(gh).ok()?;
    if candidates(gh, pull, &branch).is_empty() {
        return None;
    }
    let owners = owner_logins(gh, &pull.base_sha).ok()?;
    let h = carry_verdict(gh, pull, &branch, linked, &owners)?;
    let short: String = h.chars().take(7).collect();
    let text = format!("{CARRIED}{short}: clean merge of the default branch");
    if has_verdict(gh, head) {
        return None;
    }
    w.post(head, VERDICT, "success", &text).ok()?;
    settle(gh, w, head, &text).then_some(h)
}

const CARRIED: &str = "carried from ";
const UNSURE: &str = "carry withdrawn: could not confirm no verdict raced it";

/// True only when `ours` stands unchallenged on `head`. Another review-verdict is a real verdict
/// that raced us: the newest is posted again. If that fails, or the history is unreadable: withdraw.
fn settle(gh: &dyn Github, w: &GateWriter, head: &str, ours: &str) -> bool {
    let post = |state: &str, text: &str| w.post(head, VERDICT, state, text);
    let seen = crate::github::gate::gate_history(gh, head).map(|l| {
        l.into_iter()
            .find(|s| is_verdict(s) && s.description != ours && !s.description.starts_with(CARRIED))
    });
    match seen {
        Ok(None) => return true,
        Ok(Some(f)) if post(&f.state, &f.description).is_ok() => {}
        _ => _ = post("failure", UNSURE),
    }
    eprintln!("bosun: risk-check: a verdict raced the carry-over; it was not kept");
    false
}

/// "N files outside the task's lease: a, b" (ten names at most, then "and N more"); `None` when
/// there are none.
fn touch_warning(out: &[String]) -> Option<String> {
    if out.is_empty() {
        return None;
    }
    // File names are repo data in a PR body: quoted, and `plain`, so one cannot end the note.
    let quote = |n: &String| format!("`{}`", plain(n));
    let mut names = out
        .iter()
        .take(10)
        .map(quote)
        .collect::<Vec<_>>()
        .join(", ");
    if out.len() > 10 {
        names.push_str(&format!(" and {} more", out.len() - 10));
    }
    let n = out.len();
    Some(format!(
        "{n} file{} outside the task's lease: {names}",
        if n == 1 { "" } else { "s" }
    ))
}

/// The PR files outside the linked task's newest live lease (design C3), for a warning. `None`
/// when the PR links no task, the task has no live lease with a touch, the file list is cut or
/// the comments cannot be read. The task is the first linked issue with task metadata; humans'
/// PRs have no lease and so get `None`.
fn out_of_touch(
    app: &App,
    task: Option<u64>,
    pr_files: &[PrFile],
    changed_files: u64,
) -> Option<Vec<String>> {
    // GitHub lists at most 3000 files; a shorter list than it reports is cut.
    if (pr_files.len() as u64) < changed_files || pr_files.len() >= 3000 {
        return None;
    }
    let task = task?;
    // Only a warning: a failed read costs the warning, never the check.
    let notes = match trusted_notes(app.gh.as_ref(), task) {
        Ok(notes) => notes,
        Err(e) => {
            eprintln!(
                "bosun: risk-check: no touch warning, cannot read the comments of #{task}: {}",
                e.message
            );
            return None;
        }
    };
    let lease = leases_of(task, &notes)
        .into_iter()
        .filter(|l| l.is_live(app.ctx.now) && !l.touch.is_empty())
        .max_by_key(|l| l.comment_id)?;
    let changed: Vec<Changed> = pr_files
        .iter()
        .map(|f| Changed {
            path: f.filename.clone(),
            status: f.status.clone(),
            previous: f.previous_filename.clone(),
        })
        .collect();
    Some(outside(&changed, &lease.touch))
}

/// When a review was submitted; `None` when GitHub gave no time or it does not parse.
pub fn submitted(r: &Review) -> Option<Timestamp> {
    r.submitted_at.as_deref().and_then(parse_rfc3339)
}

/// `owner`'s newest approve, request-changes or dismissed review on `head`, by `submitted_at`
/// (ties by id), not by list position. If any of them has no usable time the order is unknown, so
/// the most withdrawing one is returned (the newest request for changes or dismissal, if any).
fn decisive<'a>(reviews: &'a [Review], owner: &str, head: &str) -> Option<&'a Review> {
    let mine: Vec<&Review> = (reviews.iter())
        .filter(|r| r.user.eq_ignore_ascii_case(owner) && r.commit_id == head)
        .filter(|r| {
            matches!(
                r.state.as_str(),
                "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
            )
        })
        .collect();
    if mine.iter().any(|r| submitted(r).is_none()) {
        let against = mine.iter().filter(|r| withdrawn(Some(&r.state)));
        return against
            .max_by_key(|r| r.id)
            .or(mine.iter().max_by_key(|r| r.id))
            .copied();
    }
    mine.into_iter().max_by_key(|r| (submitted(r), r.id))
}

/// The id of `owner`'s decisive review on `head` when it was dismissed.
pub fn dismissed_review(reviews: &[Review], owner: &str, head: &str) -> Option<u64> {
    decisive(reviews, owner, head)
        .filter(|r| r.state == "DISMISSED")
        .map(|r| r.id)
}

/// The state of `owner`'s newest approve, request-changes or dismissed review on `head`.
pub fn decisive_review(reviews: &[Review], owner: &str, head: &str) -> Option<String> {
    decisive(reviews, owner, head).map(|r| r.state.clone())
}

/// The owner who approved `head`: by a GitHub review (their newest decisive review on `head` is
/// an approval) or by an unedited PR comment `/bosun approve <head>`.
fn approver(
    gh: &dyn Github,
    repo: &RepoId,
    pr: u64,
    head: &str,
    owners: &[String],
    signed_ok: bool,
    bootstrap: Option<(&str, Form)>,
) -> Result<Option<String>> {
    let reviews = gh.list_reviews(pr)?;
    let state = |o: &str| decisive_review(&reviews, o, head);
    let by_review = owners
        .iter()
        .find(|o| state(o).as_deref() == Some("APPROVED"));
    if let Some(o) = by_review {
        return Ok(Some(o.clone()));
    }
    let comments = gh.list_comments(pr)?;
    // Only a comment written with a plain user token counts: agent sessions act as the same
    // login through an app, and GitHub marks that in `performed_via_github_app` (spec §8).
    // An edited comment does not count: users with write access can edit others' comments. An
    // owner whose newest decisive review on `head` requests changes, or was dismissed, has
    // withdrawn the approval; after a dismissal only a comment made later than the dismissal
    // (its time on the issue timeline, not the review's) clears it.
    let by_comment = owners.iter().find(|o| match state(o).as_deref() {
        Some("DISMISSED") => {
            let newer = newer_than(&comments, dismissed_at(gh, pr, &reviews, o, head));
            owner_comment(&newer, o, head).is_some()
        }
        s if withdrawn(s) => false,
        _ => owner_comment(&comments, o, head).is_some(),
    });
    if let Some(o) = by_comment {
        return Ok(Some(o.clone()));
    }
    // A signed approval stands for the owner: with no owners it counts for no one, and any
    // owner's request for changes withdraws it. A v1 signature carries no time, so a re-posted
    // old one could pass for new: it never clears a dismissal. Each dismissed owner review needs
    // an owner v2 signature naming its id, and the timeline must show the dismissal.
    let changes = |o: &String| state(o).as_deref() == Some("CHANGES_REQUESTED");
    if !signed_ok || owners.is_empty() || owners.iter().any(changes) {
        return Ok(None);
    }
    let sign = |accept: &dyn Fn(Form) -> bool| {
        signed_approver(gh, repo, pr, head, &comments, Role::Owner, accept)
    };
    let ids: Vec<u64> = (owners.iter())
        .filter_map(|o| dismissed_review(&reviews, o, head))
        .collect();
    // A bootstrap-mode signature never clears a dismissal.
    if ids.is_empty() {
        // Bootstrap and autonomous signatures are not owner approvals.
        if let Some(id) = sign(&|f| matches!(f, Form::V1 | Form::V2(_)))? {
            return Ok(Some(format!("key {id}")));
        }
        let Some((label, form)) = bootstrap else {
            return Ok(None);
        };
        let boot = sign(&|f| f == form)?;
        return Ok(boot.map(|id| format!("key {id} ({label})")));
    }
    let mut by = None;
    for id in ids {
        let seen = (gh.review_dismissed_at(pr, id))
            .map_err(|e| eprintln!("bosun: risk-check: cannot read the timeline: {}", e.message))
            .ok()
            .flatten();
        if id == 0 || seen.and_then(|t| parse_rfc3339(&t)).is_none() {
            return Ok(None);
        }
        let Some(key) = sign(&|f| f == Form::V2(id))? else {
            return Ok(None);
        };
        by.get_or_insert(key);
    }
    Ok(by.map(|id| format!("key {id}")))
}

/// A decisive review in this state withdraws the owner's approvals (anyone with write access can
/// dismiss a review; GitHub then reports DISMISSED).
pub fn withdrawn(state: Option<&str>) -> bool {
    matches!(state, Some("CHANGES_REQUESTED" | "DISMISSED"))
}

/// When `owner`'s decisive review was dismissed (issue timeline); `None` when unknown.
fn dismissed_at(
    gh: &dyn Github,
    pr: u64,
    reviews: &[Review],
    owner: &str,
    head: &str,
) -> Option<Timestamp> {
    let id = decisive(reviews, owner, head)?.id;
    parse_rfc3339(&gh.review_dismissed_at(pr, id).ok()??)
}

/// The comments created after `since`; with no time nothing is newer (fail closed).
fn newer_than(comments: &[Comment], since: Option<Timestamp>) -> Vec<Comment> {
    let Some(since) = since else {
        return Vec::new();
    };
    comments
        .iter()
        .filter(|c| parse_rfc3339(&c.created_at).is_some_and(|t| t > since))
        .cloned()
        .collect()
}

/// Why a delegate cannot approve now, if it cannot: bosun is halted (or cannot tell), the PR
/// targets another branch than the default one, or a linked issue is not an open bosun task
/// with a lease record (a decoy link would hide the real task's capabilities and its lease
/// holders). An unreadable task issue is a reason; an error listing its comments errors the
/// check (fail closed, as every other read here).
pub(crate) fn delegation_blocked(
    gh: &dyn Github,
    base_ref: &str,
    linked: &[u64],
) -> Result<Option<String>> {
    match halt_reason(gh) {
        Ok(None) => {}
        Ok(Some((why, _))) => return Ok(Some(format!("bosun is halted ({why})"))),
        Err(e) => {
            return Ok(Some(format!(
                "cannot tell whether bosun is halted: {}",
                e.message
            )));
        }
    }
    let branch = default_branch(gh)?;
    if base_ref != branch {
        return Ok(Some(format!(
            "the PR targets {base_ref}, not the default branch {branch}"
        )));
    }
    for i in linked {
        let issue = match gh.get_issue(*i) {
            Ok(Some(issue)) => issue,
            Ok(None) => return Ok(Some(format!("linked #{i} does not exist"))),
            Err(e) => return Ok(Some(format!("cannot read linked #{i}: {}", e.message))),
        };
        if issue.pull_request || issue.state != "open" || task_meta(&issue).is_none() {
            return Ok(Some(format!("linked #{i} is not an open bosun task")));
        }
        let notes = gh.list_comments(*i)?;
        if lease_sessions(notes.iter().map(|c| c.body.as_str())).is_empty() {
            return Ok(Some(format!("linked task #{i} has no lease record")));
        }
    }
    Ok(None)
}

/// Whether a signed approval of `head` cannot count: no owners, or an owner's newest decisive
/// review on `head` requests changes or was dismissed.
fn owners_withdrawn(gh: &dyn Github, pr: u64, head: &str, owners: &[String]) -> Result<bool> {
    let reviews = gh.list_reviews(pr)?;
    Ok(owners.is_empty()
        || owners
            .iter()
            .any(|o| withdrawn(decisive_review(&reviews, o, head).as_deref())))
}

/// The PR's comments oldest first, and the sessions that cannot review it independently: every
/// session with a lease record (past or live) on a linked issue. `bosun pr` records no session
/// for the PR author, so the lease is the only trace of who wrote the change.
pub fn verdict_inputs(gh: &dyn Github, n: u64, ls: &[u64]) -> Result<(Vec<Comment>, Vec<String>)> {
    let mut comments = gh.list_comments(n)?;
    comments.sort_by_key(|c| c.id);
    // In checks mode only the gate App's records count (UC-B2-4).
    for c in &mut comments {
        c.user = crate::github::gate::record_author(&c.user);
    }
    let mut excluded = Vec::new();
    for i in ls {
        let notes = gh.list_comments(*i)?;
        excluded.extend(lease_sessions(notes.iter().map(|c| c.body.as_str())));
    }
    Ok((comments, excluded))
}

/// D-017: the PR's comments and the sessions that cannot review it, only when the PR closes an
/// open task issue (not a PR, not itself, with task meta) on which the PR author wrote a lease
/// record; else no reviewer can be shown independent of the author (a decoy link fails closed).
/// The excluded sessions are every lease session on any linked issue.
pub(crate) fn autonomous_inputs(
    gh: &dyn Github,
    pull: &Pull,
    repo: &str,
) -> Result<Option<(Vec<Comment>, Vec<String>)>> {
    let mut task = false;
    for i in closing_issues_in(&pull.body, Some(repo)) {
        let open = |x: &Issue| x.state == "open" && !x.pull_request && task_meta(x).is_some();
        if i != pull.number && gh.get_issue(i)?.is_some_and(|x| open(&x)) {
            let notes = gh.list_comments(i)?;
            let by = |c: &&Comment| c.user.eq_ignore_ascii_case(&pull.user);
            let bodies = notes.iter().filter(by).map(|c| c.body.as_str());
            task |= !lease_sessions(bodies).is_empty();
        }
    }
    let linked = linked_issues_in(&pull.body, repo);
    let (comments, excluded) = verdict_inputs(gh, pull.number, &linked)?;
    Ok(task.then_some((comments, excluded)))
}

/// D-017: the sessions with a counting verdict on `head` ([`risk::autonomous_verdicts`]).
pub(crate) fn autonomous_clean(
    comments: &[Comment],
    head: &str,
    excluded: &[String],
    models: Option<&[String]>,
) -> Vec<String> {
    let c = comments.iter();
    risk::autonomous_verdicts(
        c.map(|c| (c.user.as_str(), c.body.as_str(), c.unedited())),
        head,
        excluded,
        models,
    )
}

/// `[approvals] review_models` on `base` (D-017 and the verdict classes): `None` when unset. A
/// base config that does not parse fails closed (an empty list: no verdict counts).
pub(crate) fn review_models(gh: &dyn Github, base: &str) -> Result<Option<Vec<String>>> {
    Ok(match gh.file_at(CONFIG_FILE, base)? {
        Some(t) => match Config::parse(&t, CONFIG_FILE) {
            Ok(c) => c.approvals.review_models,
            Err(_) => Some(Vec::new()),
        },
        None => None,
    })
}

/// The id of an enrolled key that signed an approval of `head` on this PR (spec §8, signed
/// approvals); `accept` takes the comment's form. Only an owner key signs v2 and bootstrap.
/// The keys come from the default branch only, so a PR cannot enroll its own. Any author and app
/// count: the signature is the proof. A missing or malformed key file, an unknown key or a bad
/// signature is no approval, logged to stderr, never an error.
fn signed_approver(
    gh: &dyn Github,
    repo: &RepoId,
    pr: u64,
    head: &str,
    comments: &[Comment],
    role: Role,
    accept: &dyn Fn(Form) -> bool,
) -> Result<Option<String>> {
    let sigs: Vec<(u64, Form, &str, &str)> = comments
        .iter()
        .flat_map(|c| {
            signature_lines(&c.body, head)
                .into_iter()
                .map(move |(form, id, sig)| (c.id, form, id, sig))
        })
        .collect();
    if sigs.is_empty() {
        return Ok(None);
    }
    let keys = match enrolled_keys(gh) {
        Ok(keys) => keys,
        Err(why) => {
            eprintln!("bosun: risk-check: signed approvals are off: {why}");
            return Ok(None);
        }
    };
    let (o, r) = (&repo.owner, &repo.name);
    for (comment, form, id, sig) in sigs {
        let msg = match (form, role) {
            (Form::V1, _) => signed_message(role, o, r, pr, head),
            (Form::V2(v), Role::Owner) => signed_message_v2(o, r, pr, head, v),
            (Form::Bootstrap, Role::Owner) => signed_message_bootstrap(o, r, pr, head),
            (Form::Autonomous, Role::Owner) => signed_message_autonomous(o, r, pr, head),
            _ => continue,
        };
        if !accept(form) {
            continue;
        }
        let Some(key) = keys.iter().find(|k| k.id == id) else {
            eprintln!("bosun: risk-check: comment {comment}: unknown key {id:?}, not counted");
            continue;
        };
        if key.role != role {
            eprintln!(
                "bosun: risk-check: comment {comment}: key {id:?} is a {} key, not counted as {}",
                key.role.name(),
                role.name()
            );
            continue;
        }
        let ok = b64::decode(&key.ed25519)
            .zip(b64::decode(sig))
            .is_some_and(|(k, s)| verify_ed25519(&k, msg.as_bytes(), &s));
        if ok {
            return Ok(Some(key.id.clone()));
        }
        eprintln!(
            "bosun: risk-check: comment {comment}: signature of key {id:?} does not verify for {msg:?}, not counted"
        );
    }
    Ok(None)
}

/// The label of an orchestrator approval (a bootstrap-form signature) when it can count on this
/// PR, else why not: `[approvals]` at base, read like the rules, decides with the window (D-012),
/// `policy` (D-014) and the PR's rules, exclusions and workflow files.
fn orchestrator_window(
    gh: &dyn Github,
    base: &str,
    today: &str,
    pr: (&[PrFile], &[risk::Match], Approval),
    (excluded, capabilities): (&[String], &[String]),
) -> Result<(std::result::Result<String, String>, Option<&'static str>)> {
    let (files, matches, requirement) = pr;
    let approvals = match gh.file_at(CONFIG_FILE, base)? {
        Some(t) => match Config::parse(&t, CONFIG_FILE) {
            Ok(c) => c.approvals,
            Err(e) => return Ok((Err(format!("bootstrap mode is off: {}", e.message)), None)),
        },
        None => Default::default(),
    };
    let paths: Vec<_> = (files.iter())
        .map(|f| (&*f.filename, f.previous_filename.as_deref()))
        .collect();
    let workflows = risk::workflow_files(&paths);
    let rules: Vec<&str> = matches.iter().map(|m| m.rule.as_str()).collect();
    let mut autonomous_excluded = risk::autonomous_excluded(&paths, matches);
    let patches: Vec<_> = (files.iter())
        .map(|f| (&*f.filename, f.patch.as_deref()))
        .collect();
    autonomous_excluded.extend(risk::path_attributes(&patches));
    let counts = orchestrator_counts(&OrchestratorCase {
        policy: approvals.policy(),
        bootstrap_until: approvals.bootstrap_until.as_deref(),
        today,
        requirement: requirement.name(),
        excluded,
        rules: &rules,
        workflows: &workflows,
        autonomous_excluded: &autonomous_excluded,
        capabilities,
    });
    let autonomous = approvals.policy() == Policy::Autonomous;
    Ok((counts, autonomous.then(|| approvals.policy_source())))
}

/// The paths and rules of the PR that a bootstrap-mode approval never covers.
fn bootstrap_excluded(files: &[PrFile], matches: &[risk::Match], bosun_code: bool) -> Vec<String> {
    let f: Vec<_> = (files.iter())
        .map(|f| {
            (
                &*f.filename,
                f.previous_filename.as_deref(),
                f.patch.as_deref(),
            )
        })
        .collect();
    risk::bootstrap_excluded(&f, matches, bosun_code)
}

/// Whether a comment on the PR has a bootstrap-mode signature line for `head`, valid or not.
fn bootstrap_signed(gh: &dyn Github, pr: u64, head: &str) -> Result<bool> {
    let comments = gh.list_comments(pr)?;
    Ok((comments.iter()).any(|c| {
        (signature_lines(&c.body, head).iter())
            .any(|(f, ..)| matches!(f, Form::Bootstrap | Form::Autonomous))
    }))
}

/// The keys of `.bosun/approvers.toml` on the default branch, read at `refs/heads/<branch>` so
/// a tag or another ref of the same name cannot stand in for it. `Err` says why signed
/// approvals are off: the file is missing, unreadable or malformed (a key that is not 32 bytes
/// of base64 makes the whole file malformed). Never an error of the check.
pub fn enrolled_keys(gh: &dyn Github) -> std::result::Result<Vec<ApproverKey>, String> {
    let git_ref = default_ref(gh).map_err(|e| format!("no default branch: {}", e.message))?;
    let text = match gh.file_at(APPROVERS_PATH, &git_ref) {
        Ok(Some(text)) => text,
        Ok(None) => return Err(format!("{APPROVERS_PATH} is missing on {git_ref}")),
        Err(e) => return Err(format!("cannot read {APPROVERS_PATH}: {}", e.message)),
    };
    let keys = parse_approvers(&text)
        .map_err(|e| format!("{APPROVERS_PATH} on {git_ref} is malformed: {e}"))?;
    let raw: Vec<Option<Vec<u8>>> = keys.iter().map(|k| b64::decode(&k.ed25519)).collect();
    if let Some(k) = keys
        .iter()
        .zip(&raw)
        .enumerate()
        .find(|(i, (_, r))| r.is_some() && raw[..*i].contains(r))
        .map(|(_, (k, _))| k)
    {
        return Err(format!(
            "{APPROVERS_PATH} on {git_ref} is malformed: the public key of {:?} is enrolled twice",
            k.id
        ));
    }
    match keys
        .iter()
        .find(|k| b64::decode(&k.ed25519).is_none_or(|b| b.len() != 32))
    {
        Some(k) => Err(format!(
            "{APPROVERS_PATH} on {git_ref} is malformed: key {:?} is not a base64 32-byte ed25519 key",
            k.id
        )),
        None => Ok(keys),
    }
}

/// The comment that approves `head` for `owner`: unedited, written with a plain user token and
/// exactly `/bosun approve <head>`.
pub fn owner_comment<'a>(comments: &'a [Comment], owner: &str, head: &str) -> Option<&'a Comment> {
    comments.iter().find(|c| {
        c.user.eq_ignore_ascii_case(owner)
            && c.unedited()
            && c.by_user_token()
            && comment_approves(&c.body, head)
    })
}
