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

fn script_pin(script: &str, name: &str) -> String {
    script
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("docs/setup-script.sh lacks {name}="))
        .to_string()
}

/// The setup script prewarms the same plugin commits the marketplace pins, and only warns.
#[test]
fn setup_script_pins_match_the_marketplace() {
    let script = std::fs::read_to_string("docs/setup-script.sh").unwrap();
    let m: Value =
        serde_json::from_str(&std::fs::read_to_string(".claude-plugin/marketplace.json").unwrap())
            .unwrap();
    for p in m["plugins"].as_array().unwrap() {
        let var = format!("{}_SHA", p["name"].as_str().unwrap().to_uppercase());
        assert_eq!(
            script_pin(&script, &var),
            p["source"]["sha"].as_str().unwrap(),
            "{var}"
        );
    }
    // A marketplace ref is a branch or tag: Claude Code clones it with `--branch`, so a sha fails.
    assert_eq!(script_pin(&script, "KEELSTACK_MARKETPLACE_REF"), "main");
    assert!(script.contains("https://github.com/keelstackhq/bosun.git#$KEELSTACK_MARKETPLACE_REF"));
    for p in ["superpowers@keelstack", "caveman@keelstack"] {
        assert!(script.contains(p), "{p}");
    }
}
