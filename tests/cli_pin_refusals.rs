//! `bosun pin --latest` refusals beyond tests/cli_pin.rs: each exits 1 and writes nothing (spec D35).

mod common;

use common::{bosun, one_object};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn sha(bytes: &[u8]) -> String {
    bosun::github::b64::encode(ring::digest::digest(&ring::digest::SHA512, bytes).as_ref())
}

type Routes = HashMap<String, (u16, Vec<u8>)>;

/// A fake npm: every tool at its pin except hoist at 0.4.1. `edit` changes the routes.
fn registry(edit: impl FnOnce(&str, &mut Routes)) -> (String, Arc<Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!(
        "http://127.0.0.1:{}",
        server.server_addr().to_ip().unwrap().port()
    );
    let mut routes = Routes::new();
    for (tool, ver) in [
        ("splice", "0.4.0"),
        ("hoist", "0.4.1"),
        ("berth", "0.1.0"),
        ("bosun", "0.9.0"),
    ] {
        for p in [
            tool.to_string(),
            format!("{tool}-linux-x64"),
            format!("{tool}-linux-arm64"),
        ] {
            let tar = format!("{p}@{ver}").into_bytes();
            let path = format!("/@keelstackhq/{p}/-/{p}-{ver}.tgz");
            let meta = json!({"version": ver, "dist": {"integrity": format!("sha512-{}", sha(&tar)), "tarball": format!("{base}{path}")}});
            for r in ["latest", ver] {
                routes.insert(
                    format!("/@keelstackhq/{p}/{r}"),
                    (200, meta.to_string().into_bytes()),
                );
            }
            routes.insert(path, (200, tar));
        }
    }
    edit(&base, &mut routes);
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let url = req.url().to_string();
            seen.lock().unwrap().push(url.clone());
            let (code, body) = routes.get(&url).cloned().unwrap_or((404, b"no".to_vec()));
            let mut resp = tiny_http::Response::from_data(body).with_status_code(code);
            if code == 302 {
                resp.add_header(
                    tiny_http::Header::from_bytes("Location", "http://evil.example/x").unwrap(),
                );
            }
            let _ = req.respond(resp);
        }
    });
    (base, log)
}

fn repo(script_extra: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let mut s = String::new();
    for (t, v) in [
        ("splice", "0.4.0"),
        ("hoist", "0.4.0"),
        ("berth", "0.1.0"),
        ("bosun", "0.9.0"),
    ] {
        let u = t.to_uppercase();
        s += &format!(
            "{u}_VERSION={v}\n{u}_SHA512=old\n{u}_LINUX_X64_SHA512=old\n{u}_LINUX_ARM64_SHA512=old\n"
        );
    }
    s += script_extra;
    std::fs::create_dir_all(d.path().join("docs")).unwrap();
    std::fs::write(d.path().join("docs/setup-script.sh"), s).unwrap();
    d
}

fn pin(d: &Path, base: &str, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", base)
        .args(["-C", d.to_str().unwrap(), "pin", "--latest"])
        .args(extra)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn refuses(d: &Path, base: &str, needle: &str) {
    let before = std::fs::read_to_string(d.join("docs/setup-script.sh")).unwrap();
    let (code, v) = pin(d, base, &[]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains(needle),
        "{v}"
    );
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(d.join("docs/setup-script.sh")).unwrap(),
        before
    );
}

fn set_meta(r: &mut Routes, key: &str, f: impl Fn(&mut Value)) {
    let (_, body) = r.get_mut(key).unwrap();
    let mut v: Value = serde_json::from_slice(body).unwrap();
    f(&mut v);
    *body = v.to_string().into_bytes();
}

#[test]
fn an_integrity_that_is_not_sha512_is_refused() {
    let (base, _) = registry(|_, r| {
        set_meta(r, "/@keelstackhq/hoist-linux-x64/0.4.1", |v| {
            v["dist"]["integrity"] = json!("sha1-abc")
        })
    });
    refuses(repo("").path(), &base, "not sha512-");
}

#[test]
fn a_latest_that_is_not_x_y_z_is_refused() {
    let (base, _) = registry(|_, r| {
        set_meta(r, "/@keelstackhq/hoist/latest", |v| {
            v["version"] = json!("0.5.0-rc.1")
        })
    });
    refuses(repo("").path(), &base, "not X.Y.Z");
}

#[test]
fn a_binary_package_answering_another_version_is_refused() {
    let (base, _) = registry(|_, r| {
        set_meta(r, "/@keelstackhq/hoist-linux-arm64/0.4.1", |v| {
            v["version"] = json!("0.4.0")
        })
    });
    refuses(repo("").path(), &base, "answered version 0.4.0");
}

#[test]
fn a_redirect_is_not_followed() {
    let (base, log) = registry(|_, r| {
        r.insert("/@keelstackhq/hoist/latest".into(), (302, Vec::new()));
    });
    refuses(repo("").path(), &base, "HTTP 302");
    assert!(log.lock().unwrap().iter().all(|u| !u.contains("evil")));
}

#[test]
fn a_pin_line_twice_is_refused() {
    let (base, _) = registry(|_, _| {});
    refuses(repo("HOIST_VERSION=0.4.0\n").path(), &base, "exactly once");
}

#[test]
fn an_unreadable_template_is_refused() {
    let (base, _) = registry(|_, _| {});
    let d = repo("");
    std::fs::create_dir_all(
        d.path()
            .join("channel/pieces/core/files/docs/setup-script.sh.jinja"),
    )
    .unwrap();
    refuses(d.path(), &base, "cannot read");
}

#[test]
fn tool_filter_asks_npm_only_for_that_tool() {
    let (base, log) = registry(|_, _| {});
    let d = repo("");
    let (code, v) = pin(d.path(), &base, &["--tool", "splice", "--check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["unchanged"], json!(["splice"]));
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|u| u.starts_with("/@keelstackhq/splice/")),
        "{:?}",
        log.lock().unwrap()
    );
}
