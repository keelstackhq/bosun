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
