use bosun::core::plan;
use bosun::core::prereqs as pq;
use bosun::core::risk::Rules;

const PLAN: &str = "# Plan: x\n\nproject: x\n\n\
## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n\
- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n  depends: T1\n- T3: c\n  touch: docs/a.md\n\
### Done when\n- UC-M1-1: works\n\n\
## M2: Two\n\narea: core\ndepends: M1\n\n### Tasks\n- T1: d\n  touch: README.md\n  risk: high\n\
### Done when\n- UC-M2-1: works\n";

fn plan_of(extra: &str) -> (String, plan::Plan) {
    let t = format!("{PLAN}{extra}");
    let p = plan::parse(&t).expect("plan parses");
    (t, p)
}

fn rules() -> Rules {
    Rules::parse(
        "[[rule]]\nid = \"R-docs\"\nwhy = \"d\"\npaths = [\"docs/a.md\"]\napproval = \"delegate\"\n\
         [[rule]]\nid = \"R-ci\"\nwhy = \"c\"\npaths = [\".github/**\"]\napproval = \"delegate\"\n\
         [[rule]]\nid = \"R-tv\"\nwhy = \"t\"\npaths = [\"assets/b.md\"]\napproval = \"two-verdicts\"\n",
    )
    .unwrap()
}

#[test]
fn approvals_classify_and_batch() {
    let (_, p) = plan_of("");
    let a = pq::approvals(&p, &rules(), true);
    let got: Vec<_> = a
        .iter()
        .map(|a| (a.task.as_str(), a.needs.as_str(), a.batch))
        .collect();
    // M1/T1 and M1/T3 hit R-docs (delegate); M1/T2 touches src/** on the floor but matches no
    // rule, so risk-check needs nothing; M2/T1 is risk high and depends on all M1 tasks.
    let want = [
        ("M1/T1", "delegate", 1),
        ("M1/T3", "delegate", 1),
        ("M2/T1", "owner", 2),
    ];
    assert_eq!(got, want);
    assert_eq!(a[2].why, ["risk: high"]);
    assert_eq!(pq::approvals(&p, &rules(), false), a);
}

#[test]
fn critical_paths_beat_a_delegate_rule_and_no_hit_is_absent() {
    let t = "# P\n\n## M1: One\n\narea: core\n\n### Tasks\n- T1: a\n  touch: .github/workflows/ci.yml\n- T2: b\n  touch: README.md\n- T3: c\n  touch: assets/b.md\n- T4: d\n  touch: README.md\n  risk: high\n### Done when\n- UC-M1-1: x\n";
    let a = pq::approvals(&plan::parse(t).unwrap(), &rules(), false);
    let got: Vec<_> = a
        .iter()
        .map(|a| (a.task.as_str(), a.needs.as_str()))
        .collect();
    assert_eq!(got, [("M1/T1", "owner"), ("M1/T4", "owner")]);
    assert_eq!(a[0].why, ["R-ci", "critical: .github/workflows/ci.yml"]);
    assert_eq!((a[0].batch, a[1].batch), (1, 1));
}

#[test]
fn mixes_follow_risk_check() {
    let rules = Rules::parse(&format!(
        "{}[[rule]]\nid = \"R-own\"\nwhy = \"o\"\npaths = [\"own/**\"]\n",
        "[[rule]]\nid = \"R-d\"\nwhy = \"d\"\npaths = [\"d/**\"]\napproval = \"delegate\"\n\
         [[rule]]\nid = \"R-tv\"\nwhy = \"t\"\npaths = [\"tv/**\"]\napproval = \"two-verdicts\"\n"
    ))
    .unwrap();
    let task = |n: u8, touch: &str, risk: &str| format!("- T{n}: t\n  touch: {touch}\n{risk}");
    let tasks = [
        task(1, "own/a", ""),
        task(2, "d/a, tv/a", ""),
        task(3, "d/a", "  risk: high\n"),
        task(4, "tv/a", "  risk: high\n"),
        task(5, "tv/a", ""),
        task(6, "CODEOWNERS", ""),
        task(7, "d/a, CODEOWNERS", ""),
    ];
    let t = format!(
        "# P\n\n## M1: One\n\narea: a\n\n### Tasks\n{}### Done when\n- UC-M1-1: x\n",
        tasks.concat()
    );
    let a = pq::approvals(&plan::parse(&t).unwrap(), &rules, false);
    let got: Vec<_> = a
        .iter()
        .map(|a| (a.task.as_str(), a.needs.as_str()))
        .collect();
    let owner = |t| (t, "owner");
    let want = [
        owner("M1/T1"),
        owner("M1/T2"),
        owner("M1/T3"),
        owner("M1/T4"),
        owner("M1/T7"),
    ];
    assert_eq!(got, want);
}

#[test]
fn unreadable_touch_and_critical_rules_need_the_owner() {
    let rules = Rules::parse(
        "[[rule]]\nid = \"R-secrets\"\nwhy = \"s\"\npaths = [\"keys/**\"]\napproval = \"delegate\"\n",
    )
    .unwrap();
    let task = |n: u8, touch: &str| format!("- T{n}: t\n  touch: {touch}\n");
    let tasks = [task(1, "x"), task(2, "keys/a"), task(3, "docs/a.md")].concat();
    let t =
        format!("# P\n\n## M1: One\n\narea: a\n\n### Tasks\n{tasks}### Done when\n- UC-M1-1: x\n");
    // plan::parse refuses `..`; approvals() fails closed on its own too.
    let mut p = plan::parse(&t).unwrap();
    p.items[0].tasks[0].touch = vec!["../out/x".into()];
    let a = pq::approvals(&p, &rules, false);
    let got: Vec<_> = a
        .iter()
        .map(|a| (a.task.as_str(), a.needs.as_str()))
        .collect();
    assert_eq!(got, [("M1/T1", "owner"), ("M1/T2", "owner")]);
    assert_eq!(a[0].why, ["unreadable touch: ../out/x"]);
}

#[test]
fn touch_globs_match_rules_and_the_floor_case_insensitively() {
    let task = |n: u8, touch: &str| format!("- T{n}: t\n  touch: {touch}\n");
    let tasks = [task(1, ".GITHUB/workflows/x"), task(2, "DOCS/A.md")].concat();
    let t =
        format!("# P\n\n## M1: One\n\narea: a\n\n### Tasks\n{tasks}### Done when\n- UC-M1-1: x\n");
    let a = pq::approvals(&plan::parse(&t).unwrap(), &rules(), false);
    let got: Vec<_> = a
        .iter()
        .map(|a| (a.task.as_str(), a.needs.as_str()))
        .collect();
    assert_eq!(got, [("M1/T1", "owner"), ("M1/T2", "delegate")]);
}
