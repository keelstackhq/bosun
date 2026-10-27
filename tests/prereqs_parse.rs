use bosun::core::plan;
use bosun::core::prereqs::{self as pq, Verify};

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

fn errs(section: &str) -> Vec<plan::PlanError> {
    let (t, p) = plan_of(section);
    pq::parse(&t, &p).unwrap_err()
}

#[test]
fn a_section_is_parsed_with_defaults() {
    let (t, p) = plan_of(SECTION);
    let v = pq::parse(&t, &p).unwrap();
    assert_eq!(v.len(), 2);
    let (h1, h2) = (&v[0], &v[1]);
    assert_eq!(
        [&*h1.id, &*h1.who, &*h1.on, &*h2.who],
        ["H1", "owner", "phone", "bob"]
    );
    let v = [h1.verify.describe(), h2.verify.describe()];
    assert_eq!(v, ["secret release/NPM_TOKEN", "owner"]);
    assert_eq!(h1.blocks, ["M1/T2", "M2"]);
    let (t, p) = plan_of(
        "\n## Prerequisites\n- H1: a\n  on: phone\n  steps: s\n  blocks: M1\n  replaces: X2, X1\n",
    );
    assert_eq!(pq::parse(&t, &p).unwrap()[0].replaces, ["X2", "X1"]);
    // The plan parser ignores the section.
    assert_eq!(p.items, plan::parse(PLAN).unwrap().items);
}

#[test]
fn no_section_or_a_fenced_one_gives_nothing() {
    let (t, p) = plan_of("");
    assert!(pq::parse(&t, &p).unwrap().is_empty());
    let (t, p) = plan_of("\n## Notes\n\n```\n## Prerequisites\n- H1: x\n```\n");
    assert!(pq::parse(&t, &p).unwrap().is_empty());
}

#[test]
fn verify_kinds_round_trip() {
    let good = "owner|secret X|secret release/NPM_TOKEN|variable A_1|variable e-1/B|\
                environment release|file a/b.txt|file a b/c.txt";
    for s in good.split('|') {
        assert_eq!(Verify::parse(s).unwrap().describe(), s);
    }
    let bad = "|secret|secret 1x|secret a b/c|environment my env|owner x|environment|file|cmd ls|file ~/.npmrc|\
               file /etc/passwd|file a/../../b|file a\\b|secret GITHUB_X|variable env/github_token";
    for s in bad.split('|') {
        assert!(Verify::parse(s).is_err(), "{s}");
    }
}

#[test]
fn every_error_has_a_line_and_a_fix() {
    // (lines after `## Prerequisites` with OK for three valid fields, error line from it, message)
    let cases = [
        ("- X1: a\nOK", 2, "not `- H"),
        ("- H0: a\nOK", 2, "not `- H"),
        ("* H1: a\nOK", 2, "not `- H"),
        ("- H1: a\nOK- H1: b\nOK", 6, "used twice"),
        ("- H1: a\n  on: phone\n  steps: s\n", 2, "no `blocks:`"),
        ("- H1: a\n  on: phone\n  blocks: M1\n", 2, "no `steps:`"),
        ("- H1: a\n  steps: s\n  blocks: M1\n", 2, "has no `on:`"),
        (
            "- H1: a\n  on: tablet\n  steps: s\n  blocks: M1\n",
            3,
            "not phone or computer",
        ),
        ("- H1: a\nOK  verify: cmd ls\n", 6, "H1: verify"),
        ("- H1: a\nOK  color: red\n", 6, "unknown prerequisite key"),
        ("- H1: a\nOK  on: phone\n", 6, "given twice"),
        ("  on: phone\n", 2, "before any"),
        (
            "- H1: a\n  on: phone\n  steps: s\n  blocks: M9, M1/T9\n",
            5,
            "blocks `M9`",
        ),
        ("- H1: a\nOK## Prerequisites\n", 6, "a second"),
        ("- H1: a\nOK  verify:\n", 6, "empty `verify:`"),
        ("- H1: a\nOK  verify: file a <!-- b\n", 6, "HTML comment"),
        (
            "- H1: a\n  on: phone\n  steps: s\u{2028}x\n  blocks: M1\n",
            4,
            "control",
        ),
        ("- H1: a\nOK  replaces: X1, H2\n", 6, "not an X item"),
        ("- H1: a\nOKverify: secret X\n", 6, "not indented"),
        // A prose line closes the bullet, so the field after it has no bullet.
        (
            "- H1: a <!-- bosun:prereq H1 verify=api ok -->\nOK",
            2,
            "HTML comment",
        ),
        ("- H1: a\nOK  who: x --> y\n", 6, "HTML comment"),
        ("- H1: a\nOK  who: ada lovelace\n", 6, "plain name"),
        (
            "- H1: a\n  on: phone\n  steps: s \u{202e}x\n  blocks: M1\n",
            4,
            "bidi",
        ),
        (
            "- H1: a\n  on: phone\nProse.\n  steps: s\n",
            5,
            "before any",
        ),
    ];
    let ok = "  on: phone\n  steps: s\n  blocks: M1\n";
    let base = PLAN.lines().count() + 2;
    for (body, off, msg) in cases {
        let e = errs(&format!("\n## Prerequisites\n{}", body.replace("OK", ok)));
        let want = base + off - 1;
        let hit = e.iter().any(|e| e.message.contains(msg) && e.line == want);
        assert!(hit, "{msg}: {e:?}");
        assert!(e.iter().all(|e| !e.fix.is_empty()));
    }
    let heads = "## Pre-|## Notes\n### Pre|## Notes\n# pre".split('|');
    for (head, at) in heads.zip([0, 1, 1]) {
        let e = errs(&format!("\n{head}requisites\n- H1: a\n{ok}"));
        let hit = e[0].message.contains("is not `## Prerequisites`") && e[0].line == base + at;
        assert!(hit, "{e:?}");
    }
    let e = errs("\n## Prerequisites\n- H1: a\n  on: x\n  blocks: M9, M1/T9\n");
    assert_eq!(e.len(), 4, "{e:?}");
}

#[test]
fn crlf_tabs_and_the_section_end_are_handled() {
    let section = "\r\n## Prerequisites\r\n- H1: a\r\n\ton: phone\r\n\tsteps: s\r\n\tblocks: M1\r\n\
                   ## Notes\r\n- H2: not a prerequisite\r\n";
    let (t, p) = plan_of(section);
    let v = pq::parse(&t, &p).unwrap();
    assert_eq!((v.len(), &*v[0].on, &*v[0].blocks[0]), (1, "phone", "M1"));
}
