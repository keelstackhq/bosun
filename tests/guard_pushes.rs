//! `core::guard::pushes`: where each allowed `git push` runs and what it pushes (push-gates,
//! spec §7), and the push words the merge-lock reads.

use bosun::core::guard::{Push, Script, pushes, tool_rules};
use serde_json::json;

fn bash(cmd: &str, branch: &str) -> Option<String> {
    tool_rules(
        "Bash",
        &json!({ "command": cmd }),
        &|_| Some(branch.into()),
        "main",
    )
}

fn push(dir: Option<&[&str]>, srcs: &[&str]) -> Push {
    Push {
        dir: dir.map(|d| d.iter().map(|s| s.to_string()).collect()),
        srcs: srcs.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn pushes_names_where_each_push_runs_and_what_it_pushes() {
    let none = |_: &str| None;
    let cases: Vec<(&str, Vec<Push>)> = vec![
        ("git status && cargo test", vec![]),
        ("git push", vec![push(Some(&[]), &[])]),
        (
            "git push -u origin claude/x",
            vec![push(Some(&[]), &["claude/x"])],
        ),
        (
            "git push origin HEAD:claude/x",
            vec![push(Some(&[]), &["HEAD"])],
        ),
        (
            "cd ../w && git push origin",
            vec![push(Some(&["-C", "../w"]), &[])],
        ),
        (
            "git -C ../w push origin y",
            vec![push(Some(&["-C", "../w"]), &["y"])],
        ),
        (
            "cd ../w | true; git push origin x",
            vec![push(None, &["x"])],
        ),
        ("git -C \"$D\" push origin x", vec![push(None, &["x"])]),
        ("git push --force origin x", vec![]),
        ("git push -o ci.skip origin", vec![push(Some(&[]), &[])]),
        (
            "git push --repo origin -o x origin y",
            vec![push(Some(&[]), &["y"])],
        ),
        (
            "python3 -c \"run(['git','push','origin','x'], check=True)\"",
            vec![push(None, &[])],
        ),
    ];
    for (cmd, want) in cases {
        assert_eq!(pushes(cmd, &none, "main"), want, "{cmd}");
    }
    let script = |p: &str| (p == "ship.sh").then(|| Script::Text("git push origin z\n".into()));
    assert_eq!(
        pushes("bash ship.sh", &script, "main"),
        vec![push(Some(&[]), &["z"])]
    );
    // Every push is checked, so an extra one (the outer walk does not parse quotes) only
    // adds a check; the nested `sh -c` push is read where its `cd` moves it.
    let nested = pushes("bash -c 'cd ../w && git push origin y'", &none, "main");
    assert!(
        nested.contains(&push(Some(&["-C", "../w"]), &["y"])),
        "{nested:?}"
    );
    for (cmd, want) in [
        (
            "git push -u origin feat 2>&1 | tail -3",
            push(Some(&[]), &["feat"]),
        ),
        ("git push origin feat > log", push(Some(&[]), &["feat"])),
        (
            "git push origin feat 2>/dev/null",
            push(Some(&[]), &["feat"]),
        ),
        (
            "(cd ../w && git push origin feat)",
            push(Some(&["-C", "../w"]), &["feat"]),
        ),
        (
            "pushd ../w && git push origin",
            push(Some(&["-C", "../w"]), &[]),
        ),
    ] {
        assert_eq!(pushes(cmd, &none, "main"), vec![want], "{cmd}");
    }
    assert_eq!(
        pushes("bash -c 'cd ../w && git push origin y'", &none, "main"),
        vec![push(Some(&["-C", "../w"]), &["y"])]
    );
    for cmd in [
        "# don't forget\ngit push origin feat",
        "cat <<EOF\nit's\nEOF\ngit push origin feat",
        "git commit -m \"fix (part 1\" && git push origin feat",
        "git commit -m 'wip :(' && git push origin feat",
    ] {
        assert!(
            pushes(cmd, &none, "main").contains(&push(Some(&[]), &["feat"])),
            "{cmd}"
        );
    }
    for cmd in [
        "git push origin main>/dev/null",
        "git push origin main>log",
        "git push origin main<x",
    ] {
        assert!(bash(cmd, "feature").is_some(), "{cmd}");
    }
    assert_eq!(
        pushes("git push origin feat>log", &none, "main"),
        vec![push(Some(&[]), &["feat"])]
    );
    assert_eq!(
        pushes("git commit -m \"it's\" && git push origin x", &none, "main"),
        vec![push(Some(&[]), &["x"])]
    );
    for cmd in [
        "git push origin 2>&1",
        "git push origin > f",
        "popd; git push origin",
    ] {
        assert!(
            bash(cmd, "main").is_some() || cmd.starts_with("popd"),
            "{cmd}"
        );
    }
    assert!(bash("popd; git push origin", "feature").is_some());
    // The code scan of a shell script adds no push beside the shell walk's.
    let moved =
        |p: &str| (p == "ship.sh").then(|| Script::Text("cd ../w\ngit push origin\n".into()));
    assert_eq!(
        pushes("bash ship.sh", &moved, "main"),
        vec![push(Some(&["-C", "../w"]), &[])]
    );
    // A flag value is not the remote: a bare push on the default branch stays refused.
    assert!(bash("git push -o ci.skip origin", "main").is_some());
}
