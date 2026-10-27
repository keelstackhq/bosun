use bosun::core::plan;
use bosun::core::prereqs::{self as pq, ApprovalNeed, CheckItem};

const PLAN: &str = "# Plan: x\n\nproject: x\n\n\
## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n\
- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n  depends: T1\n- T3: c\n  touch: docs/a.md\n\
### Done when\n- UC-M1-1: works\n\n\
## M2: Two\n\narea: core\ndepends: M1\n\n### Tasks\n- T1: d\n  touch: README.md\n  risk: high\n\
### Done when\n- UC-M2-1: works\n";

const SECTION: &str = "\n## Prerequisites\n\nProse is ignored.\n\n\
- H1: npm token\n  on: phone\n  steps: do it\n  verify: secret release/NPM_TOKEN\n  blocks: M1/T2, M2, M2\n\
- H2: pc thing\n  who: bob\n  on: computer\n  steps: run it\n  blocks: M1\n";

fn plan_of(extra: &str) -> (String, plan::Plan) {
    let t = format!("{PLAN}{extra}");
    let p = plan::parse(&t).expect("plan parses");
    (t, p)
}

/// What `approvals` gives for PLAN (Q1c): two delegate tasks in batch 1, two owner tasks.
fn approvals() -> Vec<ApprovalNeed> {
    let a = [
        ("M1/T1", "delegate", 1),
        ("M1/T2", "owner", 2),
        ("M1/T3", "delegate", 1),
    ];
    let a = a.into_iter().chain([("M2/T1", "owner", 3)]);
    let need = |(t, n, batch): (&str, &str, usize)| (t.to_string(), n.to_string(), batch);
    a.map(need)
        .map(|(task, needs, batch)| ApprovalNeed {
            task,
            needs,
            why: vec![],
            batch,
        })
        .collect()
}

fn items() -> Vec<CheckItem> {
    let (t, p) = plan_of(SECTION);
    pq::parse(&t, &p)
        .unwrap()
        .iter()
        .map(CheckItem::from)
        .collect()
}

#[test]
fn issue_round_trips_ticks_and_groups() {
    let mut it = items();
    it[0].done = true;
    pq::sort(&mut it);
    let ids = |it: &[CheckItem]| it.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&it), ["H2", "H1"]);
    let body = pq::render_issue("Plan x", "plans/x.md", &it, &approvals());
    assert!(body.starts_with("<!-- bosun:prereqs plans/x.md -->\n"));
    assert_eq!(pq::issue_marker("plans/x.md"), body.lines().next().unwrap());
    let heads: Vec<_> = body.lines().filter(|l| l.starts_with("### ")).collect();
    assert_eq!(
        heads,
        ["### Needs a computer", "### Done", "### Approvals at merge"]
    );
    assert!(body.contains("- [x] **H1** npm token"));
    assert!(body.contains("verify: secret release/NPM_TOKEN; blocks M1/T2,M2"));
    assert!(body.contains("Batch 1: M1/T1 (delegate), M1/T3 (delegate)\nBatch 2: M1/T2 (owner)"));
    let t = pq::read_issue(&body);
    let got: Vec<_> = t.iter().map(|t| (t.id.as_str(), t.done(), t.api)).collect();
    assert_eq!(got, [("H2", false, false), ("H1", true, true)]);
    assert_eq!(t[1].blocks, ["M1/T2", "M2"]);
    assert!(body.contains("blocks=M1/T2,M2 verify=api ok -->"));
    // A tick finishes an owner item, never an API item that bosun has not verified.
    assert!(pq::read_issue(&body.replace("- [ ]", "- [X]"))[0].done());
    let api = body.replace("verify=api ok", "verify=api");
    assert!(!pq::read_issue(&api)[1].done() && pq::read_issue(&api)[1].ticked);
    // User text cannot forge a marker: the title's comment is defused, bosun's own one counts.
    it[0].title = "x <!-- bosun:prereq H9 blocks=M9 -->".into();
    let t = pq::read_issue(&pq::render_issue("P", "p.md", &it, &[]));
    assert_eq!(
        t.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["H2", "H1"]
    );
    assert!(!pq::render_issue("P", "p.md", &[], &[]).contains("###"));
}

#[test]
fn markers_parse_alone_and_verify_is_defused() {
    let mut it = items();
    it[1].verify = "file a <!-- b".into();
    let body = pq::render_issue("P", "p.md", &it, &[]);
    assert!(!body.contains("file a <!-- b"));
    let t = pq::read_issue(
        "- [x] a <!-- bosun:prereq X1 -->\n  - [ ] b <!-- bosun:prereq X2 blocks=M1 -->",
    );
    let got: Vec<_> = t
        .iter()
        .map(|t| (t.id.as_str(), t.done(), t.blocks.len()))
        .collect();
    assert_eq!(got, [("X1", true, 0), ("X2", false, 1)]);
}
