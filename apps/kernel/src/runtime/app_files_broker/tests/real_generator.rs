//! `connections.action` against the real `chariox-aegs-sdk` server (not an HTTP
//! stand-in): the broker's signed request must reach the provider hook and its
//! answer must come back to the App.
use super::*;
use crate::durable_state::app_connections::ConnectionGrantCommand;
use crate::runtime::app_connection_broker::AppConnectionBroker;
use chariox_aegs_sdk::{AegsProvider, AegsStore, NormalizedEvent, WebhookInput, WebhookRoute};
use chariox_app_runtime::worker_peer::{Broker, BrokerFuture, BrokerRequest};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

const GENERATOR: &str = "dev.chariox.slack";
const CONNECTION: &str = "connection-1";
const EVENT_OWNER: &str = "event-owner-1";

struct Connections(AppConnectionBroker);
impl Broker for Connections {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let service = self.0.clone();
        Box::pin(async move { service.dispatch(request).await })
    }
}

struct Provider;
impl AegsProvider for Provider {
    fn generator_id(&self) -> &'static str {
        GENERATOR
    }

    fn provider_slug(&self) -> &'static str {
        "slack"
    }

    fn normalize_webhook(
        &self,
        _input: WebhookInput<'_>,
        _route: &WebhookRoute,
    ) -> Result<NormalizedEvent, String> {
        Err("this provider receives no webhooks".to_string())
    }
}

/// The SDK server as `run_from_environment` serves it, on a loopback port, until dropped.
struct RealServer {
    url: String,
    directory: std::path::PathBuf,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RealServer {
    fn start(store: impl FnOnce(&AegsStore)) -> Self {
        let directory =
            std::env::temp_dir().join(format!("chariox-aegs-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("aegs.sqlite");
        store(&AegsStore::open(&database).unwrap());
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let address = format!("127.0.0.1:{port}");
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = {
            // The server reads its settings once, before it accepts a connection.
            let _environment = crate::env_lock::lock();
            std::env::set_var("CHARIOX_AEGS_ADDR", &address);
            std::env::set_var("CHARIOX_AEGS_DATABASE_PATH", &database);
            std::env::set_var("CHARIOX_AEGS_ADMISSION", "open");
            let thread = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    tokio::select! {
                        _ = stopped => {}
                        result = chariox_aegs_sdk::run_from_environment(|_| Ok(Arc::new(Provider))) => {
                            panic!("the AEGS server stopped: {result:?}");
                        }
                    }
                });
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while std::net::TcpStream::connect(&address).is_err() {
                assert!(Instant::now() < deadline, "the AEGS server did not start");
                std::thread::sleep(Duration::from_millis(20));
            }
            thread
        };
        Self {
            url: format!("http://{address}"),
            directory,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for RealServer {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn a_granted_action_reaches_the_real_sdk_server_and_its_provider() {
    let fixture = Fixture::new(Mode::Ready);
    let server = RealServer::start(|store| {
        store
            .upsert_ready_connection(CONNECTION, EVENT_OWNER, "slack", &json!({}), 1)
            .unwrap();
    });
    let mut config = crate::config::DaemonConfig::for_tests();
    config.event_generator_management_targets = std::collections::BTreeMap::from([(
        GENERATOR.to_string(),
        crate::config::EventGeneratorManagementTarget {
            url: server.url.clone(),
            registry_discovered: false,
        },
    )]);
    let event_config: crate::runtime::app_lifecycle::EventConfig = Default::default();
    assert!(event_config
        .set(crate::runtime::projection::DaemonConfigProjectionStore::new(config))
        .is_ok());
    let installation = fixture.catalog.installation_id().to_owned();
    fixture
        .store
        .app_connection_grant(ConnectionGrantCommand::Grant {
            owner: "alice".into(),
            installation: installation.clone(),
            generator_id: GENERATOR.into(),
            connection_id: CONNECTION.into(),
            event_owner_id: EVENT_OWNER.into(),
            now_ms: 1,
        })
        .unwrap();
    let broker = AppConnectionBroker::new(
        fixture.store.clone(),
        "alice".into(),
        installation,
        vec![chariox_app_package::ConnectionAccess {
            generator: GENERATOR.into(),
            actions: vec!["notification.reply".into(), "slack.reaction.add".into()],
        }],
        fixture.admission.clone(),
        event_config,
    );
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(Arc::new(Connections(broker)));
        for (id, action, input) in [
            ("reply", "notification.reply", json!({"text": "On it"})),
            ("react", "slack.reaction.add", json!({"name": "eyes"})),
        ] {
            peer.send(
                id,
                "connections.action",
                json!({"connectionId": CONNECTION, "action": action, "input": input,
                    "context": {"channel_id": "C1", "message_ts": "1.2"},
                    "idempotencyKey": id}),
            )
            .await;
            let answer = peer.response().await.1.unwrap_or_else(|error| {
                panic!("{action} failed: {} ({})", error.code, error.message)
            });
            assert_eq!(answer["accepted"], true, "{answer}");
        }
        peer.close().await;
    });
}
