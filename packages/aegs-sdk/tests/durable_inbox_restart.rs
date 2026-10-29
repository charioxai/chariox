//! Real-process drill: a webhook that was received must reach AEDS even when the AEGS process
//! is killed with SIGKILL while the publication is in flight. The server under test is
//! `run_from_environment`, started as a child process on a real port with a real SQLite file.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chariox_aegs_sdk::{
    AegsProvider, AegsStore, NormalizedEvent, SubscriptionClaim, WebhookInput, WebhookRoute,
};

const GENERATOR: &str = "dev.chariox.drill";
const CHILD_ENV: &str = "CHARIOX_DRILL_CHILD";

struct DrillProvider;

impl AegsProvider for DrillProvider {
    fn generator_id(&self) -> &'static str {
        GENERATOR
    }

    fn provider_slug(&self) -> &'static str {
        "drill"
    }

    fn normalize_webhook(
        &self,
        input: WebhookInput<'_>,
        _route: &WebhookRoute,
    ) -> Result<NormalizedEvent, String> {
        let id = String::from_utf8_lossy(input.body).trim().to_string();
        Ok(NormalizedEvent {
            occurrence_id: format!("drill-{id}"),
            event_type: "drill.event".to_string(),
            occurred_at: "2026-10-29T00:00:00Z".to_string(),
            connection_scope: "scope-1".to_string(),
            prompt: "drill prompt".to_string(),
            metadata: serde_json::json!({}),
            reply_context: None,
        })
    }
}

/// Entry point of the server child; a no-op when the test binary runs normally.
#[test]
fn drill_server_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(chariox_aegs_sdk::run_from_environment(|_store| {
            Ok(Arc::new(DrillProvider))
        }))
        .unwrap();
}

#[derive(Clone, Copy, PartialEq)]
enum AedsMode {
    /// Accept the connection, read the request and never answer.
    Hang,
    Accept,
}

struct FakeAeds {
    port: u16,
    hang: Arc<AtomicBool>,
    requests_seen: Arc<AtomicUsize>,
    accepted: Arc<Mutex<Vec<String>>>,
}

impl FakeAeds {
    fn start() -> Self {
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

    fn set_mode(&self, mode: AedsMode) {
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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn_server(database: &Path, port: u16, aeds_port: u16) -> Child {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "drill_server_child", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env("CHARIOX_AEGS_ADDR", format!("127.0.0.1:{port}"))
        .env("CHARIOX_AEGS_DATABASE_PATH", database)
        .env("CHARIOX_AEGS_MANAGEMENT_TOKEN", "drill-management-token")
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

fn seed_subscription(database: &Path) {
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

fn post(port: u16, path: &str, body: &str, timeout: Duration) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nhost: localhost\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    done()
}

#[test]
fn webhook_received_before_a_sigkill_is_published_after_the_restart() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let directory = std::env::temp_dir().join(format!("chariox-aegs-drill-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("aegs.db");
    seed_subscription(&database);
    let aeds = FakeAeds::start();
    let port = free_port();

    let mut server = spawn_server(&database, port, aeds.port);
    let sender = std::thread::spawn(move || {
        post(
            port,
            "/webhooks/drill/conn-1",
            "event-1",
            Duration::from_secs(30),
        )
    });
    assert!(
        wait_until(Duration::from_secs(10), || aeds
            .requests_seen
            .load(Ordering::SeqCst)
            > 0),
        "the server never started publishing the received webhook"
    );
    // The publication is in flight: the webhook was received and is not yet delivered.
    server.kill().unwrap();
    server.wait().unwrap();
    let _ = sender.join();
    assert!(aeds.accepted.lock().unwrap().is_empty());

    aeds.set_mode(AedsMode::Accept);
    let mut restarted = spawn_server(&database, port, aeds.port);
    // The crashed attempt keeps its lease for 60 s, then the retry worker publishes it.
    let delivered = wait_until(Duration::from_secs(120), || {
        aeds.accepted.lock().unwrap().as_slice() == ["drill-event-1"]
    });
    restarted.kill().unwrap();
    restarted.wait().unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    assert!(
        delivered,
        "the webhook received before the kill was never published after the restart"
    );
}
