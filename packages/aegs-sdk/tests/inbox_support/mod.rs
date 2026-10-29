//! Shared pieces of the real-process inbox drills: a fake AEDS, an AEGS child process started
//! from the test binary itself, and small HTTP helpers.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chariox_aegs_sdk::{AegsStore, SubscriptionClaim};

pub const GENERATOR: &str = "dev.chariox.drill";
pub const CHILD_ENV: &str = "CHARIOX_DRILL_CHILD";
pub const MANAGEMENT_TOKEN: &str = "drill-management-token";

#[derive(Clone, Copy, PartialEq)]
pub enum AedsMode {
    /// Accept the connection, read the request and never answer.
    Hang,
    Accept,
}

pub struct FakeAeds {
    pub port: u16,
    hang: Arc<AtomicBool>,
    pub requests_seen: Arc<AtomicUsize>,
    pub accepted: Arc<Mutex<Vec<String>>>,
}

impl FakeAeds {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let aeds = Self {
            port,
            hang: Arc::new(AtomicBool::new(true)),
            requests_seen: Arc::new(AtomicUsize::new(0)),
            accepted: Arc::new(Mutex::new(Vec::new())),
        };
        let (hang, seen, accepted) = (
            Arc::clone(&aeds.hang),
            Arc::clone(&aeds.requests_seen),
            Arc::clone(&aeds.accepted),
        );
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                let (hang, seen, accepted) =
                    (Arc::clone(&hang), Arc::clone(&seen), Arc::clone(&accepted));
                let mut stream = stream;
                let Some(body) = read_request_body(&mut stream) else {
                    continue;
                };
                seen.fetch_add(1, Ordering::SeqCst);
                if hang.load(Ordering::SeqCst) {
                    held.push(stream);
                    continue;
                }
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let occurrence = request["occurrence_id"].as_str().unwrap().to_string();
                let duplicate = {
                    let mut accepted = accepted.lock().unwrap();
                    let duplicate = accepted.contains(&occurrence);
                    if !duplicate {
                        accepted.push(occurrence.clone());
                    }
                    duplicate
                };
                let response = serde_json::json!({
                    "occurrence_id": occurrence,
                    "accepted_route_count": 1,
                    "delivery_ids": ["delivery"],
                    "duplicate": duplicate,
                })
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                    response.len()
                );
            }
        });
        aeds
    }

    pub fn set_mode(&self, mode: AedsMode) {
        self.hang.store(mode == AedsMode::Hang, Ordering::SeqCst);
    }
}

fn read_request_body(stream: &mut TcpStream) -> Option<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(split) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..split]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buffer.len() < split + 4 + length {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        return Some(buffer[split + 4..split + 4 + length].to_vec());
    }
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn spawn_server(child_test: &str, database: &Path, port: u16, aeds_port: u16) -> Child {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", child_test, "--nocapture"])
        .env(CHILD_ENV, "1")
        .env("CHARIOX_AEGS_ADDR", format!("127.0.0.1:{port}"))
        .env("CHARIOX_AEGS_DATABASE_PATH", database)
        .env("CHARIOX_AEGS_MANAGEMENT_TOKEN", MANAGEMENT_TOKEN)
        .env(
            "CHARIOX_AEDS_EVENTS_URL",
            format!("http://127.0.0.1:{aeds_port}/v1/events"),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the AEGS child did not start listening"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    child
}

pub fn seed_subscription(database: &Path) {
    let store = AegsStore::open(database).unwrap();
    store
        .upsert_ready_connection("conn-1", "owner-1", "drill", &serde_json::Value::Null, 1)
        .unwrap();
    store
        .reconcile(
            "kernel-1",
            GENERATOR,
            &[SubscriptionClaim {
                binding_id: "binding-1".to_string(),
                generator_id: GENERATOR.to_string(),
                connection_id: "conn-1".to_string(),
                connection_scope: "scope-1".to_string(),
                event_interest_key: "interest-1".to_string(),
                event_type: "drill.event".to_string(),
                event_type_version: 1,
                filter: serde_json::json!({}),
                revision: 1,
                active: true,
            }],
        )
        .unwrap();
}

pub struct Reply {
    pub status: u16,
    pub body: String,
}

/// One HTTP/1.1 POST; `None` when the connection fails or nothing is answered in `timeout`.
pub fn post(
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout: Duration,
) -> Option<Reply> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    let extra: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nhost: localhost\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let status = response.split(' ').nth(1)?.parse().ok()?;
    let body = response.split_once("\r\n\r\n")?.1.to_string();
    Some(Reply { status, body })
}

pub fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    done()
}
