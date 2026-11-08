//! The gate jobs run the bosun release pinned in `.bosun/version`, and its config parser rejects
//! unknown keys. So bosun's own `.bosun/bosun.toml` may only use tables that release knows: a
//! newer key breaks every gate on main until a release that reads it is pinned.

/// The top-level keys each pinned release accepts. Add a release only when it accepts new keys
/// (see `known_for`).
const KNOWN: &[(&str, &[&str])] = &[
    ("0.2.3", &["project", "stack", "review", "slack", "prices"]),
    (
        "0.3.0",
        &["project", "stack", "review", "slack", "prices", "approvals"],
    ),
    (
        "0.4.0",
        &["project", "stack", "review", "slack", "prices", "approvals"],
    ),
    (
        "0.5.0",
        &[
            "project",
            "stack",
            "review",
            "slack",
            "cloud",
            "prices",
            "approvals",
            "intake",
        ],
    ),
    (
        "0.6.0",
        &[
            "project",
            "stack",
            "review",
            "slack",
            "cloud",
            "prices",
            "approvals",
            "intake",
        ],
    ),
    (
        "0.7.0",
        &[
            "project",
            "stack",
            "review",
            "slack",
            "cloud",
            "prices",
            "approvals",
            "intake",
        ],
    ),
    (
        "0.8.0",
        &[
            "project",
            "stack",
            "review",
            "slack",
            "cloud",
            "prices",
            "approvals",
            "intake",
        ],
    ),
    (
        "0.9.0",
        &[
            "project",
            "stack",
            "review",
            "slack",
            "cloud",
            "prices",
            "approvals",
            "intake",
            "trailers",
        ],
    ),
    (
        "0.10.0",
        &[
            "project",
            "stack",
            "review",
            "slack",
            "cloud",
            "prices",
            "approvals",
            "intake",
            "trailers",
        ],
    ),
];

/// The `.bosun/limits.toml` keys (`table.key`) each pinned release accepts.
const KNOWN_LIMITS: &[(&str, &[&str])] = &[
    (
        "0.4.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
    (
        "0.5.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "waves.relay_waves",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
    (
        "0.6.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "waves.relay_waves",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
    (
        "0.7.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "waves.relay_waves",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
    (
        "0.8.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "waves.relay_waves",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
    (
        "0.9.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "waves.relay_waves",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
    (
        "0.10.0",
        &[
            "sessions.max_depth",
            "sessions.max_live_sessions",
            "sessions.max_children",
            "waves.max_parallel",
            "waves.relay_waves",
            "turns.build",
            "turns.build_cap",
            "turns.review",
            "turns.fix_round",
            "models.read",
            "models.build",
            "models.review",
            "models.hard",
            "budget.daily_usd",
            "budget.per_task_usd",
        ],
    ),
];

/// The `table.key` names of a limits file that `known` does not list.
fn unknown_limits(text: &str, known: &[&str]) -> Vec<String> {
    let table: toml::Table = text.parse().expect("limits.toml");
    let mut out = Vec::new();
    for (name, value) in &table {
        match value.as_table() {
            Some(t) => out.extend(
                t.keys()
                    .map(|k| format!("{name}.{k}"))
                    .filter(|k| !known.contains(&k.as_str())),
            ),
            None => out.push(name.clone()),
        }
    }
    out
}

/// The keys of `[approvals]` each pinned release accepts (`policy` from 0.5.0).
const KNOWN_APPROVALS: &[(&str, &[&str])] = &[
    ("0.3.0", &["bootstrap_until"]),
    ("0.4.0", &["bootstrap_until"]),
    ("0.5.0", &["bootstrap_until", "policy"]),
    ("0.6.0", &["bootstrap_until", "policy"]),
    ("0.7.0", &["bootstrap_until", "policy"]),
    ("0.8.0", &["bootstrap_until", "policy"]),
    ("0.9.0", &["bootstrap_until", "policy"]),
    ("0.10.0", &["bootstrap_until", "policy"]),
];

/// The `[approvals] policy` values each pinned release accepts (`autonomous` from 0.8.0).
const KNOWN_POLICIES: &[(&str, &[&str])] = &[
    ("0.5.0", &["owner", "owner-gated", "orchestrator"]),
    ("0.6.0", &["owner", "owner-gated", "orchestrator"]),
    ("0.7.0", &["owner", "owner-gated", "orchestrator"]),
    (
        "0.8.0",
        &["owner", "owner-gated", "orchestrator", "autonomous"],
    ),
    (
        "0.9.0",
        &["owner", "owner-gated", "orchestrator", "autonomous"],
    ),
    (
        "0.10.0",
        &["owner", "owner-gated", "orchestrator", "autonomous"],
    ),
];

/// `major.minor.patch` as numbers, so `0.10.0` sorts after `0.9.0`. A pre-release or `v`-prefixed
/// pin (`0.10.0-rc.1`, `v0.10.0`) panics: the test fails closed.
fn semver(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|p| {
            p.parse()
                .unwrap_or_else(|_| panic!("`{v}` is not an x.y.z version"))
        })
        .collect()
}

/// The entry of `version` in a release table, else of the newest older release: a later release
/// only adds keys (`release_tables_only_grow` checks the listed ones), so the older set is a safe
/// subset and a pin bump needs no new entry. Add one when a release accepts new keys, and when it
/// removes or renames one. Backstop for a removal: `tests/governance.rs` parses the `.bosun`
/// config with the current code, which is the next release. `None` before the first entry.
fn known_for(
    table: &[(&'static str, &'static [&'static str])],
    version: &str,
) -> Option<&'static [&'static str]> {
    (table.iter())
        .filter(|(v, _)| semver(v) <= semver(version))
        .max_by_key(|(v, _)| semver(v))
        .map(|(_, keys)| *keys)
}

/// Whether release `version` accepts `[approvals] policy = value`; `None` before the first entry.
fn policy_known(version: &str, value: &str) -> Option<bool> {
    let values = known_for(KNOWN_POLICIES, version)?;
    Some(values.contains(&value))
}

/// `path` as committed at HEAD: CI swaps the base branch's `.bosun/` into the checkout.
fn committed(path: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["show", &format!("HEAD:{path}")])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git show");
    assert!(out.status.success(), "git show HEAD:{path}");
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn bosun_toml_uses_only_keys_the_pinned_release_knows() {
    let version = committed(".bosun/version").trim().to_string();
    let Some(keys) = known_for(KNOWN, &version) else {
        panic!(
            "add the top-level keys release {version} accepts to KNOWN in tests/pinned_config.rs"
        );
    };
    let config: toml::Table = committed(".bosun/bosun.toml").parse().expect("bosun.toml");
    for key in config.keys() {
        assert!(
            keys.contains(&key.as_str()),
            "`{key}` in .bosun/bosun.toml is unknown to the pinned release {version}: the gates on main would fail; set it in the pin PR of a release that reads it"
        );
    }
}

#[test]
fn limits_toml_uses_only_keys_the_pinned_release_knows() {
    let version = committed(".bosun/version").trim().to_string();
    let Some(keys) = known_for(KNOWN_LIMITS, &version) else {
        panic!(
            "add the limits keys release {version} accepts to KNOWN_LIMITS in tests/pinned_config.rs"
        );
    };
    let unknown = unknown_limits(&committed(".bosun/limits.toml"), keys);
    assert!(
        unknown.is_empty(),
        "{unknown:?} in .bosun/limits.toml are unknown to the pinned release {version}: the gates on main would fail; set them in the pin PR of a release that reads them"
    );
}

#[test]
fn a_key_newer_than_the_pin_is_caught() {
    let known = KNOWN_LIMITS[0].1;
    assert_eq!(
        unknown_limits(
            "[waves]\nmax_parallel = 6\nrelay_waves = 6\n[cloud]\nx = 1\n",
            known
        ),
        ["cloud.x", "waves.relay_waves"]
    );
    assert!(unknown_limits("[waves]\n# relay_waves = 6\nmax_parallel = 6\n", known).is_empty());
}

#[test]
fn approvals_uses_only_keys_the_pinned_release_knows() {
    let version = committed(".bosun/version").trim().to_string();
    let config: toml::Table = committed(".bosun/bosun.toml").parse().expect("bosun.toml");
    let Some(approvals) = config.get("approvals").and_then(|a| a.as_table()) else {
        return;
    };
    let Some(keys) = known_for(KNOWN_APPROVALS, &version) else {
        panic!(
            "add the `[approvals]` keys release {version} accepts to KNOWN_APPROVALS in tests/pinned_config.rs"
        );
    };
    for key in approvals.keys() {
        assert!(
            keys.contains(&key.as_str()),
            "`approvals.{key}` in .bosun/bosun.toml is unknown to the pinned release {version}: the gates on main would fail; set it in the pin PR of a release that reads it"
        );
    }
}

#[test]
fn approvals_policy_uses_only_values_the_pinned_release_knows() {
    let version = committed(".bosun/version").trim().to_string();
    let config: toml::Table = committed(".bosun/bosun.toml").parse().expect("bosun.toml");
    let Some(policy) = config
        .get("approvals")
        .and_then(|a| a.get("policy"))
        .and_then(|p| p.as_str())
    else {
        return;
    };
    let known = policy_known(&version, policy).unwrap_or_else(|| {
        panic!("add the `[approvals] policy` values release {version} accepts to KNOWN_POLICIES in tests/pinned_config.rs")
    });
    assert!(
        known,
        "`approvals.policy = \"{policy}\"` in .bosun/bosun.toml is unknown to the pinned release {version}: the gates on main would fail; set it in the pin PR of a release that reads it"
    );
}

#[test]
fn policy_known_follows_the_release_table() {
    assert_eq!(policy_known("0.7.0", "autonomous"), Some(false));
    assert_eq!(policy_known("0.8.0", "autonomous"), Some(true));
    assert_eq!(policy_known("0.7.0", "owner-gated"), Some(true));
    assert_eq!(policy_known("0.4.0", "owner"), None);
}

#[test]
fn an_unlisted_release_falls_back_to_the_newest_older_entry() {
    // Key sets only grow, so the newest older release is a safe subset: no hand-added entry.
    assert_eq!(known_for(KNOWN, "0.99.0"), known_for(KNOWN, "0.10.0"));
    assert_eq!(
        known_for(KNOWN_LIMITS, "0.9.3"),
        known_for(KNOWN_LIMITS, "0.9.0")
    );
    assert_eq!(policy_known("0.99.0", "autonomous"), Some(true));
    // Numeric order, not string order: 0.99.0 is newer than 0.10.0, 0.2.9 older than 0.3.0.
    assert_eq!(known_for(KNOWN, "0.2.9"), known_for(KNOWN, "0.2.3"));
    assert_eq!(known_for(KNOWN, "0.2.2"), None);
}

#[test]
fn release_tables_only_grow() {
    // The fallback in `known_for` is safe only while each release accepts every key the one
    // before it accepted.
    for (name, table) in [
        ("KNOWN", KNOWN),
        ("KNOWN_LIMITS", KNOWN_LIMITS),
        ("KNOWN_APPROVALS", KNOWN_APPROVALS),
        ("KNOWN_POLICIES", KNOWN_POLICIES),
    ] {
        for pair in table.windows(2) {
            let ((old, old_keys), (new, new_keys)) = (pair[0], pair[1]);
            assert!(
                semver(old) < semver(new),
                "{name}: {old} must come before {new}"
            );
            for key in old_keys {
                assert!(
                    new_keys.contains(key),
                    "{name}: {new} drops `{key}` that {old} accepts; known_for would fall back wrongly"
                );
            }
        }
    }
}
