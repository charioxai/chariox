//! Actual relay sockets around the kernel adapter, without Room admission.

use super::*;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chariox_relay::protocol::{
    RelayDisplayTunnelResponseStart, RelayDisplayTunnelStreamChunk, RelayEnvelope,
};
use chariox_relay::{RelayConfig, RelayServer};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub(super) struct LiveRelay {
    pub(super) input: mpsc::Sender<EncryptedRelayPayload>,
    pub(super) output: mpsc::Receiver<EncryptedRelayPayload>,
    pump: JoinHandle<()>,
    server: JoinHandle<()>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl LiveRelay {
    pub(super) async fn connect(
        kernel_input: mpsc::Sender<EncryptedRelayPayload>,
        mut kernel_output: mpsc::Receiver<EncryptedRelayPayload>,
    ) -> Self {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_owned(),
            port: 0,
            shared_token: Some("local-display-drill".to_owned()),
        });
        let listener = server.bind_listener().await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown, stopped) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let setup = timeout(Duration::from_secs(10), async {
            let (mut daemon, _) = connect_async(format!("ws://{address}")).await.unwrap();
            daemon.send(Message::Text(serde_json::json!({"kind":"daemon_register", "registration": {
                "auth_token":"local-display-drill", "daemon_id":"live-selkies-kernel",
                "machine_id":"local-docker", "public_key":"transport-test-key", "capabilities":[]
            }}).to_string().into())).await.unwrap();
            daemon.send(Message::Text(serde_json::json!({"kind":"daemon_display_tunnel_register", "registration": {
                "tunnel_id":"live-selkies", "expires_at_ms": crate::session::unix_epoch_ms() + 60_000,
                "capabilities":["view", "encrypted_websocket"]
            }}).to_string().into())).await.unwrap();
            let registered = daemon.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(matches!(serde_json::from_str::<RelayEnvelope>(&registered).unwrap(), RelayEnvelope::DaemonDisplayTunnelRegistered { error: None, .. }));
            let connect_browser = connect_async(format!("ws://{address}/display/live-selkies/stream"));
            let accept = async {
                let message = daemon.next().await.unwrap().unwrap().into_text().unwrap();
                let RelayEnvelope::DaemonDisplayTunnelOpen { request } = serde_json::from_str(&message).unwrap() else { panic!("missing tunnel open"); };
                daemon.send(Message::Text(serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelResponseStart {
                    response: RelayDisplayTunnelResponseStart { stream_id: request.stream_id.clone(), status: 101, headers: vec![] }
                }).unwrap().into())).await.unwrap();
                request.stream_id
            };
            let (browser, stream_id) = tokio::join!(connect_browser, accept);
            (daemon, browser.unwrap().0, stream_id)
        }).await;
        let (mut daemon, mut browser, stream_id) = match setup {
            Ok(value) => value,
            Err(error) => {
                server_task.abort();
                panic!("relay setup timed out: {error}");
            }
        };
        let (input, mut commands) = mpsc::channel::<EncryptedRelayPayload>(4);
        let (frames, output) = mpsc::channel(4);
        let pump = tokio::spawn(async move {
            loop {
                tokio::select! {
                    packet = kernel_output.recv() => {
                        let Some(packet) = packet else { break; };
                        let bytes = serde_json::to_vec(&packet).unwrap();
                        daemon.send(Message::Text(serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelChunk {
                            chunk: RelayDisplayTunnelStreamChunk { stream_id: stream_id.clone(), data: BASE64.encode(bytes), message_kind: Some("text".to_owned()) }
                        }).unwrap().into())).await.unwrap();
                    }
                    packet = commands.recv() => {
                        let Some(packet) = packet else { break; };
                        browser.send(Message::Text(serde_json::to_string(&packet).unwrap().into())).await.unwrap();
                    }
                    message = browser.next() => {
                        let Some(Ok(Message::Text(bytes))) = message else { break; };
                        frames.send(serde_json::from_str(&bytes).unwrap()).await.unwrap();
                    }
                    message = daemon.next() => {
                        let Some(Ok(Message::Text(bytes))) = message else { break; };
                        let RelayEnvelope::DaemonDisplayTunnelClientChunk { chunk } = serde_json::from_str(&bytes).unwrap() else { panic!("unexpected relay event"); };
                        assert_eq!(chunk.stream_id, stream_id);
                        let packet = serde_json::from_slice(&BASE64.decode(chunk.data).unwrap()).unwrap();
                        if kernel_input.send(packet).await.is_err() { break; }
                    }
                }
            }
            let _ = browser.close(None).await;
            let _ = daemon.close(None).await;
        });
        Self {
            input,
            output,
            pump,
            server: server_task,
            shutdown: Some(shutdown),
        }
    }

    pub(super) async fn close(&mut self) {
        timeout(Duration::from_secs(5), &mut self.pump)
            .await
            .unwrap()
            .unwrap();
        let _ = self.shutdown.take().unwrap().send(());
        timeout(Duration::from_secs(5), &mut self.server)
            .await
            .unwrap()
            .unwrap();
    }
}

impl Drop for LiveRelay {
    fn drop(&mut self) {
        self.pump.abort();
        self.server.abort();
    }
}
