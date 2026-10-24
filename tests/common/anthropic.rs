//! A tiny fake Anthropic Messages API on 127.0.0.1 (tiny_http). It records every request
//! (path, headers, body) and answers with one canned reply or status.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One recorded request; header names are lowercase.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone)]
enum Canned {
    Ok {
        text: String,
        input: u64,
        output: u64,
    },
    Status(u16, String),
}

pub struct FakeAnthropic {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    canned: Arc<Mutex<Canned>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeAnthropic {
    /// Answers 200 with `text` and usage 1000 in / 200 out until told otherwise.
    pub fn start(text: &str) -> FakeAnthropic {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind fake anthropic");
        let url = format!("http://{}", server.server_addr().to_ip().expect("ip"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let canned = Arc::new(Mutex::new(Canned::Ok {
            text: text.into(),
            input: 1000,
            output: 200,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (seen, canned, stop) = (seen.clone(), canned.clone(), stop.clone());
            std::thread::spawn(move || serve(server, seen, canned, stop))
        };
        FakeAnthropic {
            url,
            seen,
            canned,
            stop,
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn reply(&self, text: &str, input: u64, output: u64) {
        *self.canned.lock().unwrap() = Canned::Ok {
            text: text.into(),
            input,
            output,
        };
    }

    /// Answer every request with this status and an API-style error body.
    pub fn fail(&self, status: u16) {
        self.fail_with(status, "fake failure");
    }

    /// Like `fail`, with the error message the API sends.
    pub fn fail_with(&self, status: u16, message: &str) {
        *self.canned.lock().unwrap() = Canned::Status(status, message.into());
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for FakeAnthropic {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve(
    server: tiny_http::Server,
    seen: Arc<Mutex<Vec<Seen>>>,
    canned: Arc<Mutex<Canned>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        let mut req = match server.recv_timeout(Duration::from_millis(25)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(_) => break,
        };
        let mut raw = String::new();
        let _ = std::io::Read::read_to_string(req.as_reader(), &mut raw);
        seen.lock().unwrap().push(Seen {
            method: req.method().as_str().to_string(),
            path: req.url().to_string(),
            headers: req
                .headers()
                .iter()
                .map(|h| {
                    (
                        h.field.as_str().as_str().to_ascii_lowercase(),
                        h.value.as_str().to_string(),
                    )
                })
                .collect(),
            body: serde_json::from_str(&raw).unwrap_or(Value::Null),
        });
        let (status, body) = match canned.lock().unwrap().clone() {
            Canned::Ok {
                text,
                input,
                output,
            } => (
                200,
                json!({"id": "msg_1", "type": "message", "role": "assistant",
                    "content": [{"type": "text", "text": text}],
                    "usage": {"input_tokens": input, "output_tokens": output}}),
            ),
            Canned::Status(code, message) => (
                code,
                json!({"type": "error", "error": {"type": "api_error", "message": message}}),
            ),
        };
        let mut resp = tiny_http::Response::from_string(body.to_string()).with_status_code(status);
        resp.add_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        );
        let _ = req.respond(resp);
    }
}
