//! Real-process drill: a webhook that was received must reach AEDS even when the AEGS process
//! is killed with SIGKILL while the publication is in flight. The server under test is
//! `run_from_environment`, started as a child process on a real port with a real SQLite file.

mod inbox_support;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chariox_aegs_sdk::{AegsProvider, NormalizedEvent, WebhookInput, WebhookRoute};
use inbox_support::{
    free_port, post, seed_subscription, spawn_server, wait_until, AedsMode, FakeAeds, CHILD_ENV,
    GENERATOR,
};

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

    let mut server = spawn_server("drill_server_child", &database, port, aeds.port);
    let sender = std::thread::spawn(move || {
        post(
            port,
            "/webhooks/drill/conn-1",
            &[],
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
    let mut restarted = spawn_server("drill_server_child", &database, port, aeds.port);
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
