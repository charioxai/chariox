use std::sync::Arc;
use std::time::Duration;

use futures_util::SinkExt;
use tokio::sync::{watch, RwLock};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{DaemonRegistration, RelayEnvelope};

use crate::config::DaemonConfig;

#[derive(Debug, Clone, Default)]
pub struct RelayClientState {
    connected: bool,
}

pub async fn run_daemon_relay_connector(
    config: DaemonConfig,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let Some(relay_url) = config.relay_url.clone() else {
        return;
    };
    let Some(relay_token) = config.relay_token.clone() else {
        return;
    };

    let heartbeat = Duration::from_millis(config.relay_heartbeat_ms);
    loop {
        if *shutdown.borrow() {
            set_connected(&state, false).await;
            return;
        }

        match connect_async(&relay_url).await {
            Ok((mut socket, _)) => {
                let register = RelayEnvelope::DaemonRegister {
                    registration: DaemonRegistration {
                        auth_token: relay_token.clone(),
                        daemon_id: config.daemon_id.clone(),
                        machine_id: config.host_machine_id.clone(),
                        daemon_alias: config.daemon_alias.clone(),
                        capabilities: vec!["kernel_websocket".to_string()],
                    },
                };
                let register_message = Message::Text(
                    serde_json::to_string(&register)
                        .expect("daemon relay registration should serialize")
                        .into(),
                );
                if socket.send(register_message).await.is_err() {
                    set_connected(&state, false).await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                set_connected(&state, true).await;

                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                let _ = socket.send(Message::Text(
                                    serde_json::to_string(&RelayEnvelope::Close {
                                        reason: "daemon shutting down".to_string(),
                                    })
                                    .expect("relay close should serialize")
                                    .into(),
                                )).await;
                                let _ = socket.close(None).await;
                                set_connected(&state, false).await;
                                return;
                            }
                        }
                        _ = sleep(heartbeat) => {
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: config.daemon_id.clone(),
                            };
                            let heartbeat_message = Message::Text(
                                serde_json::to_string(&heartbeat_frame)
                                    .expect("daemon relay heartbeat should serialize")
                                    .into(),
                            );
                            if socket.send(heartbeat_message).await.is_err() {
                                set_connected(&state, false).await;
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                set_connected(&state, false).await;
                let reconnect_delay = sleep(Duration::from_secs(1));
                tokio::pin!(reconnect_delay);
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_ok() && *shutdown.borrow() {
                            return;
                        }
                    }
                    _ = &mut reconnect_delay => {}
                }
            }
        }
    }
}

async fn set_connected(state: &Arc<RwLock<RelayClientState>>, connected: bool) {
    state.write().await.connected = connected;
}

#[cfg(test)]
mod tests {
    use super::*;

    use arroba_relay::{RelayConfig, RelayServer};
    use tokio::sync::oneshot;
    use tokio::time::{sleep, Duration};

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_connector_registers_with_relay() {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            config.clone(),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        {
            let guard = registry.read().await;
            assert!(guard.daemon(&config.daemon_id).is_some());
        }
        assert!(state.read().await.connected);

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        sleep(Duration::from_millis(50)).await;
        {
            let guard = registry.read().await;
            assert!(guard.daemon(&config.daemon_id).is_none());
        }
        assert!(!state.read().await.connected);

        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    async fn wait_for_daemon_registration(
        registry: Arc<RwLock<arroba_relay::server::RelayRegistry>>,
        daemon_id: &str,
    ) {
        for _ in 0..40 {
            if registry.read().await.daemon(daemon_id).is_some() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("daemon `{daemon_id}` did not register with relay");
    }
}
