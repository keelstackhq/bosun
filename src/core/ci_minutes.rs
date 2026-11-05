//! CI-minute check (docs/ci-minutes.md): findings for one workflow file, from a small line-based
//! reader of the YAML subset GitHub workflows use here (no YAML crate; conservative). Pure: the
//! caller reads the files.
//!
//! Rules:
//! - `draft`: a `pull_request(_target)` workflow whose job has no draft skip in its job-level `if:`
//!   (the aggregate `ci-ok` / `bosun-ok` is exempt: see `aggregate`). A job confined to other
//!   events (`github.event_name == 'push'`) needs no guard, nor does a job that `needs:` a
//!   draft-skipping job (without a status function): it is skipped with it. A `pull_request`
//!   trigger whose `types:` has none of opened, synchronize, reopened, ready_for_review, edited
//!   never starts a draft's run, so no rule reads it.
//! - `aggregate`: the required aggregate job `ci-ok` or `bosun-ok` skips drafts, or has `needs:`
//!   and an `if:` without `always()`, or has `needs:` and no step that fails on a draft: a step
//!   that reads `pull_request.draft` and exits non-zero, or a `test` that a `needs.*.result` is
//!   `success`. A skipped required check counts as passed (a stale green), so it must run and
//!   fail on a draft.
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

/// The `if:` that skips drafts in a `pull_request_target` workflow, where the event name is
/// never `pull_request`.
pub const DRAFT_SKIP_TARGET: &str = "github.event.pull_request.draft == false";

/// `expr` without whitespace and without a `${{ }}` wrapper.
fn squeeze(expr: &str) -> String {
    let flat: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    match flat.strip_prefix("${{").and_then(|f| f.strip_suffix("}}")) {
        Some(inner) => inner.to_string(),
        None => flat,
    }
}

/// `expr` without the parentheses that wrap all of it.
fn unwrap_parens(mut expr: &str) -> &str {
    while let Some(inner) = expr.strip_prefix('(').and_then(|e| e.strip_suffix(')')) {
        let mut depth = 0i32;
        // Only when the first `(` closes at the very end: `(a)&&(b)` is not wrapped.
        for c in inner.chars() {
            depth += match c {
                '(' => 1,
                ')' => -1,
                _ => 0,
            };
            if depth < 0 {
                return expr;
            }
        }
        expr = inner;
    }
    expr
}

/// Split `expr` at the top-level (outside parentheses and quotes) occurrences of `op`.
fn split_top<'a>(expr: &'a str, op: &str) -> Vec<&'a str> {
    let (mut depth, mut quote, mut start) = (0i32, false, 0);
    let mut parts = Vec::new();
    let mut i = 0;
    while i < expr.len() {
        let rest = &expr[i..];
        let c = rest.chars().next().unwrap_or(' ');
        match c {
            '\'' => quote = !quote,
            '(' if !quote => depth += 1,
            ')' if !quote => depth -= 1,
            _ if !quote && depth == 0 && rest.starts_with(op) => {
                parts.push(&expr[start..i]);
                i += op.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    parts.push(&expr[start..]);
    parts
}

/// Whether a job `if:` skips drafts: it is a conjunction with the draft test as one term. A
/// top-level `||` breaks it (`always() || ...draft == false` never skips), except the guard's
/// own `event_name != 'pull_request' || ...draft == false` form, which only skips outside
/// `pull_request_target` (there the event name is never `pull_request`).
fn skips_draft(cond: &str, target: bool) -> bool {
    let plain = squeeze(DRAFT_SKIP_TARGET);
    let event = squeeze(DRAFT_SKIP);
    let expr = squeeze(cond);
    let expr = unwrap_parens(&expr);
    let guard = |term: &str| {
        let term = unwrap_parens(term);
        let not_true = plain.replace("==false", "!=true");
        let event_not_true = event.replace("==false", "!=true");
        term == plain
            || term == not_true
            || term == "!github.event.pull_request.draft"
            || (!target && (term == event || term == event_not_true))
    };
    // A label event's own branch (`github.event.label.name == '...'`): one named label may judge
    // a draft; such events are rare and cheap.
    let label = |term: &str| {
        unwrap_parens(term)
            .strip_prefix("github.event.label.name==")
            .is_some_and(|v| v.len() > 2 && v.starts_with('\'') && v.ends_with('\''))
    };
    // A term that confines the job to an event other than a pull request.
    let other_event = |term: &str| {
        let term = unwrap_parens(term);
        term.strip_prefix("github.event_name==")
            .is_some_and(|e| !matches!(e, "'pull_request'" | "'pull_request_target'"))
    };
    let conj = |part: &str, f: &dyn Fn(&str) -> bool| {
        split_top(unwrap_parens(part), "&&").into_iter().any(f)
    };
    let ors = split_top(expr, "||");
    if ors.len() > 1 {
        // The guard's own form, or every branch skips drafts or is another event's.
        return guard(expr)
            || ors
                .iter()
                .all(|p| conj(p, &guard) || conj(p, &other_event) || conj(p, &label));
    }
    conj(expr, &|t| guard(t) || other_event(t))
}

/// A YAML quoted scalar without its quotes; other text as it is.
fn unquote(v: &str) -> String {
    let v = v.trim();
    let inner = |q: char| v.strip_prefix(q).and_then(|x| x.strip_suffix(q));
    if v.len() >= 2 {
        if let Some(x) = inner('\'') {
            return x.replace("''", "'");
        }
        if let Some(x) = inner('"') {
            return x.replace("\\\"", "\"");
        }
    }
    v.to_string()
}

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
    /// The job-level `if:`, if any.
    cond: Option<String>,
    needs: bool,
    /// The ids in `needs:` (empty when `needs:` is absent or unreadable).
    needs_ids: Vec<String>,
    heavy: bool,
    /// The text of the job's whole body (steps included).
    body: String,
    /// Its first step reads `pull_request.draft` and exits non-zero: a fail-fast check of any
    /// name, which must run on a draft and not be skipped.
    fails_fast: bool,
}

impl Job {
    /// The required aggregate check: it runs on a draft and fails there.
    fn aggregate(&self) -> bool {
        matches!(self.id.as_str(), "ci-ok" | "bosun-ok")
    }
}

/// Read the properties of a job (or of an anchor merged into one) into `job`.
fn apply_props(job: &mut Job, body: &[Line], props: &[usize]) {
    for &j in props {
        let value = body[j].value();
        match body[j].key() {
            Some("if") => job.cond = Some(unquote(&value)),
            Some("needs") => {
                job.needs = true;
                job.needs_ids = flow_list(&value);
                job.needs_ids.extend(
                    children(body, j)
                        .iter()
                        .filter_map(|c| c.item())
                        .map(|c| c.trim_matches(|x| x == '"' || x == '\'').to_string()),
                );
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
            cond: None,
            needs: false,
            needs_ids: Vec::new(),
            heavy: false,
            body: body
                .iter()
                .map(|l| format!("{} {}", l.text, l.body))
                .collect::<Vec<_>>()
                .join("\n"),
            fails_fast: first_step_fails_on_draft(body, &props),
        };
        apply_props(&mut job, body, &props);
        // A merge key (`<<: *anchor`) brings the anchor's properties; the job's own win.
        for &j in &props {
            if body[j].key() != Some("<<") {
                continue;
            }
            let value = body[j].value();
            let Some(name) = value.strip_prefix('*') else {
                continue;
            };
            let marker = format!("&{}", name.trim());
            let anchor = lines.iter().position(|l| {
                l.key().is_some() && l.value_inline().split_whitespace().any(|w| w == marker)
            });
            let Some(a) = anchor else { continue };
            let abody = children(lines, a);
            let Some(first) = abody.first() else { continue };
            let aprops: Vec<usize> = (0..abody.len())
                .filter(|&k| abody[k].indent == first.indent)
                .collect();
            let mut from = Job {
                id: String::new(),
                cond: None,
                needs: false,
                needs_ids: Vec::new(),
                heavy: false,
                body: String::new(),
                fails_fast: false,
            };
            apply_props(&mut from, abody, &aprops);
            job.cond = job.cond.take().or(from.cond);
            if !job.needs {
                (job.needs, job.needs_ids) = (from.needs, from.needs_ids);
            }
            job.heavy |= from.heavy;
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

/// Whether an aggregate job's text fails when its needed jobs were skipped on a draft: a step
/// that reads `pull_request.draft` and exits non-zero, or a check that a `needs.*.result` is
/// `success` (a skipped need is not).
fn fails_on_draft(body: &str) -> bool {
    let flat: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let exits = ["exit1", "exit2", "exit3"].iter().any(|e| flat.contains(e));
    let draft_exit = flat.contains("pull_request.draft") && exits;
    let success = flat.contains("needs.")
        && [r#"="success""#, "=='success'", "=success", r#"=="success""#]
            .iter()
            .any(|p| flat.contains(p));
    draft_exit || success
}

/// Whether the job's first step (under `steps:`) reads `pull_request.draft` and exits non-zero.
fn first_step_fails_on_draft(body: &[Line], props: &[usize]) -> bool {
    let Some(&s) = props.iter().find(|&&j| body[j].key() == Some("steps")) else {
        return false;
    };
    let steps = children(body, s);
    let Some(first) = steps.first() else {
        return false;
    };
    let text: String = steps
        .iter()
        .enumerate()
        .take_while(|(k, l)| *k == 0 || l.indent > first.indent)
        .map(|(_, l)| format!("{}{}", l.text, l.body))
        .collect();
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    flat.contains("pull_request.draft")
        && ["exit1", "exit2", "exit3"].iter().any(|e| flat.contains(e))
}

/// Findings for one workflow file (`file` is only a label), in file order.
pub fn check(file: &str, text: &str) -> Vec<Finding> {
    let lines = lex(text);
    let trig = triggers(&lines);
    let is_pr = |t: &&Trigger| matches!(t.name.as_str(), "pull_request" | "pull_request_target");
    // Types that never start a run of a draft (closed, labeled, ...) leave nothing to guard.
    let live = |t: &&Trigger| {
        t.types.as_ref().is_none_or(|ty| {
            ty.is_empty()
                || ty.iter().any(|x| {
                    matches!(
                        x.as_str(),
                        "opened" | "synchronize" | "reopened" | "ready_for_review" | "edited"
                    )
                })
        })
    };
    let prs: Vec<&Trigger> = trig.iter().filter(is_pr).filter(live).collect();
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
        let target = prs.iter().any(|t| t.name == "pull_request_target");
        let skip = if target {
            DRAFT_SKIP_TARGET
        } else {
            DRAFT_SKIP
        };
        let skips = |j: &Job| j.cond.as_deref().is_some_and(|c| skips_draft(c, target));
        // A job that needs a skipped job is skipped with it, unless a status function in its
        // `if:` runs it anyway.
        let mut skipped: Vec<&str> = jobs
            .iter()
            .filter(|j| skips(j))
            .map(|j| j.id.as_str())
            .collect();
        loop {
            let more: Vec<&str> = jobs
                .iter()
                .filter(|j| !skipped.contains(&j.id.as_str()))
                .filter(|j| {
                    let cond = j.cond.as_deref().unwrap_or_default();
                    let status = ["always()", "cancelled()", "failure()"]
                        .iter()
                        .any(|f| cond.contains(f));
                    !status && j.needs_ids.iter().any(|n| skipped.contains(&n.as_str()))
                })
                .map(|j| j.id.as_str())
                .collect();
            if more.is_empty() {
                break;
            }
            skipped.extend(more);
        }
        for j in jobs
            .iter()
            .filter(|j| !skipped.contains(&j.id.as_str()) && !j.aggregate() && !j.fails_fast)
        {
            add(
                Some(&j.id),
                "draft",
                format!(
                    "add `if: {skip}` to job `{}` (with `&&` to its other conditions) so a draft PR starts no runner; never on a required check: a skipped required check counts as passed; make it fail fast on a draft instead",
                    j.id
                ),
            );
        }
        for j in jobs.iter().filter(|j| j.aggregate()) {
            let cond = j.cond.as_deref().unwrap_or_default();
            if skips_draft(cond, false) {
                add(
                    Some(&j.id),
                    "aggregate",
                    format!(
                        "job `{}` is a required check: remove the draft skip from its `if:` (keep `always()`) and make its first step fail on a draft, because a skipped required check counts as passed",
                        j.id
                    ),
                );
            } else if j.needs && !cond.contains("always()") {
                add(
                    Some(&j.id),
                    "aggregate",
                    format!(
                        "job `{}` is a required check with `needs:`: set `if: always()`, because it is skipped when a needed job is skipped and a skipped required check counts as passed",
                        j.id
                    ),
                );
            } else if j.needs && !fails_on_draft(&j.body) {
                add(
                    Some(&j.id),
                    "aggregate",
                    format!(
                        "job `{}` is a required check with `needs:` but no step fails on a draft: add a step that exits 1 when `github.event.pull_request.draft` is true, or `test \"${{{{ needs.<job>.result }}}}\" = \"success\"`, because a needed job skipped on a draft must not leave this check green",
                        j.id
                    ),
                );
            }
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
            "{CLEAN}  ci-ok:\n    needs: [ci]\n    if: always()\n    runs-on: ubuntu-latest\n    steps:\n      - run: test \"${{{{ needs.ci.result }}}}\" = \"success\"\n"
        );
        assert_eq!(rules(&ok), vec![], "no draft skip needed, none allowed");
        let bare = format!("{CLEAN}  bosun-ok:\n    needs: [ci]\n    runs-on: ubuntu-latest\n");
        assert_eq!(
            rules(&bare),
            vec![(Some("bosun-ok".into()), "aggregate")],
            "a skipped need skips a job whose if lacks always()"
        );
        let alone = format!("{CLEAN}  ci-ok:\n    runs-on: ubuntu-latest\n");
        assert_eq!(rules(&alone), vec![], "no needs, nothing to skip it");
        let other = format!(
            "{CLEAN}  ci-ok:\n    needs: [ci]\n    if: github.event_name == 'push'\n    runs-on: ubuntu-latest\n"
        );
        assert_eq!(rules(&other), vec![(Some("ci-ok".into()), "aggregate")]);
        let skipped = format!(
            "{CLEAN}  ci-ok:\n    needs: [ci]\n    if: ${{{{ always() && github.event.pull_request.draft == false }}}}\n    runs-on: ubuntu-latest\n"
        );
        assert_eq!(rules(&skipped), vec![(Some("ci-ok".into()), "aggregate")]);
    }

    fn with_if(cond: &str) -> String {
        CLEAN.replace(
            "github.event_name != 'pull_request' || github.event.pull_request.draft == false",
            cond,
        )
    }

    #[test]
    fn an_or_with_always_is_not_a_draft_skip() {
        for bad in [
            "always() || github.event.pull_request.draft == false",
            "${{ always() || github.event.pull_request.draft == false }}",
            "github.event.pull_request.draft == false || github.actor == 'bot'",
            "(always() || github.event.pull_request.draft == false) && true",
        ] {
            assert_eq!(
                rules(&with_if(bad)),
                vec![(Some("ci".into()), "draft")],
                "{bad}"
            );
        }
        for good in [
            "github.event.pull_request.draft == false",
            "needs.x.result == 'success' && github.event.pull_request.draft == false",
            "always() && (github.event_name != 'pull_request' || github.event.pull_request.draft == false)",
            "${{ github.event_name != 'pull_request' || github.event.pull_request.draft == false }}",
            "!github.event.pull_request.draft && github.actor != 'bot'",
        ] {
            assert_eq!(rules(&with_if(good)), vec![], "{good}");
        }
    }

    #[test]
    fn pull_request_target_needs_the_plain_draft_test() {
        let t = CLEAN.replace("  pull_request:", "  pull_request_target:");
        let f = check("w.yml", &t);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!((f[0].job.as_deref(), f[0].rule), (Some("ci"), "draft"));
        assert!(
            f[0].fix
                .contains("`if: github.event.pull_request.draft == false`"),
            "{}",
            f[0].fix
        );
        let plain = t.replace(
            "github.event_name != 'pull_request' || github.event.pull_request.draft == false",
            "github.event.pull_request.draft == false",
        );
        assert_eq!(rules(&plain), vec![]);
    }

    #[test]
    fn the_draft_fix_warns_about_required_checks() {
        let t = CLEAN.replace(
            "    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false\n",
            "",
        );
        let fix = &check("w.yml", &t)[0].fix;
        assert!(
            fix.contains("never on a required check: a skipped required check counts as passed; make it fail fast on a draft instead"),
            "{fix}"
        );
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
        let t = CLEAN
            .replace("  pull_request:", "  pull_request_target:")
            .replace(DRAFT_SKIP, DRAFT_SKIP_TARGET);
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

    const AGG: &str =
        "  ci-ok:\n    needs: [ci]\n    if: always()\n    runs-on: ubuntu-latest\n    steps:\n";

    #[test]
    fn an_aggregate_that_goes_green_on_a_draft_is_found() {
        // needs a draft-skipping job, always() runs it, but no step fails when that need is skipped.
        let green = format!("{CLEAN}{AGG}      - run: echo ok\n");
        assert_eq!(rules(&green), vec![(Some("ci-ok".into()), "aggregate")]);
        for good in [
            "      - run: test \"${{ needs.ci.result }}\" = \"success\"\n",
            "      - run: test ${{ needs.ci.result }} == 'success'\n",
            "      - env:\n          DRAFT: ${{ github.event.pull_request.draft }}\n        run: |\n          if [ \"$DRAFT\" = true ]; then exit 1; fi\n",
        ] {
            let t = format!("{CLEAN}{AGG}{good}");
            assert_eq!(rules(&t), vec![], "{good}");
        }
        // A draft check that never exits non-zero is not enough.
        let weak = format!(
            "{CLEAN}{AGG}      - run: echo \"draft ${{{{ github.event.pull_request.draft }}}}\"\n"
        );
        assert_eq!(rules(&weak), vec![(Some("ci-ok".into()), "aggregate")]);
    }

    fn push_only(cond: &str) -> String {
        format!("{CLEAN}  rel:\n    if: {cond}\n    runs-on: ubuntu-latest\n")
    }

    #[test]
    fn a_job_confined_to_other_events_needs_no_draft_guard() {
        for ok in [
            "github.event_name == 'push'",
            "${{ github.event_name == 'push' }}",
            "github.event_name == 'push' && github.ref == 'refs/heads/main'",
            "github.event_name == 'push' || github.event_name == 'schedule'",
        ] {
            assert_eq!(rules(&push_only(ok)), vec![], "{ok}");
        }
        for bad in [
            "github.event_name != 'push'",
            "github.event_name == 'push' || github.actor == 'bot'",
            "github.event_name == 'pull_request'",
        ] {
            assert_eq!(
                rules(&push_only(bad)),
                vec![(Some("rel".into()), "draft")],
                "{bad}"
            );
        }
    }

    #[test]
    fn a_job_that_needs_a_skipped_job_is_skipped_with_it() {
        let after = |needs: &str, cond: &str| {
            format!("{CLEAN}  b:\n    needs: {needs}\n{cond}    runs-on: ubuntu-latest\n")
        };
        assert_eq!(rules(&after("[ci]", "")), vec![]);
        assert_eq!(rules(&after("ci", "")), vec![]);
        assert_eq!(rules(&after("\n      - ci", "")), vec![]);
        let chain = format!(
            "{}  c:\n    needs: [b]\n    runs-on: x\n",
            after("[ci]", "")
        );
        assert_eq!(rules(&chain), vec![]);
        // always() runs it on a draft, so it needs its own guard.
        assert_eq!(
            rules(&after("[ci]", "    if: always()\n")),
            vec![(Some("b".into()), "draft")]
        );
        // A need that does not skip drafts does not help.
        let loose = format!("{CLEAN}  a:\n    runs-on: x\n  b:\n    needs: [a]\n    runs-on: x\n");
        assert_eq!(
            rules(&loose),
            vec![(Some("a".into()), "draft"), (Some("b".into()), "draft")]
        );
    }

    #[test]
    fn a_trigger_with_only_inert_types_is_not_read() {
        let bare = "jobs:\n  a:\n    runs-on: x\n";
        for ty in ["[closed]", "[labeled, unlabeled]", "[closed, labeled]"] {
            let t = format!(
                "on:\n  pull_request:\n    types: {ty}\nconcurrency:\n  group: g\n  cancel-in-progress: true\n{bare}"
            );
            assert_eq!(rules(&t), vec![], "{ty}");
        }
        let live = "on:\n  pull_request:\n    types: [labeled, edited]\nconcurrency:\n  group: g\n  cancel-in-progress: true\njobs:\n  a:\n    runs-on: x\n";
        assert_eq!(
            rules(live),
            vec![(Some("a".into()), "draft"), (None, "ready_for_review")]
        );
    }

    #[test]
    fn quoted_guards_count() {
        for cond in [
            "'!github.event.pull_request.draft'",
            "\"!github.event.pull_request.draft\"",
            "'${{ github.event.pull_request.draft == false }}'",
            "'github.event_name != ''pull_request'' || github.event.pull_request.draft == false'",
            "\"github.event.pull_request.draft == false\"",
        ] {
            assert_eq!(rules(&with_if(cond)), vec![], "{cond}");
        }
    }

    #[test]
    fn merge_keys_do_not_break_parsing() {
        let t = "on:
  pull_request:
    types: [opened, ready_for_review]
concurrency:
  group: g
  cancel-in-progress: true
x-base: &base
  runs-on: ubuntu-latest
  if: github.event.pull_request.draft == false
jobs:
  a:
    <<: *base
    steps:
      - run: x
  b:
    <<: *base
    if: always()
    steps:
      - run: y
  c:
    runs-on: ubuntu-latest
";
        assert_eq!(
            rules(t),
            vec![(Some("b".into()), "draft"), (Some("c".into()), "draft")]
        );
    }

    #[test]
    fn a_heavy_job_on_push_only_is_not_a_paths_finding() {
        let t = "on:\n  push:\n    branches: [main]\nconcurrency:\n  group: g\n  cancel-in-progress: true\njobs:\n  b:\n    runs-on: windows-latest\n";
        assert_eq!(rules(t), vec![]);
    }
    #[test]
    fn draft_not_true_counts_as_a_skip() {
        for g in [
            "github.event.pull_request.draft != true",
            "github.event_name != 'pull_request' || github.event.pull_request.draft != true",
        ] {
            let t = CLEAN.replace(
                "github.event_name != 'pull_request' || github.event.pull_request.draft == false",
                g,
            );
            assert_eq!(rules(&t), vec![], "{g}");
        }
    }

    #[test]
    fn a_job_whose_first_step_fails_on_a_draft_needs_no_skip() {
        let fast = CLEAN.replace(
            "    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false\n    runs-on: ubuntu-latest\n    steps:\n",
            "    runs-on: ubuntu-latest\n    steps:\n      - env:\n          DRAFT: ${{ github.event.pull_request.draft || false }}\n        run: if [ \"$DRAFT\" = true ]; then exit 1; fi\n",
        );
        assert_ne!(fast, CLEAN);
        assert_eq!(rules(&fast), vec![]);
        // The draft test after a heavy step does not save the minutes.
        let late = CLEAN.replace(
            "    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
            "    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n      - env:\n          DRAFT: ${{ github.event.pull_request.draft || false }}\n        run: if [ \"$DRAFT\" = true ]; then exit 1; fi\n",
        );
        assert_ne!(late, CLEAN);
        assert_eq!(rules(&late), vec![(Some("ci".into()), "draft")]);
    }

    #[test]
    fn a_label_branch_may_judge_a_draft() {
        let t = CLEAN.replace(
            "github.event_name != 'pull_request' || github.event.pull_request.draft == false",
            "(github.event.pull_request.draft == false && github.event.action != 'labeled') || (github.event.action == 'labeled' && github.event.label.name == 'bosun:kind/mainfix')",
        );
        assert_eq!(rules(&t), vec![]);
        // Any other `||` branch still breaks the skip.
        let t = CLEAN.replace(
            "github.event_name != 'pull_request' || github.event.pull_request.draft == false",
            "github.event.pull_request.draft == false || github.event.label.name != ''",
        );
        assert_eq!(rules(&t), vec![(Some("ci".into()), "draft")]);
    }
}
