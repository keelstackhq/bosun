mod common;

use common::{bosun, one_object};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn b64(bytes: &[u8]) -> String {
    bosun::github::b64::encode(bytes)
}

fn sha(bytes: &[u8]) -> String {
    b64(ring::digest::digest(&ring::digest::SHA512, bytes).as_ref())
}

/// The three npm packages of a tool.
fn pkgs(tool: &str) -> [String; 3] {
    [
        tool.into(),
        format!("{tool}-linux-x64"),
        format!("{tool}-linux-arm64"),
    ]
}

/// A fake npm registry; `log` holds every requested path.
struct Registry {
    base: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl Registry {
    /// `latest`: tool -> version. `tamper`: serve other bytes than the integrity covers for this package.
    /// `foreign`: this package's tarball URL points at another host.
    fn start(latest: &[(&str, &str)], tamper: Option<&str>, foreign: Option<&str>) -> Registry {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!(
            "http://127.0.0.1:{}",
            server.server_addr().to_ip().unwrap().port()
        );
        let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
        for (tool, ver) in latest {
            for p in pkgs(tool) {
                let tar = format!("{p}@{ver}").into_bytes();
                let host = if foreign == Some(p.as_str()) {
                    "http://evil.example"
                } else {
                    base.as_str()
                };
                let tarball = format!("{host}/@keelstackhq/{p}/-/{p}-{ver}.tgz");
                let meta = json!({"version": ver, "dist": {"integrity": format!("sha512-{}", sha(&tar)), "tarball": tarball}});
                let served = if tamper == Some(p.as_str()) {
                    b"other bytes".to_vec()
                } else {
                    tar
                };
                routes.insert(
                    format!("/@keelstackhq/{p}/latest"),
                    meta.to_string().into_bytes(),
                );
                routes.insert(
                    format!("/@keelstackhq/{p}/{ver}"),
                    meta.to_string().into_bytes(),
                );
                routes.insert(format!("/@keelstackhq/{p}/-/{p}-{ver}.tgz"), served);
            }
        }
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let url = req.url().to_string();
                seen.lock().unwrap().push(url.clone());
                let _ = match routes.get(&url) {
                    Some(b) => req.respond(tiny_http::Response::from_data(b.clone())),
                    None => {
                        req.respond(tiny_http::Response::from_string("no").with_status_code(404))
                    }
                };
            }
        });
        Registry { base, log }
    }

    fn tarballs(&self) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.contains("/-/"))
            .count()
    }
}

const PINS: [(&str, &str); 4] = [
    ("splice", "0.4.0"),
    ("hoist", "0.4.0"),
    ("berth", "0.1.0"),
    ("bosun", "0.9.0"),
];

fn lines(tool: &str, ver: &str) -> String {
    let u = tool.to_uppercase();
    format!(
        "{u}_VERSION={ver}\n{u}_SHA512=old\n{u}_LINUX_X64_SHA512=old\n{u}_LINUX_ARM64_SHA512=old\n"
    )
}

/// A repo with the setup script, .bosun/version and the channel template.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let mut script = String::from("NODE_MAJOR=24\n");
    let mut tpl = script.clone();
    for (t, v) in PINS {
        script += &lines(t, v);
        if t != "bosun" {
            tpl += &lines(t, v);
        }
    }
    tpl += "BOSUN_VERSION={% if bosun_version %}x{% endif %}\n";
    for (rel, text) in [
        ("docs/setup-script.sh", script.as_str()),
        (
            "channel/pieces/core/files/docs/setup-script.sh.jinja",
            tpl.as_str(),
        ),
        (".bosun/version", "0.9.0\n"),
    ] {
        let p = d.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    d
}

fn read(d: &Path, rel: &str) -> String {
    std::fs::read_to_string(d.join(rel)).unwrap()
}

fn snapshot(d: &Path) -> Vec<String> {
    [
        "docs/setup-script.sh",
        "channel/pieces/core/files/docs/setup-script.sh.jinja",
        ".bosun/version",
    ]
    .iter()
    .map(|r| read(d, r))
    .collect()
}

fn pin(d: &Path, reg: &Registry, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", &reg.base)
        .args(["-C", d.to_str().unwrap(), "pin", "--latest"])
        .args(extra)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn same_as_pins() -> Vec<(&'static str, &'static str)> {
    PINS.to_vec()
}

#[test]
fn bump_rewrites_the_script_the_version_file_and_the_template() {
    let d = repo();
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.1"),
        ],
        None,
        None,
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 0, "{v}");
    let data = &v["data"];
    assert_eq!(
        data["changed"],
        json!([{"tool": "hoist", "from": "0.4.0", "to": "0.4.1"}, {"tool": "bosun", "from": "0.9.0", "to": "0.9.1"}])
    );
    assert_eq!(data["unchanged"], json!(["splice", "berth"]));
    assert_eq!(data["branch"], "bosun/pin");
    assert_eq!(data["title"], "chore: pin hoist 0.4.1, bosun 0.9.1");
    let script = read(d.path(), "docs/setup-script.sh");
    let tar = |p: &str, ver: &str| sha(format!("{p}@{ver}").as_bytes());
    for (suffix, p) in [
        ("", "hoist"),
        ("LINUX_X64_", "hoist-linux-x64"),
        ("LINUX_ARM64_", "hoist-linux-arm64"),
    ] {
        assert!(
            script.contains(&format!("HOIST_{suffix}SHA512={}\n", tar(p, "0.4.1"))),
            "{suffix}"
        );
    }
    assert!(script.contains("HOIST_VERSION=0.4.1\n") && script.contains("BOSUN_VERSION=0.9.1\n"));
    assert!(script.contains("SPLICE_VERSION=0.4.0\nSPLICE_SHA512=old\n"));
    assert_eq!(read(d.path(), ".bosun/version"), "0.9.1\n");
    let tpl = read(
        d.path(),
        "channel/pieces/core/files/docs/setup-script.sh.jinja",
    );
    assert!(tpl.contains("HOIST_VERSION=0.4.1\n") && tpl.contains("SPLICE_VERSION=0.4.0\n"));
    assert!(tpl.contains("BOSUN_VERSION={% if bosun_version %}x{% endif %}\n"));
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 3, "{files:?}");
}

#[test]
fn equal_versions_change_nothing() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(&same_as_pins(), None, None);
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], json!([]));
    assert_eq!(
        v["data"]["unchanged"],
        json!(["splice", "hoist", "berth", "bosun"])
    );
    assert_eq!(snapshot(d.path()), before);
    assert_eq!(reg.tarballs(), 0);
}

#[test]
fn a_downgrade_is_a_conflict_and_writes_nothing() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.5.0"),
            ("hoist", "0.3.9"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        None,
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("hoist"));
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn a_hash_mismatch_is_refused_and_writes_nothing() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.4.1"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        Some("hoist-linux-arm64"),
        None,
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("hoist-linux-arm64") && msg.contains("0.4.1"),
        "{msg}"
    );
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn a_tarball_outside_the_registry_is_refused() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        Some("hoist-linux-x64"),
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hoist-linux-x64")
    );
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn a_channel_consumer_is_sent_to_splice_update() {
    let d = repo();
    std::fs::create_dir_all(d.path().join(".splice/channels")).unwrap();
    std::fs::write(d.path().join(".splice/channels/bosun.toml"), "x = 1\n").unwrap();
    let before = snapshot(d.path());
    let reg = Registry::start(&[("hoist", "0.4.1")], None, None);
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("splice update --channel bosun")
    );
    assert_eq!(snapshot(d.path()), before);
    assert!(reg.log.lock().unwrap().is_empty());
}

#[test]
fn check_lists_what_is_behind_without_tarballs_or_writes() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        None,
    );
    let (code, v) = pin(d.path(), &reg, &["--check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["behind"],
        json!([{"tool": "hoist", "from": "0.4.0", "to": "0.4.1"}])
    );
    assert_eq!(snapshot(d.path()), before);
    assert_eq!(reg.tarballs(), 0);
}
