//! The `keelstack` plugin marketplace pins each plugin source to a commit (session defaults, R3).

use serde_json::Value;

#[test]
fn marketplace_pins_superpowers_and_caveman_to_commits() {
    let text = std::fs::read_to_string(".claude-plugin/marketplace.json").unwrap();
    let m: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(m["name"], "keelstack");
    let plugins = m["plugins"].as_array().unwrap();
    let mut names: Vec<&str> = plugins
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["caveman", "superpowers"]);
    for p in plugins {
        let src = &p["source"];
        assert_eq!(src["source"], "github", "{}", p["name"]);
        let sha = src["sha"].as_str().unwrap_or_default();
        assert!(
            sha.len() == 40
                && sha
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{} needs a full commit sha, got {sha:?}",
            p["name"]
        );
        assert!(src.get("ref").is_none(), "a ref would float: {}", p["name"]);
    }
}

/// Both settings files enable the plugins from the `keelstack` marketplace, from bosun's main branch.
#[test]
fn settings_pin_the_keelstack_marketplace() {
    let root = std::fs::read_to_string(".claude/settings.json").unwrap();
    let channel =
        std::fs::read_to_string("channel/pieces/core/files/.claude/settings.json").unwrap();
    let mut refs = Vec::new();
    for text in [&root, &channel] {
        let s: Value = serde_json::from_str(text).unwrap();
        let src = &s["extraKnownMarketplaces"]["keelstack"]["source"];
        assert_eq!(src["source"], "github");
        assert_eq!(src["repo"], "keelstackhq/bosun");
        // A branch or tag: Claude Code clones a marketplace with `--branch`, so a sha fails.
        // The plugin commits are pinned by sha inside .claude-plugin/marketplace.json.
        let r = src["ref"].as_str().unwrap_or_default();
        assert_eq!(r, "main");
        refs.push(r.to_string());
        for p in ["superpowers@keelstack", "caveman@keelstack"] {
            assert_eq!(s["enabledPlugins"][p], true, "{p}");
        }
    }
    assert_eq!(
        refs[0], refs[1],
        "root and channel settings pin the same commit"
    );
}
