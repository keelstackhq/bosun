//! CI-minute check (docs/ci-minutes.md): findings for one workflow file, from a small line-based
//! reader of the YAML subset GitHub workflows use here (no YAML crate; conservative). Pure: the
//! caller reads the files.
//!
//! Rules:
//! - `draft`: a `pull_request(_target)` workflow whose job has no draft skip in its job-level `if:`
//!   (the aggregate `ci-ok` / `bosun-ok` is exempt: see `aggregate`).
//! - `aggregate`: the required aggregate job `ci-ok` or `bosun-ok` skips drafts. A skipped
//!   required check counts as passed (a stale green), so it must run and fail on a draft.
//! - `ready_for_review`: such a workflow whose `types:` omits `ready_for_review` (no `types:` is
//!   the default set, which lacks it).
//! - `concurrency`: a `pull_request(_target)` or `push` workflow without a top-level
//!   `concurrency:` that cancels in progress.
//! - `paths`: a heavy job (macOS, Windows or a matrix) on a `pull_request(_target)` trigger that
//!   has no `paths` or `paths-ignore`.
//!
//! A line `# ci-minutes: allow <rule> <reason>` in the file switches off `concurrency` or `paths`
//! for that file (a release workflow that must never cancel itself). `draft` and
//! `ready_for_review` cannot be allowed.

/// One finding: a workflow file, the job (if the rule is about one), the rule and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub job: Option<String>,
    pub rule: &'static str,
    pub fix: String,
}

/// The `if:` that skips drafts and keeps every other event running.
pub const DRAFT_SKIP: &str =
    "github.event_name != 'pull_request' || github.event.pull_request.draft == false";

/// Entry of the reader: a line (comments and blanks dropped) and, for a block scalar
/// (`key: |` or `key: >-`), its body joined by spaces.
struct Line {
    indent: usize,
    text: String,
    body: String,
}

impl Line {
    /// The key of `key: value` or `- key: value`; a bare `- item` has none.
    fn key(&self) -> Option<&str> {
        let t = self.text.strip_prefix("- ").unwrap_or(&self.text);
        let end = t.find(": ").or_else(|| t.strip_suffix(':').map(str::len))?;
        Some(t[..end].trim().trim_matches(|c| c == '"' || c == '\''))
    }

    /// What follows `key:` on the line, plus a block scalar's body.
    fn value(&self) -> String {
        let t = self.text.strip_prefix("- ").unwrap_or(&self.text);
        let v = t.find(": ").map_or("", |i| t[i + 2..].trim());
        if self.body.is_empty() {
            v.to_string()
        } else {
            self.body.clone()
        }
    }

    /// A list item `- x` (or `- key: v`): the text after the dash.
    fn item(&self) -> Option<&str> {
        self.text.strip_prefix("- ").map(str::trim)
    }
}

fn is_block_header(v: &str) -> bool {
    let v = v.trim();
    matches!(v, "|" | ">" | "|-" | ">-" | "|+" | ">+")
}

fn lex(text: &str) -> Vec<Line> {
    let raw: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let l = raw[i];
        i += 1;
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = l.len() - l.trim_start().len();
        let mut line = Line {
            indent,
            text: t.to_string(),
            body: String::new(),
        };
        if is_block_header(&line.value_inline()) {
            let mut parts = Vec::new();
            while i < raw.len() {
                let n = raw[i];
                let nt = n.trim();
                if !nt.is_empty() && n.len() - n.trim_start().len() <= indent {
                    break;
                }
                if !nt.is_empty() {
                    parts.push(nt);
                }
                i += 1;
            }
            // An empty body stays distinguishable from "no block": use a marker space.
            line.body = if parts.is_empty() {
                " ".into()
            } else {
                parts.join(" ")
            };
        }
        out.push(line);
    }
    out
}

impl Line {
    fn value_inline(&self) -> String {
        let t = self.text.strip_prefix("- ").unwrap_or(&self.text);
        t.find(": ").map_or("", |i| t[i + 2..].trim()).to_string()
    }
}

/// The lines under `lines[at]`: those after it with a deeper indent.
fn children(lines: &[Line], at: usize) -> &[Line] {
    let base = lines[at].indent;
    let n = lines[at + 1..]
        .iter()
        .take_while(|l| l.indent > base)
        .count();
    &lines[at + 1..at + 1 + n]
}

/// Direct children of a section: the entries at the section's first child indent, with the index
/// into `sec` of each.
fn direct(sec: &[Line]) -> Vec<usize> {
    let Some(first) = sec.first() else {
        return Vec::new();
    };
    (0..sec.len())
        .filter(|&i| sec[i].indent == first.indent)
        .collect()
}

fn flow_list(v: &str) -> Vec<String> {
    let v = v.trim();
    let v = v
        .strip_prefix('[')
        .and_then(|x| x.split(']').next())
        .unwrap_or(v);
    v.split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// One trigger of the `on:` block with the properties the rules read.
#[derive(Default)]
struct Trigger {
    name: String,
    /// `None`: no `types:` key.
    types: Option<Vec<String>>,
    paths: bool,
}

fn triggers(lines: &[Line]) -> Vec<Trigger> {
    let Some(at) = lines
        .iter()
        .position(|l| l.indent == 0 && matches!(l.key(), Some("on" | "true")))
    else {
        return Vec::new();
    };
    let inline = lines[at].value();
    if !inline.is_empty() {
        return flow_list(&inline)
            .into_iter()
            .map(|name| Trigger {
                name,
                ..Trigger::default()
            })
            .collect();
    }
    let sec = children(lines, at);
    let mut out = Vec::new();
    for &i in &direct(sec) {
        let l = &sec[i];
        if let Some(item) = l.item().filter(|_| l.key().is_none()) {
            out.push(Trigger {
                name: item.to_string(),
                ..Trigger::default()
            });
            continue;
        }
        let Some(name) = l.key() else { continue };
        let mut t = Trigger {
            name: name.to_string(),
            ..Trigger::default()
        };
        let v = l.value();
        if v.starts_with('{') {
            // A flow mapping: `{ types: [a, b], paths: [...] }`.
            if let Some(p) = v.find("types:") {
                t.types = Some(flow_list(v[p + 6..].trim()));
            }
            t.paths = v.contains("paths:") || v.contains("paths-ignore:");
        }
        let sub = children(sec, i);
        for (j, s) in sub.iter().enumerate() {
            match s.key() {
                Some("types") if s.indent == sub[0].indent => {
                    let mut ty = flow_list(&s.value());
                    ty.extend(
                        children(sub, j)
                            .iter()
                            .filter_map(|c| c.item())
                            .map(|c| c.trim_matches(|x| x == '"' || x == '\'').to_string()),
                    );
                    t.types = Some(ty);
                }
                Some("paths" | "paths-ignore") if s.indent == sub[0].indent => t.paths = true,
                _ => {}
            }
        }
        out.push(t);
    }
    out
}

/// Whether the top-level `concurrency:` cancels runs in progress.
fn cancels(lines: &[Line]) -> bool {
    let Some(at) = lines
        .iter()
        .position(|l| l.indent == 0 && l.key() == Some("concurrency"))
    else {
        return false;
    };
    let rest = |v: &str| {
        let v = v.trim().trim_end_matches('}').trim();
        !v.is_empty() && v != "false"
    };
    let inline = lines[at].value();
    if let Some(p) = inline.find("cancel-in-progress:") {
        let v = &inline[p + "cancel-in-progress:".len()..];
        return rest(v.split(',').next().unwrap_or(""));
    }
    children(lines, at)
        .iter()
        .find(|c| c.key() == Some("cancel-in-progress"))
        .is_some_and(|c| rest(&c.value()))
}

struct Job {
    id: String,
    skips_drafts: bool,
    heavy: bool,
}

impl Job {
    /// The required aggregate check: it runs on a draft and fails there.
    fn aggregate(&self) -> bool {
        matches!(self.id.as_str(), "ci-ok" | "bosun-ok")
    }
}

fn jobs(lines: &[Line]) -> Vec<Job> {
    let Some(at) = lines
        .iter()
        .position(|l| l.indent == 0 && l.key() == Some("jobs"))
    else {
        return Vec::new();
    };
    let sec = children(lines, at);
    let mut out = Vec::new();
    for &i in &direct(sec) {
        let Some(id) = sec[i].key() else { continue };
        let body = children(sec, i);
        let Some(first) = body.first() else { continue };
        let props: Vec<usize> = (0..body.len())
            .filter(|&j| body[j].indent == first.indent)
            .collect();
        let mut job = Job {
            id: id.to_string(),
            skips_drafts: false,
            heavy: false,
        };
        for &j in &props {
            let value = body[j].value();
            match body[j].key() {
                Some("if") => {
                    let flat: String = value.chars().filter(|c| !c.is_whitespace()).collect();
                    job.skips_drafts = flat.contains("pull_request.draft==false")
                        || flat.contains("!github.event.pull_request.draft");
                }
                Some("runs-on") => {
                    let mut all = value;
                    for c in children(body, j) {
                        all.push(' ');
                        all.push_str(&c.text);
                    }
                    let all = all.to_ascii_lowercase();
                    job.heavy |= all.contains("macos") || all.contains("windows");
                }
                Some("strategy") => {
                    job.heavy |= children(body, j).iter().any(|c| c.key() == Some("matrix"));
                }
                _ => {}
            }
        }
        out.push(job);
    }
    out
}

/// A `# ci-minutes: allow <rule> <reason>` comment (with a reason) for `rule`.
fn allowed(text: &str, rule: &str) -> bool {
    text.lines().any(|l| {
        l.trim()
            .strip_prefix("# ci-minutes: allow ")
            .and_then(|r| r.strip_prefix(rule))
            .is_some_and(|r| !r.trim().is_empty())
    })
}

/// Findings for one workflow file (`file` is only a label), in file order.
pub fn check(file: &str, text: &str) -> Vec<Finding> {
    let lines = lex(text);
    let trig = triggers(&lines);
    let is_pr = |t: &&Trigger| matches!(t.name.as_str(), "pull_request" | "pull_request_target");
    let prs: Vec<&Trigger> = trig.iter().filter(is_pr).collect();
    let mut out = Vec::new();
    let mut add = |job: Option<&str>, rule: &'static str, fix: String| {
        out.push(Finding {
            file: file.to_string(),
            job: job.map(String::from),
            rule,
            fix,
        });
    };
    let jobs = jobs(&lines);
    if !prs.is_empty() {
        for j in jobs.iter().filter(|j| !j.skips_drafts && !j.aggregate()) {
            add(
                Some(&j.id),
                "draft",
                format!(
                    "add `if: {DRAFT_SKIP}` to job `{}` (with `&&` to its other conditions) so a draft PR starts no runner",
                    j.id
                ),
            );
        }
        for j in jobs.iter().filter(|j| j.skips_drafts && j.aggregate()) {
            add(
                Some(&j.id),
                "aggregate",
                format!(
                    "job `{}` is a required check: remove the draft skip from its `if:` (keep `always()`) and make its first step fail on a draft, because a skipped required check counts as passed",
                    j.id
                ),
            );
        }
        for t in &prs {
            if !t
                .types
                .as_ref()
                .is_some_and(|ty| ty.iter().any(|x| x == "ready_for_review"))
            {
                add(
                    None,
                    "ready_for_review",
                    format!(
                        "add `ready_for_review` to `types:` of {} (a skipped draft needs a run when it becomes ready; without `types:` the default set lacks it)",
                        t.name
                    ),
                );
            }
        }
        if !prs.iter().all(|t| t.paths) && !allowed(text, "paths") {
            for j in jobs.iter().filter(|j| j.heavy) {
                add(
                    Some(&j.id),
                    "paths",
                    format!(
                        "job `{}` runs on macOS, Windows or a matrix: add `paths:` or `paths-ignore:` to the pull_request trigger, or move the job to a release workflow",
                        j.id
                    ),
                );
            }
        }
    }
    if trig.iter().any(|t| {
        matches!(
            t.name.as_str(),
            "pull_request" | "pull_request_target" | "push"
        )
    }) && !cancels(&lines)
        && !allowed(text, "concurrency")
    {
        add(
            None,
            "concurrency",
            "add a top-level `concurrency:` with a `group` and `cancel-in-progress: true` (or an expression) so a new push cancels the older run".into(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = "name: ci
on:
  pull_request:
    branches: [main]
    types: [opened, synchronize, ready_for_review]
  push:
    branches: [main]
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}
jobs:
  ci:
    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
";

    fn rules(text: &str) -> Vec<(Option<String>, &'static str)> {
        check("w.yml", text)
            .into_iter()
            .map(|f| (f.job, f.rule))
            .collect()
    }

    #[test]
    fn a_clean_workflow_has_no_findings() {
        assert_eq!(rules(CLEAN), vec![]);
    }

    #[test]
    fn a_job_without_a_draft_skip_is_found_by_name() {
        let t = CLEAN.replace(
            "    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false\n",
            "",
        );
        let f = check("w.yml", &t);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!((f[0].job.as_deref(), f[0].rule), (Some("ci"), "draft"));
        assert_eq!(f[0].file, "w.yml");
        assert!(f[0].fix.contains(DRAFT_SKIP), "{}", f[0].fix);
    }

    #[test]
    fn every_job_needs_the_skip_and_a_folded_if_counts() {
        let t = format!(
            "{CLEAN}  gates:\n    needs: [ci]\n    if: >-\n      always() &&\n      (github.event_name != 'pull_request' ||\n      github.event.pull_request.draft == false)\n    runs-on: ubuntu-latest\n  other:\n    runs-on: ubuntu-latest\n"
        );
        assert_eq!(rules(&t), vec![(Some("other".into()), "draft")]);
        let neg = CLEAN.replace(
            "github.event_name != 'pull_request' || github.event.pull_request.draft == false",
            "!github.event.pull_request.draft",
        );
        assert_eq!(rules(&neg), vec![]);
    }

    #[test]
    fn the_aggregate_check_must_run_on_drafts() {
        let ok = format!(
            "{CLEAN}  ci-ok:\n    needs: [ci]\n    if: always()\n    runs-on: ubuntu-latest\n"
        );
        assert_eq!(rules(&ok), vec![], "no draft skip needed, none allowed");
        let bare = format!("{CLEAN}  bosun-ok:\n    needs: [ci]\n    runs-on: ubuntu-latest\n");
        assert_eq!(rules(&bare), vec![], "no if at all runs on a draft");
        let skipped = format!(
            "{CLEAN}  ci-ok:\n    needs: [ci]\n    if: ${{{{ always() && github.event.pull_request.draft == false }}}}\n    runs-on: ubuntu-latest\n"
        );
        assert_eq!(rules(&skipped), vec![(Some("ci-ok".into()), "aggregate")]);
    }

    #[test]
    fn a_run_script_or_comment_is_not_read_as_workflow_structure() {
        let t = format!(
            "{CLEAN}      - run: |\n          concurrency:\n          other:\n            if: x\n# jobs:\n"
        );
        assert_eq!(rules(&t), vec![]);
    }

    #[test]
    fn types_must_list_ready_for_review() {
        let t = CLEAN.replace(", ready_for_review", "");
        assert_eq!(rules(&t), vec![(None, "ready_for_review")]);
        let none = CLEAN.replace("    types: [opened, synchronize, ready_for_review]\n", "");
        assert_eq!(rules(&none), vec![(None, "ready_for_review")]);
        let block = CLEAN.replace(
            "    types: [opened, synchronize, ready_for_review]\n",
            "    types:\n      - opened\n      - ready_for_review\n",
        );
        assert_eq!(rules(&block), vec![]);
    }

    #[test]
    fn pull_request_target_is_checked_the_same() {
        let t = CLEAN.replace("  pull_request:", "  pull_request_target:");
        assert_eq!(rules(&t), vec![]);
        let bad = t.replace(", ready_for_review", "");
        assert_eq!(rules(&bad), vec![(None, "ready_for_review")]);
    }

    #[test]
    fn concurrency_must_cancel() {
        let none = CLEAN.replace(
            "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
            "",
        );
        assert_eq!(rules(&none), vec![(None, "concurrency")]);
        let off = CLEAN.replace(
            "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
            "cancel-in-progress: false",
        );
        assert_eq!(rules(&off), vec![(None, "concurrency")]);
        let missing = CLEAN
            .replace("concurrency:\n", "")
            .replace("  group: ci-${{ github.ref }}\n", "")
            .replace(
                "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
                "",
            );
        assert_eq!(rules(&missing), vec![(None, "concurrency")]);
        let flow = CLEAN.replace(
            "concurrency:\n  group: ci-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
            "concurrency: { group: g, cancel-in-progress: true }\n",
        );
        assert_eq!(rules(&flow), vec![]);
        let plain = CLEAN.replace(
            "concurrency:\n  group: ci-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
            "concurrency: just-a-group\n",
        );
        assert_eq!(rules(&plain), vec![(None, "concurrency")]);
    }

    #[test]
    fn an_allow_comment_with_a_reason_switches_off_concurrency_and_paths_only() {
        let off = CLEAN.replace(
            "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
            "cancel-in-progress: false",
        );
        let ok = format!("# ci-minutes: allow concurrency a release never cancels itself\n{off}");
        assert_eq!(rules(&ok), vec![]);
        let bare = format!("# ci-minutes: allow concurrency\n{off}");
        assert_eq!(
            rules(&bare),
            vec![(None, "concurrency")],
            "a reason is required"
        );
        let draft = CLEAN.replace(
            "    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false\n",
            "",
        );
        let t = format!("# ci-minutes: allow draft because\n{draft}");
        assert_eq!(rules(&t), vec![(Some("ci".into()), "draft")]);
    }

    #[test]
    fn workflows_without_pr_or_push_triggers_are_exempt() {
        for on in [
            "on:\n  schedule: [{ cron: \"1 2 * * 1\" }]\n  workflow_dispatch:\n",
            "on:\n  issue_comment:\n    types: [created]\n",
            "on:\n  workflow_run:\n    workflows: [ci]\n    types: [completed]\n",
            "on: workflow_call\n",
            "on: status\n",
            "on: [workflow_dispatch, schedule]\n",
        ] {
            let t = format!(
                "name: x\n{on}jobs:\n  a:\n    runs-on: macos-latest\n    strategy:\n      matrix:\n        v: [1, 2]\n"
            );
            assert_eq!(rules(&t), vec![], "{on}");
        }
    }

    #[test]
    fn a_heavy_job_on_a_pull_request_needs_paths() {
        let heavy = |runs_on: &str, trigger_extra: &str| {
            format!(
                "on:\n  pull_request:\n{trigger_extra}    types: [ready_for_review]\nconcurrency:\n  group: g\n  cancel-in-progress: true\njobs:\n  b:\n    if: github.event.pull_request.draft == false\n    runs-on: {runs_on}\n"
            )
        };
        for r in ["macos-14", "windows-latest", "[self-hosted, macOS]"] {
            assert_eq!(
                rules(&heavy(r, "")),
                vec![(Some("b".into()), "paths")],
                "{r}"
            );
        }
        assert_eq!(rules(&heavy("ubuntu-latest", "")), vec![]);
        assert_eq!(rules(&heavy("macos-14", "    paths: ['src/**']\n")), vec![]);
        assert_eq!(
            rules(&heavy("macos-14", "    paths-ignore:\n      - '**.md'\n")),
            vec![]
        );
        let matrix = heavy("${{ matrix.os }}", "")
            + "    strategy:\n      matrix:\n        os: [ubuntu-latest]\n";
        assert_eq!(rules(&matrix), vec![(Some("b".into()), "paths")]);
    }

    #[test]
    fn a_heavy_job_on_push_only_is_not_a_paths_finding() {
        let t = "on:\n  push:\n    branches: [main]\nconcurrency:\n  group: g\n  cancel-in-progress: true\njobs:\n  b:\n    runs-on: windows-latest\n";
        assert_eq!(rules(t), vec![]);
    }
}
