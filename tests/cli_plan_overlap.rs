mod common;

fn plan_text(m1_t2_touch: &str, m1_t2_depends: &str, m2_depends: &str) -> String {
    format!(
        "# Plan: widget\n\nproject: widget\n\n\
## M1: One\n\narea: one\n\n### Tasks\n\n\
- T1: First\n  touch: src/a.rs, Cargo.toml\n\
- T2: Second\n  touch: {m1_t2_touch}\n{m1_t2_depends}\n\
### Done when\n\n- UC-M1-1: works.\n\n\
## M2: Two\n\narea: two\n{m2_depends}\n### Tasks\n\n\
- T1: Third\n  touch: src/**\n\n\
### Done when\n\n- UC-M2-1: works.\n"
    )
}

fn one_item(tasks: &str) -> String {
    format!(
        "# Plan: w\n\n## M1: One\n\narea: one\n\n### Tasks\n\n{tasks}\n\
### Done when\n\n- UC-M1-1: ok.\n"
    )
}

/// `n` tasks on one file, in a chain or all parallel.
fn tasks_on_mod(n: usize, chained: bool) -> String {
    let dep = |k: usize| match (chained, k) {
        (true, 2..) => format!("  depends: T{}\n", k - 1),
        _ => String::new(),
    };
    (1..=n)
        .map(|k| format!("- T{k}: Task\n  touch: src/cli/mod.rs\n{}", dep(k)))
        .collect()
}

/// The messages of the warnings of `text` that contain `needle`; each fix is non-empty.
fn warned(text: &str, needle: &str) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plan.md"), text).unwrap();
    let d = dir.path().to_str().unwrap();
    let out = common::bosun()
        .args(["-C", d, "plan", "check", "plan.md"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(v["ok"], true, "{v}");
    let all = v["data"]["warnings"].as_array().unwrap();
    all.iter()
        .filter(|x| x["message"].as_str().unwrap().contains(needle))
        .map(|x| {
            assert!(!x["fix"].as_str().unwrap().is_empty());
            x["message"].as_str().unwrap().to_string()
        })
        .collect()
}

const OVERLAP: &str = "no depends between";

#[test]
fn a_depends_chain_removes_the_warning() {
    let o = warned(&plan_text("src/b.rs", "", ""), OVERLAP);
    assert!(
        o.iter()
            .any(|m| m.contains("M1/T1 and M2/T1 touch overlapping paths (src/a.rs ~ src/**)")),
        "{o:?}"
    );
    // M1/T2 depends on M1/T1, but M2/T1 still overlaps both; an item depends covers both.
    let chain = "  depends: T1\n";
    assert_eq!(warned(&plan_text("src/b.rs", chain, ""), OVERLAP).len(), 2);
    let w = warned(&plan_text("src/b.rs", chain, "depends: M1\n"), OVERLAP);
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn a_hotspot_of_three_parallel_tasks_warns_once() {
    let h = warned(&one_item(&tasks_on_mod(3, false)), "touched by");
    assert_eq!(h.len(), 1, "{h:?}");
    assert!(h[0].starts_with("src/cli/mod.rs is touched by 3 tasks"));
}

#[test]
fn chained_tasks_are_no_hotspot() {
    let text = one_item(&tasks_on_mod(3, true));
    assert!(warned(&text, "touched by").is_empty() && warned(&text, OVERLAP).is_empty());
}

#[test]
fn overlap_warnings_are_capped_at_twenty() {
    // 7 parallel tasks make 21 pairs.
    let text = one_item(&tasks_on_mod(7, false));
    assert_eq!(warned(&text, OVERLAP).len(), 20);
    assert_eq!(
        warned(&text, "more overlapping pair"),
        ["1 more overlapping pair"]
    );
}

#[test]
fn a_huge_plan_stops_the_scan_and_says_so() {
    let tasks: String = (1..=300)
        .map(|k| format!("- T{k}: Task\n  touch: src/**\n"))
        .collect();
    let t = std::time::Instant::now();
    let m = warned(&one_item(&tasks), "more overlapping pair");
    assert_eq!(m, ["at least 980 more overlapping pairs"]);
    assert!(t.elapsed().as_secs_f32() < 1.0, "{:?}", t.elapsed());
}

#[test]
fn an_invalid_touch_glob_is_a_plan_error() {
    let dir = tempfile::tempdir().unwrap();
    let text = one_item("- T1: Task\n  touch: src/[ab].rs\n");
    std::fs::write(dir.path().join("plan.md"), text).unwrap();
    let d = dir.path().to_str().unwrap();
    let out = common::bosun()
        .args(["-C", d, "plan", "check", "plan.md"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v = common::one_object(&out.stdout).to_string();
    assert!(v.contains("src/[ab].rs"), "{v}");
    assert!(v.contains("\"line\":9"), "{v}");
    assert!(
        v.contains("use repo-relative globs with * ? ** only, such as src/core/**"),
        "{v}"
    );
}
