//! The gate jobs run the bosun release pinned in `.bosun/version`, and its config parser rejects
//! unknown keys. So bosun's own `.bosun/bosun.toml` may only use tables that release knows: a
//! newer key breaks every gate on main until a release that reads it is pinned.

/// The top-level keys each pinned release accepts. Add the new release when pinning it.
const KNOWN: &[(&str, &[&str])] = &[("0.2.3", &["project", "stack", "review", "slack", "prices"])];

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
    let Some((_, keys)) = KNOWN.iter().find(|(v, _)| *v == version) else {
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
