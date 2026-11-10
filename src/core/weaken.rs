//! R-weaken (spec §8): findings for a PR that loosens a gate, from base/head file text and diffs.
//! Pure functions; a finding reads `{file}: {key} {old} → {new}`.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use toml::Value;

pub const CAPABILITY: &str = "weaken";
pub const RULE: &str = "R-weaken";
pub const WHY: &str = "A change that loosens a gate weakens what verifies every later change.";

#[derive(PartialEq)]
enum Kind {
    Caps,
    Config,
    Rules,
    Deny,
    Cargo,
    Profile,
}

fn kind(path: &str) -> Option<Kind> {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or_default();
    let toml = name.ends_with(".toml");
    Some(
        if lower.starts_with(".bosun/limits") && toml || name.contains("budget") && toml {
            Kind::Caps
        } else if lower == ".bosun/bosun.toml" {
            Kind::Config
        } else if lower == ".bosun/risk.toml" || name == "risk_default.toml" {
            Kind::Rules
        } else if [
            "deny.toml",
            ".deny.toml",
            "deny.exceptions.toml",
            ".deny.exceptions.toml",
        ]
        .contains(&name)
        {
            Kind::Deny
        } else if name == "cargo.toml" {
            Kind::Cargo
        } else if toml && format!("/{lower}").contains("/profiles/") {
            Kind::Profile
        } else {
            return None;
        },
    )
}

/// Files risk-check fetches whole at base and head.
pub fn reads_text(path: &str) -> bool {
    kind(path).is_some()
}

type Flat = BTreeMap<String, Value>;

fn flatten(v: &Value, key: &str, out: &mut Flat) {
    match v {
        Value::Table(t) => t.iter().for_each(|(k, v)| {
            // A dotted key stays quoted, so `"a.b" = 1` never collides with `[a] b = 1`.
            let k = if k.contains('.') {
                format!("{k:?}")
            } else {
                k.clone()
            };
            let k = if key.is_empty() {
                k
            } else {
                format!("{key}.{k}")
            };
            flatten(v, &k, out);
        }),
        _ => _ = out.insert(key.into(), v.clone()),
    }
}

fn show(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

fn num(v: &Value) -> Option<f64> {
    v.as_integer().map(|i| i as f64).or(v.as_float())
}

fn rank(v: &Value, order: &[&str]) -> Option<usize> {
    order.iter().position(|l| v.as_str() == Some(l))
}

fn items(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(show).collect())
        .unwrap_or_default()
}

fn parse(text: Option<&str>) -> Option<Value> {
    toml::from_str(text.unwrap_or_default()).ok()
}

/// Findings from a gate file's text at base and head. A missing side (an added or deleted file)
/// counts as empty; an unparseable head is a finding, an unparseable base gives none.
pub fn text_findings(path: &str, base: Option<&str>, head: Option<&str>) -> Vec<String> {
    let (Some(kind), Some(b)) = (kind(path), parse(base)) else {
        return Vec::new();
    };
    let Some(h) = parse(head) else {
        return vec![format!("{path}: toml parses → (unparseable)")];
    };
    let (mut fb, mut fh) = (Flat::new(), Flat::new());
    flatten(&b, "", &mut fb);
    flatten(&h, "", &mut fh);
    let mut out = Vec::new();
    let mut add = |k: &str, old: &str, new: &str| out.push(format!("{path}: {k} {old} → {new}"));
    match kind {
        Kind::Caps => {
            for (k, old) in &fb {
                // A non-number (`[models]`): any change, as a weaker model reviews less.
                let Some(o) = num(old) else {
                    if fh.get(k) != Some(old) {
                        add(k, &show(old), &fh.get(k).map_or("(removed)".into(), show));
                    }
                    continue;
                };
                match fh.get(k) {
                    None => add(k, &show(old), "(removed)"),
                    // NaN, a string or a bool is no cap.
                    Some(new) if !num(new).is_some_and(|n| n.is_finite() && n <= o) => {
                        add(k, &show(old), &show(new))
                    }
                    _ => {}
                }
            }
            // A new cap may lift a built-in default.
            for (k, new) in fh.iter().filter(|(k, _)| !fb.contains_key(*k)) {
                add(k, "(none)", &show(new));
            }
        }
        Kind::Config => {
            let gated = |k: &str| {
                k == "stack"
                    || ["approvals.", "prices.", "review."]
                        .iter()
                        .any(|p| k.starts_with(p))
            };
            for (k, old) in fb.iter().filter(|(k, _)| gated(k)) {
                match fh.get(k) {
                    None => add(k, &show(old), "(removed)"),
                    Some(new) if new != old => add(k, &show(old), &show(new)),
                    _ => {}
                }
            }
            for (k, new) in fh.iter().filter(|(k, _)| gated(k) && !fb.contains_key(*k)) {
                add(k, "(none)", &show(new));
            }
            // Switching CI off drops every CI check; another provider or the default is fine.
            if let Some(new) = fh.get("ci.provider").filter(|n| n.as_str() == Some("none"))
                && fb.get("ci.provider") != Some(new)
            {
                add(
                    "ci.provider",
                    &fb.get("ci.provider").map_or("(none)".into(), show),
                    &show(new),
                );
            }
        }
        Kind::Deny => {
            let adds = |l: &str| {
                let all = "allow ignore skip skip-tree exceptions allow-git allow-registry";
                all.split(' ').any(|a| a == l)
            };
            const LEVELS: [&str; 3] = ["allow", "warn", "deny"];
            for (k, old) in &fb {
                let org = k.split('.').any(|s| s == "allow-org");
                let last = if org {
                    "allow-org"
                } else {
                    k.rsplit('.').next().unwrap_or_default()
                };
                let new = fh.get(k);
                if let Some(r) = rank(old, &LEVELS) {
                    if new.and_then(|n| rank(n, &LEVELS)).is_none_or(|n| n < r) {
                        add(k, &show(old), &new.map_or("(removed)".into(), show));
                    }
                } else if old.is_array() && (last == "deny" || org || adds(last)) {
                    let (o, n) = (items(old), new.map(items).unwrap_or_default());
                    if last == "deny" {
                        o.iter()
                            .filter(|i| !n.contains(i))
                            .for_each(|i| add(k, i, "(removed)"));
                    } else {
                        n.iter()
                            .filter(|i| !o.contains(i))
                            .for_each(|i| add(k, "(none)", i));
                    }
                } else if new != Some(old) {
                    // Any other setting (graph, scopes, thresholds, bools) changed or removed.
                    add(k, &show(old), &new.map_or("(removed)".into(), show));
                }
            }
            // Keys new at head: allow-list items, any other setting but a `deny`.
            for (k, new) in fh.iter().filter(|(k, _)| !fb.contains_key(*k)) {
                let last = k.rsplit('.').next().unwrap_or_default();
                if new.is_array() && (adds(last) || k.split('.').any(|s| s == "allow-org")) {
                    items(new).iter().for_each(|i| add(k, "(none)", i));
                } else if last != "deny" && rank(new, &LEVELS) != Some(2) {
                    // Any new setting may narrow what is checked; only a new `deny` tightens.
                    add(k, "(none)", &show(new));
                }
            }
        }
        Kind::Cargo => {
            const LEVELS: [&str; 4] = ["allow", "warn", "deny", "forbid"];
            let under = |k: &str| k.starts_with("lints.") || k.starts_with("workspace.lints.");
            let key = |k: &str| k.strip_suffix(".level").unwrap_or(k).to_string();
            let norm = |f: &Flat| -> Flat {
                f.iter()
                    .filter(|(k, _)| under(k))
                    .map(|(k, v)| (key(k), v.clone()))
                    .collect()
            };
            let (nb, nh) = (norm(&fb), norm(&fh));
            for (k, new) in nh.iter().filter(|(k, _)| !nb.contains_key(*k)) {
                // A new `priority` can lift an allow group over a deny.
                if rank(new, &LEVELS) == Some(0) || k.ends_with(".priority") {
                    add(k, "(none)", &show(new));
                }
            }
            for (k, old) in nb {
                let new = nh.get(&k);
                if k.ends_with(".priority") && new != Some(&old) {
                    add(&k, &show(&old), &new.map_or("(removed)".into(), show));
                } else if k.ends_with("lints.workspace") {
                    if old.as_bool() == Some(true) && new.and_then(Value::as_bool) != Some(true) {
                        add(&k, "true", &new.map_or("(removed)".into(), show));
                    }
                } else if let Some(r) = rank(&old, &LEVELS)
                    && new.and_then(|n| rank(n, &LEVELS)).is_none_or(|n| n < r)
                {
                    add(&k, &show(&old), &new.map_or("(removed)".into(), show));
                }
            }
        }
        // Any key changed or removed: a profile replaces the built-in one whole, and `manifest`,
        // `adapters`, `when_no_package` or a `cmd` argument each decide what a gate checks.
        Kind::Profile => {
            if fb.is_empty() && !fh.is_empty() {
                add("profile", "(built-in)", "(new)");
            }
            for (k, old) in &fb {
                match fh.get(k) {
                    None => add(k, &show(old), "(removed)"),
                    Some(new) if new != old => add(k, &show(old), &show(new)),
                    _ => {}
                }
            }
            // A key added to what exists (`manifest`, `when_no_package`, `each_package`) can
            // narrow a gate; a whole new gate cannot.
            let gate = |k: &str| k.split('.').take(2).collect::<Vec<_>>().join(".");
            for (k, new) in fh.iter().filter(|(k, _)| !fb.contains_key(*k)) {
                let fresh = k.starts_with("gates.") && !fb.keys().any(|b| gate(b) == gate(k));
                if !fb.is_empty() && !fresh {
                    add(k, "(none)", &show(new));
                }
            }
        }
        Kind::Rules => {
            let rules = |v: &Value| -> Vec<(String, Value)> {
                let all = v.get("rule").and_then(Value::as_array);
                (all.into_iter().flatten())
                    .filter_map(|r| Some((r.get("id")?.as_str()?.to_string(), r.clone())))
                    .collect()
            };
            let head: HashMap<_, _> = rules(&h).into_iter().collect();
            for (id, old) in rules(&b) {
                let Some(new) = head.get(&id) else {
                    add(&id, "rule present", "(removed)");
                    continue;
                };
                let get = |v: &Value, k: &str| v.get(k).cloned();
                let np = items(new.get("paths").unwrap_or(&Value::Array(vec![])));
                for p in items(old.get("paths").unwrap_or(&Value::Array(vec![]))) {
                    if !np.contains(&p) {
                        add(&format!("{id}.paths"), &p, "(removed)");
                    }
                }
                let appr = |v: &Value| v.get("approval").map_or("owner".into(), show);
                if appr(new) != appr(&old) && appr(new) != "owner" {
                    add(&format!("{id}.approval"), &appr(&old), &appr(new));
                }
                for k in ["content", "capability", "new_dependency"] {
                    let (o, n) = (get(&old, k), get(new, k));
                    let lost = k == "new_dependency" && o == Some(Value::Boolean(true)) && n != o;
                    if lost || k != "new_dependency" && o.is_some() && n != o {
                        let n = n.as_ref().map_or("(none)".into(), show);
                        add(
                            &format!("{id}.{k}"),
                            &show(&o.unwrap_or(Value::Boolean(false))),
                            &n,
                        );
                    }
                }
            }
        }
    }
    out
}

/// The `-` and `+` lines of a unified diff, without the marker; `---`/`+++` file headers are
/// skipped only before the first hunk (GitHub patches have none, so a later `--x` is content).
pub fn split_patch(patch: &str) -> (Vec<&str>, Vec<&str>) {
    let (mut del, mut ins, mut hunk) = (Vec::new(), Vec::new(), false);
    for l in patch.lines() {
        hunk |= l.starts_with("@@");
        if !hunk && (l.starts_with("---") || l.starts_with("+++")) {
            continue;
        }
        if let Some(r) = l.strip_prefix('-') {
            del.push(r);
        } else if let Some(r) = l.strip_prefix('+') {
            ins.push(r);
        }
    }
    (del, ins)
}

static COVERAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(fail[-_]under(?:[-_]lines)?|[\w-]*threshold[\w-]*|min[-_]coverage|coverage[-_]min)\b["']?\s*[=: ]\s*["']?(\d+(?:\.\d+)?)"#).unwrap()
});
static CONST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bconst\s+([A-Z][A-Z0-9_]*)\s*:[^=]+=\s*(-?\d[\d_]*(?:\.\d+)?)").unwrap()
});

type Vals = Vec<(String, Vec<f64>)>;

/// Every value per key over the lines, keys in first-seen order.
fn all_values(re: &Regex, lines: &[&str], lower: bool) -> Vals {
    let mut out = Vals::new();
    for c in lines.iter().filter_map(|l| re.captures(l)) {
        let k = if lower {
            c[1].to_lowercase()
        } else {
            c[1].to_string()
        };
        let v = c[2].replace('_', "").parse().unwrap_or(0.0);
        match out.iter_mut().find(|(n, _)| *n == k) {
            Some((_, vs)) => vs.push(v),
            None => out.push((k, vec![v])),
        }
    }
    out
}

fn extreme(v: &[f64], max: bool) -> f64 {
    v.iter()
        .copied()
        .reduce(if max { f64::max } else { f64::min })
        .unwrap_or(0.0)
}

fn count(lines: &[&str], f: impl Fn(&str) -> bool) -> usize {
    lines.iter().filter(|l| f(l)).count()
}

fn grows(
    path: &str,
    what: &str,
    del: &[&str],
    ins: &[&str],
    f: impl Fn(&str) -> bool,
) -> Option<String> {
    let (r, a) = (count(del, &f), count(ins, &f));
    (a > r).then(|| format!("{path}: {what} {r} → {a}"))
}

fn cmd_of(line: &str) -> String {
    let t = line.trim();
    let t = t.strip_prefix("- ").unwrap_or(t).trim();
    let t = t.strip_prefix("run:").unwrap_or(t).trim();
    let t = t.trim_matches(['"', '\'']).trim();
    // An action by name, so a version bump is no removal.
    match t.strip_prefix("uses:") {
        Some(u) => format!("uses: {}", u.trim().split('@').next().unwrap_or_default()),
        None => t.to_string(),
    }
}

pub fn is_ci(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or_default();
    [
        ".github/workflows/",
        ".github/actions/",
        ".circleci/",
        ".gitlab/ci/",
        ".buildkite/",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
        || [
            "action.yml",
            "action.yaml",
            ".gitlab-ci.yml",
            "jenkinsfile",
            "azure-pipelines.yml",
            "azure-pipelines.yaml",
            "bitbucket-pipelines.yml",
            ".travis.yml",
            ".drone.yml",
        ]
        .contains(&name)
}

/// `key: value` of a YAML line with spaces, quotes and a trailing ` #` comment dropped.
fn kv(line: &str) -> Option<(String, String)> {
    let (k, v) = line.trim().trim_start_matches("- ").split_once(':')?;
    let v = v.split(" #").next().unwrap_or_default();
    let v = v
        .chars()
        .filter(|c| !c.is_whitespace() && !"'\"".contains(*c));
    Some((k.trim().into(), v.collect()))
}

/// CI command lines of a diff: (removed `run:`/tool lines, all added ones). Names and comments
/// are no commands. Compare removed with added over the whole PR: folding moves steps.
pub fn ci_commands(path: &str, patch: &str) -> (Vec<String>, Vec<String>) {
    if !is_ci(path) {
        return Default::default();
    }
    let (del, ins) = split_patch(patch);
    let cmd = |l: &&str| -> bool {
        let t = l.trim();
        !(t.starts_with("name:") || t.starts_with('#') || t.starts_with("- name:"))
    };
    // Lines of a removed `run: |` block are commands too.
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut block: Option<usize> = None;
    let removed = del.iter().copied().filter(|l| cmd(l)).filter_map(|l| {
        let t = l.trim().trim_start_matches("- ");
        let inside = block.is_some_and(|i| indent(l) > i && !t.is_empty());
        if !inside {
            block = None;
        }
        let run = t.starts_with("run:") || t.starts_with("uses:") || inside;
        let tool = ["cargo ", "bosun ", "npm ", "pnpm ", "npx "]
            .iter()
            .any(|t| l.contains(t));
        let c = cmd_of(l);
        let header = ["|", "|-", "|+", ">", ">-", ">+"].contains(&c.as_str());
        if header {
            block = Some(indent(l));
        }
        ((run || tool) && !c.is_empty() && !header).then_some(c)
    });
    (
        removed.collect(),
        ins.iter().copied().filter(|l| cmd(l)).map(cmd_of).collect(),
    )
}

/// Rust text with `/* */` comments (nested too) blanked, line breaks kept.
pub fn strip_block_comments(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let (mut out, mut depth, mut i) = (String::new(), 0usize, 0);
    let at = |i: usize, s: &str| s.chars().enumerate().all(|(k, x)| c.get(i + k) == Some(&x));
    while i < c.len() {
        let start = i;
        if depth > 0 {
            depth = if at(i, "/*") {
                depth + 1
            } else if at(i, "*/") {
                depth - 1
            } else {
                depth
            };
            i += if at(i, "/*") || at(i, "*/") { 2 } else { 1 };
            out.extend(c[start..i].iter().filter(|x| **x == '\n'));
            continue;
        }
        if at(i, "/*") {
            (depth, i) = (1, i + 2);
            continue;
        }
        // Code that cannot open a comment: a line comment, a string, a raw string, a char.
        if at(i, "//") {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if c[i] == '"' || (c[i] == 'r' && matches!(c.get(i + 1), Some('"' | '#'))) {
            let hashes = c[i..]
                .iter()
                .skip(usize::from(c[i] == 'r'))
                .take_while(|x| **x == '#')
                .count();
            let close: String = std::iter::once('"')
                .chain("#".repeat(hashes).chars())
                .collect();
            i += usize::from(c[i] == 'r') + hashes + 1;
            while i < c.len() && !at(i, &close) {
                i += if c[i] == '\\' && hashes == 0 { 2 } else { 1 };
            }
            i = (i + close.len()).min(c.len());
        } else if c[i] == '\'' && c.get(i + 2) == Some(&'\'') || at(i, "'\\") {
            i += 1 + c[i + 1..]
                .iter()
                .skip(1)
                .position(|x| *x == '\'')
                .map_or(0, |p| p + 2);
        } else {
            i += 1;
        }
        out.extend(&c[start..i.max(start + 1).min(c.len())]);
        i = i.max(start + 1);
    }
    out
}

/// A `#[cfg(..)]` line that never holds: `any()`, `not(all())`, or a bare name rustc does not
/// set (`false`, `FALSE`, a typo). A `key = ".."` pair is fine.
pub fn cfg_off(line: &str) -> bool {
    const KNOWN: [&str; 10] = [
        "test",
        "debug_assertions",
        "unix",
        "windows",
        "doc",
        "doctest",
        "miri",
        "not",
        "all",
        "any",
    ];
    let l: String = (line.split("//").next().unwrap_or_default().chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    let Some(inner) = l.strip_prefix("#[cfg(").or(l.strip_prefix("#![cfg(")) else {
        return false;
    };
    let off = ["any()", "not(all())", "not(test)", "feature=\"never\""];
    if off.iter().any(|o| inner.contains(o)) {
        return true;
    }
    let mut rest = inner;
    while let Some(i) = rest.find(|c: char| c.is_alphanumeric() || c == '_') {
        let r = &rest[i..];
        let end = r
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(r.len());
        let (name, after) = r.split_at(end);
        if after.starts_with('=') {
            // Skip the value.
            rest = after
                .split_once(['"'])
                .and_then(|(_, v)| v.split_once('"'))
                .map_or("", |x| x.1);
            continue;
        }
        if !KNOWN.contains(&name) {
            return true;
        }
        rest = after;
    }
    false
}

/// Findings from a diff: CI steps (`added` is every command the PR adds), coverage floors, lint
/// switches and Rust threshold constants.
pub fn patch_findings(path: &str, patch: &str, added: &[String]) -> Vec<String> {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or_default();
    let ext = |es: &[&str]| es.iter().any(|e| lower.ends_with(e));
    let js = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"];
    let (del, ins) = split_patch(patch);
    let mut out = Vec::new();
    if is_ci(path) {
        let coe = |l: &str| {
            kv(l).is_some_and(|(k, v)| {
                k == "continue-on-error"
                    && ["true", "${{true}}"].contains(&v.to_lowercase().as_str())
            })
        };
        out.extend(grows(path, "continue-on-error: true", &del, &ins, coe));
        let never = |l: &str| {
            kv(l).is_some_and(|(k, v)| {
                k == "if"
                    && (["false", "${{false}}", "${{!true}}"].contains(&v.as_str())
                        || v.contains("0==1"))
            })
        };
        if ins.iter().any(|l| never(l)) {
            out.push(format!("{path}: if (none) → false"));
        }
        // Digits masked: a version bump (`tool@1.2` → `tool@1.3`) is an edit, not a removal.
        let mask = |c: &str| -> String {
            c.chars()
                .map(|x| if x.is_ascii_digit() { '0' } else { x })
                .collect()
        };
        let kept: Vec<String> = added.iter().map(|a| mask(a)).collect();
        for c in (ci_commands(path, patch).0.iter()).filter(|c| !kept.contains(&mask(c))) {
            out.push(format!("{path}: step \"{c}\" present → (removed)"));
        }
    }
    if lower.ends_with(".rs") {
        let allow = |l: &str| {
            let t = l.trim();
            (t.starts_with("#[") || t.starts_with("#!["))
                && (t.contains("allow(") || t.contains("expect("))
        };
        out.extend(grows(path, "lint allow/expect", &del, &ins, allow));
        let strict =
            |l: &str| l.trim().starts_with("#![forbid(") || l.trim().starts_with("#![deny(");
        let (r, a) = (count(&del, strict), count(&ins, strict));
        if r > a {
            out.push(format!("{path}: #![forbid/deny] {r} → {a}"));
        }
    }
    if ["oxlint", "eslintrc", "eslint.config"]
        .iter()
        .any(|n| name.contains(n))
    {
        let has = |ps: [&'static str; 2]| move |l: &str| ps.iter().any(|p| l.contains(p));
        let (off, warn) = (has(["\"off\"", "'off'"]), has(["\"warn\"", "'warn'"]));
        out.extend(grows(path, "\"off\"", &del, &ins, off));
        out.extend(grows(path, "\"warn\"", &del, &ins, warn));
    }
    if ext(&js) {
        let dis = |l: &str| l.contains("oxlint-disable") || l.contains("eslint-disable");
        out.extend(grows(path, "lint disable", &del, &ins, dis));
    }
    let cfg = [
        ".toml",
        ".yml",
        ".yaml",
        ".json",
        ".cfg",
        ".ini",
        ".conf",
        ".xml",
        ".coveragerc",
    ];
    let (cd, ci) = (
        all_values(&COVERAGE, &del, true),
        all_values(&COVERAGE, &ins, true),
    );
    for (k, o) in cd.iter().filter(|_| ext(&cfg) && name != "clippy.toml") {
        let old = extreme(o, false);
        match ci.iter().find(|(n, _)| n == k) {
            None => out.push(format!("{path}: {k} {old} → (removed)")),
            Some((_, n)) if extreme(n, false) < old => {
                out.push(format!("{path}: {k} {old} → {}", extreme(n, false)));
            }
            _ => {}
        }
    }
    if lower.ends_with(".rs") {
        let new = all_values(&CONST, &ins, false);
        for (k, o) in all_values(&CONST, &del, false) {
            let has = |w: &[&str]| k.split('_').any(|s| w.contains(&s));
            let max = if has(&["MIN", "THRESHOLD", "NEEDED", "REQUIRED"]) {
                false
            } else if has(&["MAX", "LIMIT", "CAP", "BUDGET"]) {
                true
            } else {
                continue;
            };
            let Some((_, n)) = new.iter().find(|(n, _)| *n == k) else {
                out.push(format!("{path}: {k} {} → (removed)", extreme(&o, max)));
                continue;
            };
            let (old, new) = (extreme(&o, max), extreme(n, max));
            if (new > old) == max && new != old {
                out.push(format!("{path}: {k} {old} → {new}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(path: &str, base: &str, head: &str) -> Vec<String> {
        text_findings(path, Some(base), Some(head))
    }
    fn p(path: &str, patch: &str) -> Vec<String> {
        patch_findings(path, patch, &ci_commands(path, patch).1)
    }

    #[test]
    fn block_comments_skip_strings_and_chars() {
        let t = "let a = \"src/**\";\n#[test]\nfn x() {}\nlet b = '\"'; // /* no\nlet r = r#\"/*\"#;\n#[test]\n";
        assert_eq!(strip_block_comments(t), t);
        assert_eq!(strip_block_comments("a /* \"*/ b"), "a  b");
    }

    #[test]
    fn added_profile_keys_narrow_a_gate() {
        let base = "[gates.test]\ncmd = [\"cargo\", \"test\"]\n";
        let got = t(
            "profiles/rust.toml",
            base,
            &format!("manifest = \"x\"\n{base}when_no_package = \"skip\"\n"),
        );
        assert_eq!(
            got,
            [
                "profiles/rust.toml: gates.test.when_no_package (none) → skip",
                "profiles/rust.toml: manifest (none) → x"
            ]
        );
        assert!(
            t(
                "profiles/rust.toml",
                base,
                &format!("{base}[gates.more]\ncmd = [\"y\"]\nall = [\"z\"]\n")
            )
            .is_empty()
        );
    }

    #[test]
    fn cfg_switch_offs() {
        for on in [
            "#[cfg(test)]",
            "#[cfg(all(unix, feature = \"x-y\"))]",
            "#[cfg(not(windows))]",
        ] {
            assert!(!cfg_off(on), "{on}");
        }
        for off in [
            "#[cfg(any())]",
            "#[cfg(FALSE)]",
            "#[cfg(false)]",
            "#[cfg(tset)]",
            "#[cfg(not(test))]",
            "#[cfg(feature = \"never\")]",
            "#[cfg(not(all()))]",
        ] {
            assert!(cfg_off(off), "{off}");
        }
    }

    #[test]
    fn version_bumps_and_block_comments() {
        let w = ".github/workflows/ci.yml";
        let bump = ["cargo install cargo-deny@0.16.2".to_string()];
        assert!(
            patch_findings(
                w,
                "@@\n-      - run: cargo install cargo-deny@0.14.1\n",
                &bump
            )
            .is_empty()
        );
        assert_eq!(strip_block_comments("a/* x\n/* y */ */b\n"), "a\nb\n");
    }

    #[test]
    fn review_round_three_diffs() {
        let w = ".github/workflows/ci.yml";
        let patch = "@@\n-      - uses: EmbarkStudios/cargo-deny-action@v1\n-        run: |-\n-          make test\n";
        assert_eq!(
            patch_findings(w, patch, &[]),
            [
                ".github/workflows/ci.yml: step \"uses: EmbarkStudios/cargo-deny-action\" present → (removed)",
                ".github/workflows/ci.yml: step \"make test\" present → (removed)"
            ]
        );
        let bump = ["uses: EmbarkStudios/cargo-deny-action".to_string()];
        assert!(
            patch_findings(
                w,
                "@@\n-  - uses: EmbarkStudios/cargo-deny-action@v1\n",
                &bump
            )
            .is_empty()
        );
        assert!(is_ci(".gitlab-ci.yml") && is_ci("ci/Jenkinsfile"));
        assert_eq!(
            patch_findings("src/a.rs", "@@\n-const MIN_X: u8 = 3;\n", &[]),
            ["src/a.rs: MIN_X 3 → (removed)"]
        );
    }

    #[test]
    fn caps() {
        let f = ".bosun/limits.toml";
        assert_eq!(
            t(
                f,
                "[budget]\ndaily_usd = 150\n",
                "[budget]\ndaily_usd = 300\n"
            ),
            [".bosun/limits.toml: budget.daily_usd 150 → 300"]
        );
        assert!(
            t(
                f,
                "[budget]\ndaily_usd = 150\n",
                "[budget]\ndaily_usd = 100\n"
            )
            .is_empty()
        );
        assert_eq!(
            t(f, "a = 1\n", "b = 1\n"),
            [
                ".bosun/limits.toml: a 1 → (removed)",
                ".bosun/limits.toml: b (none) → 1"
            ]
        );
        assert_eq!(t("x/team-budget.toml", "a = 1.5\n", "a = 2.5\n").len(), 1);
        assert!(t(f, "a = [", "a = 5").is_empty());
        assert_eq!(
            text_findings(f, None, Some("a = 5")),
            [".bosun/limits.toml: a (none) → 5"]
        );
    }

    #[test]
    fn risk_rules() {
        let f = ".bosun/risk.toml";
        let base = "[[rule]]\nid = \"A\"\npaths = [\"a/**\", \"b/**\"]\ncontent = \"x\"\nnew_dependency = true\ncapability = \"c\"\n[[rule]]\nid = \"B\"\n";
        assert_eq!(
            t(
                f,
                base,
                "[[rule]]\nid = \"A\"\npaths = [\"a/**\", \"b/**\"]\ncontent = \"x\"\nnew_dependency = true\ncapability = \"c\"\n"
            ),
            [".bosun/risk.toml: B rule present → (removed)"]
        );
        let head = "[[rule]]\nid = \"A\"\npaths = [\"a/**\"]\napproval = \"delegate\"\ncontent = \"y\"\n[[rule]]\nid = \"B\"\n";
        let got = t(f, base, head).join("\n");
        for want in [
            "A.paths b/** → (removed)",
            "A.approval owner → delegate",
            "A.content x → y",
            "A.new_dependency true → (none)",
            "A.capability c → (none)",
        ] {
            assert!(got.contains(want), "{want}\n{got}");
        }
        // Tightening or equal: no findings.
        let more = "[[rule]]\nid = \"A\"\npaths = [\"a/**\", \"b/**\", \"c/**\"]\ncontent = \"x\"\nnew_dependency = true\ncapability = \"c\"\n[[rule]]\nid = \"B\"\napproval = \"owner\"\n[[rule]]\nid = \"Z\"\n";
        assert!(t("src/core/risk_default.toml", base, more).is_empty());
    }

    #[test]
    fn cargo_deny() {
        let f = "deny.toml";
        let base = "[bans]\nmultiple-versions = \"deny\"\ndeny = [\"x\"]\n[licenses]\nallow = [\"MIT\"]\n[advisories]\nignore = []\n";
        let head = "[bans]\nmultiple-versions = \"warn\"\ndeny = []\n[licenses]\nallow = [\"MIT\", \"GPL\"]\n[advisories]\nignore = [\"RUSTSEC-1\"]\n";
        let got = t(f, base, head).join("\n");
        for want in [
            "bans.multiple-versions deny → warn",
            "bans.deny x → (removed)",
            "licenses.allow (none) → GPL",
            "advisories.ignore (none) → RUSTSEC-1",
        ] {
            assert!(got.contains(want), "{want}\n{got}");
        }
        assert_eq!(t(f, base, head).len(), 4);
        assert!(t(f, base, base).is_empty());
        assert_eq!(t(f, base, "").len(), 2);
        let tight = "[bans]\nmultiple-versions = \"deny\"\ndeny = [\"x\", \"y\"]\n[licenses]\nallow = []\n[advisories]\nignore = []\n";
        assert!(t(".deny.toml", base, tight).is_empty());
    }

    #[test]
    fn review_round_two() {
        let l = ".bosun/limits.toml";
        assert_eq!(
            t(l, "a = 1\n", "a = nan\n"),
            [".bosun/limits.toml: a 1 → nan"]
        );
        assert_eq!(
            t(l, "a = 1\n", "a = \"x\"\n"),
            [".bosun/limits.toml: a 1 → x"]
        );
        assert_eq!(
            text_findings(l, None, Some("a = 9\n")),
            [".bosun/limits.toml: a (none) → 9"]
        );
        let c = ".bosun/bosun.toml";
        let base = "project = \"p\"\n[approvals]\npolicy = \"owner-gated\"\n";
        assert_eq!(
            t(c, base, &base.replace("owner-gated", "autonomous")),
            [".bosun/bosun.toml: approvals.policy owner-gated → autonomous"]
        );
        assert!(t(c, base, &base.replace("\"p\"", "\"q\"")).is_empty());
        assert_eq!(
            t(
                "deny.toml",
                "[graph]\nall-features = true\n",
                "[graph]\nall-features = false\n"
            ),
            ["deny.toml: graph.all-features true → false"]
        );
    }

    #[test]
    fn ci_provider_none_is_a_finding() {
        let c = ".bosun/bosun.toml";
        let gh = "project = \"p\"\n[ci]\nprovider = \"github\"\n";
        assert_eq!(
            t(c, gh, &gh.replace("github", "none")),
            [".bosun/bosun.toml: ci.provider github → none"]
        );
        assert_eq!(
            t(c, "project = \"p\"\n", &gh.replace("github", "none")),
            [".bosun/bosun.toml: ci.provider (none) → none"]
        );
        // Another provider, or dropping the key (github), keeps CI on.
        assert!(t(c, gh, &gh.replace("github", "circleci")).is_empty());
        assert!(t(c, &gh.replace("github", "none"), "project = \"p\"\n").is_empty());
    }

    #[test]
    fn review_round_three() {
        let d = "deny.toml";
        let g = "[graph]\nall-features = true\n";
        assert_eq!(
            t(d, g, &format!("{g}exclude = [\"x\"]\n")),
            ["deny.toml: graph.exclude (none) → [\"x\"]"]
        );
        assert!(t(d, g, &format!("{g}[bans]\ndeny = [\"y\"]\n")).is_empty());
        let p = "[licenses.private]\nignore = false\n";
        assert_eq!(
            t(d, p, &p.replace("false", "true")),
            ["deny.toml: licenses.private.ignore false → true"]
        );
        let l = ".bosun/limits.toml";
        assert_eq!(
            t(
                l,
                "[models]\nreview = \"sonnet\"\n",
                "[models]\nreview = \"haiku\"\n"
            ),
            [".bosun/limits.toml: models.review sonnet → haiku"]
        );
        let c = ".bosun/bosun.toml";
        assert_eq!(
            t(
                c,
                "[review]\nmode = \"ci\"\n",
                "[review]\nmode = \"subagent\"\n"
            ),
            [".bosun/bosun.toml: review.mode ci → subagent"]
        );
    }

    #[test]
    fn dotted_keys_and_lint_priority() {
        let c = "Cargo.toml";
        let base = "[lints.rust]\nunsafe_code = \"forbid\"\n";
        let head =
            "\"lints.rust.unsafe_code\" = \"forbid\"\n[lints.rust]\nunsafe_code = \"allow\"\n";
        assert_eq!(
            t(c, base, head),
            ["Cargo.toml: lints.rust.unsafe_code forbid → allow"]
        );
        let p = |n: i32| format!("[lints.clippy]\nall = {{ level = \"allow\", priority = {n} }}\n");
        assert_eq!(
            t(c, &p(-1), &p(1)),
            ["Cargo.toml: lints.clippy.all.priority -1 → 1"]
        );
    }

    #[test]
    fn new_keys_that_loosen() {
        let d = "deny.toml";
        assert_eq!(
            t(d, "[sources]\n", "[sources.allow-org]\ngithub = [\"x\"]\n"),
            ["deny.toml: sources.allow-org.github (none) → x"]
        );
        assert_eq!(
            t(d, "[sources]\n", "[sources]\nunknown-git = \"allow\"\n"),
            ["deny.toml: sources.unknown-git (none) → allow"]
        );
        assert_eq!(
            text_findings(d, None, Some("[advisories]\nignore = [\"R-1\"]\n")),
            ["deny.toml: advisories.ignore (none) → R-1"]
        );
        assert_eq!(
            t(
                "Cargo.toml",
                "[lints.rust]\n",
                "[lints.rust]\ndead_code = \"allow\"\n"
            ),
            ["Cargo.toml: lints.rust.dead_code (none) → allow"]
        );
    }

    #[test]
    fn cargo_lints() {
        let base = "[lints.rust]\nunsafe_code = \"forbid\"\n[lints.clippy]\nx = { level = \"deny\", priority = -1 }\n[workspace.lints.clippy]\ny = \"warn\"\n";
        let head = "[lints.rust]\nunsafe_code = \"deny\"\n[lints.clippy]\nx = \"deny\"\n";
        let got = t("crates/a/Cargo.toml", base, head).join("\n");
        assert!(
            got.contains("lints.rust.unsafe_code forbid → deny"),
            "{got}"
        );
        assert!(
            got.contains("workspace.lints.clippy.y warn → (removed)"),
            "{got}"
        );
        assert!(!got.contains("lints.clippy.x deny"), "{got}");
        assert!(
            got.contains("lints.clippy.x.priority -1 → (removed)"),
            "{got}"
        );
        assert_eq!(
            t("Cargo.toml", "[lints]\nworkspace = true\n", "[package]\n"),
            ["Cargo.toml: lints.workspace true → (removed)"]
        );
        assert!(
            t(
                "Cargo.toml",
                "[lints.rust]\na = \"warn\"\n",
                "[lints.rust]\na = \"deny\"\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn profiles() {
        let base = "manifest = \"Cargo.toml\"\n[gates.fmt]\ncmd = [\"cargo\", \"fmt\", \"--check\"]\n[gates.lint]\ncmd = [\"x\"]\n";
        let head = "manifest = \"no.toml\"\n[gates.fmt]\ncmd = [\"cargo\", \"fmt\"]\n";
        let got = t("profiles/rust.toml", base, head);
        assert_eq!(
            got,
            [
                "profiles/rust.toml: gates.fmt.cmd [\"cargo\", \"fmt\", \"--check\"] → [\"cargo\", \"fmt\"]",
                "profiles/rust.toml: gates.lint.cmd [\"x\"] → (removed)",
                "profiles/rust.toml: manifest Cargo.toml → no.toml"
            ]
        );
        assert_eq!(t(".bosun/profiles/x.toml", base, head).len(), 3);
        // A new gate is no weakening.
        assert!(
            t(
                "profiles/rust.toml",
                base,
                &format!("{base}[gates.more]\ncmd = [\"y\"]\n")
            )
            .is_empty()
        );
        assert_eq!(
            t("profiles/rust.toml", base, "manifest = ["),
            ["profiles/rust.toml: toml parses → (unparseable)"]
        );
        assert!(!reads_text("src/main.rs") && reads_text("a/Cargo.toml"));
    }

    #[test]
    fn ci_steps() {
        let w = ".github/workflows/ci.yml";
        let got = p(
            w,
            "@@\n-      - run: cargo clippy -D warnings\n+      - run: cargo fmt\n-        continue-on-error: false\n+        continue-on-error: true\n+        if: false\n",
        );
        let got = got.join("\n");
        assert!(got.contains("continue-on-error: true 0 → 1"), "{got}");
        assert!(got.contains("ci.yml: if (none) → false"), "{got}");
        assert!(
            got.contains("step \"cargo clippy -D warnings\" present → (removed)"),
            "{got}"
        );
        // A moved step is not removed; a non-CI file is not read for steps.
        assert!(
            p(
                w,
                "@@\n-  - run: cargo test\n+  - run: cargo test\n+  - run: cargo fmt\n"
            )
            .is_empty()
        );
        assert!(p("README.md", "@@\n-cargo test\n").is_empty());
        assert_eq!(p(w, "@@\n-        cargo nextest run\n").len(), 1);
        assert_eq!(p(w, "@@\n+  if: ${{ false }}\n").len(), 1);
    }

    #[test]
    fn coverage_lint_and_consts() {
        assert_eq!(
            p("ci.cfg", "@@\n-fail-under = 90\n+fail-under = 80\n"),
            ["ci.cfg: fail-under 90 → 80"]
        );
        assert_eq!(p("a.json", "@@\n-  \"threshold\": 80\n").len(), 1);
        assert!(p("ci.cfg", "@@\n-fail-under = 80\n+fail-under = 90\n").is_empty());
        assert_eq!(
            p("src/a.rs", "@@\n+#[allow(clippy::x)]\n"),
            ["src/a.rs: lint allow/expect 0 → 1"]
        );
        assert!(p("src/a.rs", "@@\n-#[allow(clippy::x)]\n+fn a() {}\n").is_empty());
        assert_eq!(p(".oxlintrc.json", "@@\n+  \"a\": \"off\"\n").len(), 1);
        assert_eq!(p("web/a.ts", "@@\n+// eslint-disable-next-line\n").len(), 1);
        let c = p(
            "src/core/risk.rs",
            "@@\n-const VERDICT_MIN_CONFIDENCE: f64 = 0.9;\n+const VERDICT_MIN_CONFIDENCE: f64 = 0.8;\n-const MAX_N: usize = 3;\n+const MAX_N: usize = 5;\n",
        );
        assert_eq!(
            c,
            [
                "src/core/risk.rs: VERDICT_MIN_CONFIDENCE 0.9 → 0.8",
                "src/core/risk.rs: MAX_N 3 → 5"
            ]
        );
        assert!(
            p(
                "src/a.rs",
                "@@\n-const IDLE_MINUTES: u8 = 9;\n+const IDLE_MINUTES: u8 = 1;\n"
            )
            .is_empty()
        );
        assert!(p("src/a.rs", "@@\n-const MIN_X: u8 = 3;\n+const MIN_X: u8 = 4;\n-const MAX_X: u8 = 3;\n+const MAX_X: u8 = 2;\n").is_empty());
    }

    #[test]
    fn review_fixes() {
        // Decoy lines do not hide the real value: all values compared.
        assert_eq!(
            p(
                "ci.toml",
                "@@\n-fail-under = 90\n+fail-under = 95\n+fail-under = 70\n"
            )
            .len(),
            1
        );
        assert!(
            p(
                "a.toml",
                "@@\n-fail-under = 90\n-fail-under = 50\n+fail-under = 50\n"
            )
            .is_empty()
        );
        let c = "@@\n-const MAX_N: u8 = 3;\n+const MAX_N: u8 = 1;\n+const MAX_N: u8 = 9;\n";
        assert_eq!(p("src/a.rs", c).len(), 1);
        // Coverage only in config files; removed only when no added value.
        assert!(p("src/a.rs", "@@\n-fail-under = 90\n").is_empty());
        assert!(p("README.md", "@@\n-fail-under = 90\n").is_empty());
        assert!(
            p(
                "clippy.toml",
                "@@\n-cognitive-complexity-threshold = 9\n+cognitive-complexity-threshold = 30\n"
            )
            .is_empty()
        );
        let w = ".github/workflows/ci.yml";
        for v in ["true", "\"true\"", "'true'", "${{ true }}", "true # why"] {
            assert_eq!(
                p(w, &format!("@@\n+  continue-on-error: {v}\n")).len(),
                1,
                "{v}"
            );
        }
        for v in ["false", "${{false}}", "${{ !true }}", "${{ 0 == 1 }}"] {
            assert_eq!(p(w, &format!("@@\n+  if: {v}\n")).len(), 1, "{v}");
        }
        assert!(
            p(
                w,
                "@@\n+  if: github.ref == 'x'\n+  continue-on-error: false\n"
            )
            .is_empty()
        );
        // Rust lint attributes.
        assert_eq!(p("src/a.rs", "@@\n+#[expect(clippy::x)]\n").len(), 1);
        assert_eq!(p("src/a.rs", "@@\n+#![allow(unused)]\n").len(), 1);
        assert_eq!(
            p("src/lib.rs", "@@\n-#![forbid(unsafe_code)]\n"),
            ["src/lib.rs: #![forbid/deny] 1 → 0"]
        );
        assert!(p("src/lib.rs", "@@\n-#![deny(x)]\n+#![forbid(x)]\n").is_empty());
        // eslint flat config and warn.
        assert_eq!(p("eslint.config.mjs", "@@\n+  'a': 'off',\n").len(), 1);
        assert_eq!(
            p(
                ".eslintrc.json",
                "@@\n-\"a\": \"error\"\n+\"a\": \"warn\"\n"
            )
            .len(),
            1
        );
        assert_eq!(p("a.mts", "@@\n+// oxlint-disable-next-line\n").len(), 1);
        // Only headers before the first hunk are skipped.
        assert_eq!(split_patch("@@ -1 +1 @@\n--x\n++y").0, ["-x"]);
        assert_eq!(split_patch("--- a\n+++ b\n@@\n-x").1.len(), 0);
        // A step moved to another workflow is not removed; names and comments are no steps.
        let gone = "@@\n-      - run: cargo test\n-      # cargo fmt\n-      - name: cargo build\n";
        let moved = vec!["cargo test".to_string()];
        assert!(patch_findings(w, gone, &moved).is_empty());
        assert_eq!(patch_findings(w, gone, &[]).len(), 1);
    }
}
