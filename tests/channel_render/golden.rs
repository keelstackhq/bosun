//! Golden snapshots of the four gate workflows the channel renders for a consumer (risk, verdict,
//! main-open, main-health), for fixed input. A template change that alters what consumers get
//! shows up as a diff of these files; `BLESS=1 cargo test --test channel_render golden` rewrites
//! them, and the diff is then reviewed like code.

use super::{file, render_channel};
use std::collections::BTreeMap;
use std::path::Path;

const GATES: [&str; 4] = ["risk", "verdict", "main-open", "main-health"];

fn vars(bosun_app: &str) -> BTreeMap<&'static str, String> {
    [
        ("project", "app"),
        ("owner", "acme"),
        ("bosun_version", "0.13.0"),
        ("template.version", "1.2.3"),
        ("rust_path", "."),
        ("ts_path", "."),
        ("own_ci", ""),
        ("intake", ""),
        ("policy", ""),
        ("policy_pin_checked", ""),
        ("bosun_app", bosun_app),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect()
}

fn check(variant: &str, bosun_app: &str) {
    let files = render_channel(&["core", "rust"], &vars(bosun_app));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/channel_render/golden");
    let bless = std::env::var_os("BLESS").is_some_and(|v| v == "1");
    for name in GATES {
        let got = file(&files, &format!(".github/workflows/{name}.yml"));
        let path = dir.join(format!("{variant}-{name}.yml"));
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, got).unwrap();
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{} is missing; run with BLESS=1", path.display()));
        assert!(
            got == want,
            "{} differs from the rendered workflow; review the change, then run with BLESS=1",
            path.display()
        );
    }
}

#[test]
fn the_gate_workflows_match_their_golden_files() {
    check("plain", "");
}

#[test]
fn the_gate_workflows_with_bosun_app_match_their_golden_files() {
    check("app", "1");
}
