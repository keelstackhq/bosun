//! The `bosun_app` var (spec §15, O5): set, the gate jobs of the channel's workflows run in the
//! `bosun-app` environment with the App token step; empty, the workflows carry no trace of it.

use super::{file, render_channel, yaml_json};
use std::collections::BTreeMap;

/// The gate workflows and their keyed job.
const GATES: [(&str, &str); 4] = [
    (".github/workflows/risk.yml", "risk"),
    (".github/workflows/verdict.yml", "verdict"),
    (".github/workflows/main-open.yml", "judge"),
    (".github/workflows/main-health.yml", "report"),
];

fn render(pieces: &[&str], own_ci: &str, bosun_app: &str) -> Vec<(String, String)> {
    let vars: BTreeMap<&str, String> = [
        ("project", "app"),
        ("owner", ""),
        ("bosun_version", ""),
        ("template.version", "1.2.3"),
        ("rust_path", "."),
        ("ts_path", "."),
        ("own_ci", own_ci),
        ("intake", ""),
        ("policy", ""),
        ("policy_pin_checked", ""),
        ("bosun_app", bosun_app),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect();
    render_channel(pieces, &vars)
}

#[test]
fn bosun_app_puts_the_gate_jobs_on_the_app_token() {
    for pieces in [
        &["core", "rust"][..],
        &["core", "rust", "circleci", "circleci-only"],
    ] {
        for own_ci in ["", "1"] {
            let at = format!("{pieces:?} own_ci={own_ci:?}");
            let files = render(pieces, own_ci, "1");
            for (path, job) in GATES {
                let text = file(&files, path);
                // One token step, its key, and the token only in bosun's env.
                assert_eq!(
                    text.matches("secrets.BOSUN_APP_KEY").count(),
                    1,
                    "{at} {path}"
                );
                assert_eq!(
                    text.matches("steps.app.outputs.token").count(),
                    1,
                    "{at} {path}"
                );
                let Some(j) = yaml_json(text) else { return };
                let job = &j["jobs"][job];
                assert_eq!(job["environment"]["name"], "bosun-app", "{at} {path}");
                assert_eq!(job["environment"]["deployment"], false, "{at} {path}");
                let steps = job["steps"].as_array().unwrap();
                let token = steps
                    .iter()
                    .position(|s| s["id"] == "app")
                    .unwrap_or_else(|| panic!("{at} {path}: no token step"));
                let s = &steps[token];
                assert!(
                    s["uses"].as_str().unwrap().starts_with(
                        "actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510"
                    ),
                    "{at} {path}"
                );
                assert_eq!(s["continue-on-error"], true, "{at} {path}");
                assert_eq!(s["with"]["permission-checks"], "write", "{at} {path}");
                let bosun = steps
                    .iter()
                    .position(|s| s["env"]["BOSUN_APP_TOKEN"].is_string())
                    .unwrap_or_else(|| panic!("{at} {path}: no bosun step with the token"));
                assert!(token < bosun, "{at} {path}: token after its use");
                let run = steps[bosun]["run"].as_str().unwrap();
                assert!(run.contains("bosun "), "{at} {path}: {run}");
            }
        }
    }
}

#[test]
fn without_bosun_app_the_workflows_never_name_the_app() {
    for pieces in [
        &["core", "rust"][..],
        &["core", "rust", "circleci", "circleci-only"],
    ] {
        for own_ci in ["", "1"] {
            let files = render(pieces, own_ci, "");
            for (path, _) in GATES {
                let text = file(&files, path);
                for word in ["bosun-app", "BOSUN_APP", "create-github-app-token"] {
                    assert!(!text.contains(word), "{pieces:?} {own_ci:?} {path}: {word}");
                }
            }
        }
    }
}
