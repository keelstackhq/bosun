mod common;

use common::bosun;
use common::fake::FakeGithub;
use common::one_object;

/// A command whose GitHub calls keep failing with 5xx stops at the budget with an error and a
/// fix, instead of retrying and sleeping on for minutes (heal, wave tick and metrics hung).
#[test]
fn calls_past_the_budget_fail_fast_with_a_fix() {
    let fake = FakeGithub::start();
    fake.fail_path("/pulls", 503);
    let t = std::time::Instant::now();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_HTTP_BUDGET_SECS", "1")
        .args(["metrics", "--days", "1"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{v}");
    assert!(t.elapsed().as_secs() < 20);
    assert!(v["error"]["fix"].as_str().unwrap().len() > 5, "{v}");
}
