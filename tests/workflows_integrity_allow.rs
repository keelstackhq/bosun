//! CI reads test-integrity's allowed kinds from the PR's linked task in a step of its own, before
//! the gates, so the token never reaches the step where lint and test run PR code (spec §7).
use std::path::Path;

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

/// The text of the step whose `name:` is `name`, up to the next step at the same indent.
fn step<'a>(yaml: &'a str, name: &str) -> &'a str {
    let at = yaml
        .find(&format!("name: {name}\n"))
        .unwrap_or_else(|| panic!("no step {name}"));
    // A step may start mid-line, after a template tag like `{% raw %}`.
    let start = yaml[..at].rfind("      - ").unwrap();
    let end = yaml[at..].find("\n      - ").map_or(yaml.len(), |i| at + i);
    &yaml[start..end]
}

#[test]
fn the_allow_list_is_read_before_the_gates_and_only_there() {
    for rel in [
        ".github/workflows/ci.yml",
        "channel/pieces/core/files/.github/workflows/bosun.yml.jinja",
    ] {
        let y = read(rel);
        let allow = step(&y, "task allow list");
        let gates = step(&y, "bosun gates");
        assert!(
            y.find(allow).unwrap() < y.find(gates).unwrap(),
            "{rel}: order"
        );
        assert!(allow.contains("id: allow"), "{rel}");
        assert!(allow.contains("GITHUB_TOKEN: ${{ github.token }}"), "{rel}");
        assert!(allow.contains("test-integrity --pr \"$PR\""), "{rel}");
        assert!(
            allow.contains("github.event_name == 'pull_request'"),
            "{rel}"
        );
        assert!(allow.contains("sha256sum -c"), "{rel}: the checked binary");
        // Only the four kinds can come out, whatever the task says.
        assert!(
            allow.contains(r#"select(IN("delete-test","skip","only","fewer-asserts"))"#),
            "{rel}"
        );
        assert!(
            gates.contains("BOSUN_INTEGRITY_ALLOW: ${{ steps.allow.outputs.kinds }}"),
            "{rel}"
        );
        for leak in ["GITHUB_TOKEN", "GH_TOKEN", "github.token", "secrets."] {
            assert!(!gates.contains(leak), "{rel}: {leak} in the gates step");
        }
        let perms = &y[y.find("\npermissions:\n").unwrap()..];
        let perms = &perms[..perms[1..].find("\n\n").unwrap() + 1];
        for p in ["contents: read", "issues: read", "pull-requests: read"] {
            assert!(perms.contains(p), "{rel}: {p}");
        }
        assert!(!perms.contains("write"), "{rel}: read only");
    }
}
