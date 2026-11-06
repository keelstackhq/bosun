//! R-weaken (spec §8): findings for a PR that loosens a gate, from gate files at base and head.
//! Pure functions; a finding reads `{file}: {key} {old} → {new}`.

use std::collections::{BTreeMap, HashMap};
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

#[cfg(test)]
mod tests {
    use super::*;

    fn t(path: &str, base: &str, head: &str) -> Vec<String> {
        text_findings(path, Some(base), Some(head))
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
}
