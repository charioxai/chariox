use tokio::time::{sleep, Duration};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_relay_connection_executor::ensure_cloud_relay_connection;
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::slice::{LocalDockerSliceRelay, SliceRecord};

pub(super) type SliceWorkerProvision =
    Box<dyn FnOnce() -> Result<(), DaemonError> + Send + 'static>;

pub(super) async fn provision_and_prepare_worker_discovery(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay: &LocalDockerSliceRelay,
    provision: SliceWorkerProvision,
) -> Result<DaemonConfig, DaemonError> {
    tokio::task::spawn_blocking(provision)
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.start",
            message: format!("slice supervisor task failed: {error}"),
        })??;
    if relay.uses_shared_relay() {
        ensure_cloud_relay_connection(runtime_state, config_projection).await?;
    }
    Ok(relay.worker_discovery_config(config_projection.snapshot()))
}

pub(super) async fn discover_started_slice_worker(
    config: &DaemonConfig,
    slice: &SliceRecord,
) -> Result<chariox_relay::protocol::RelayKernelPresence, DaemonError> {
    let worker_ref = slice.worker_kernel_ref.clone();
    let mut last_error = None;
    for _ in 0..20 {
        match crate::transport::relay_discovery::get_live_kernel(config, &worker_ref).await {
            Ok(kernel) => return Ok(kernel),
            Err(error) => {
                last_error = Some(error);
                sleep(Duration::from_millis(250)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| DaemonError::LocalTransport {
        operation: "slice.discover_worker",
        message: format!("slice `{}` worker did not appear", slice.name),
    }))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::app::DaemonApp;
    use crate::runtime::router::CommandRouter;
    use tokio::sync::Mutex;

    fn expired_cloud_profile(api_url: String) -> crate::config::PersistedCloudRelayProfile {
        crate::config::PersistedCloudRelayProfile {
            api_url,
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "acct".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_alias: None,
            machine_id: Some("machine-1".to_string()),
            machine_alias: None,
            machine_credential: Some("machine-secret".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: Some(1),
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("set Cloud fixture timeout");
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            let bytes = stream.read(&mut chunk).expect("read token refresh request");
            if bytes == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..bytes]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8(request).expect("token refresh request should be UTF-8")
    }

    #[tokio::test]
    async fn shared_relay_discovery_refreshes_expired_owner_token_after_provisioning() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Cloud fixture");
        let address = listener.local_addr().expect("Cloud fixture address");
        let provisioned = Arc::new(AtomicBool::new(false));
        let fixture_provisioned = Arc::clone(&provisioned);
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept token refresh request");
            assert!(
                fixture_provisioned.load(Ordering::SeqCst),
                "owner token refresh must start after worker provisioning"
            );
            let request = read_http_request(&mut stream);
            assert!(request.starts_with("POST /relay/token HTTP/1.1"));
            assert!(request.contains(r#""machineCredential":"machine-secret""#));
            let body = r#"{"token":"refreshed-owner-token","expiresAt":"2099-01-01T00:00:00Z"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write token refresh response");
        });
        let mut expired = crate::config::DaemonConfig::for_tests();
        expired.relay_url = Some("wss://relay.example.test".to_string());
        expired.relay_token = Some("expired-owner-token".to_string());
        expired.cloud_relay = Some(expired_cloud_profile(format!("http://{address}")));
        let app = DaemonApp::bootstrap(expired).expect("test daemon should boot");
        let projection = app.config_projection_store();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let runtime_state = router.runtime_state();
        let relay = LocalDockerSliceRelay {
            relay_url: "wss://relay.example.test".to_string(),
            container_relay_url: Some("wss://relay.example.test".to_string()),
            relay_token: "worker-bootstrap-token".to_string(),
            cloud_relay_config_json: None,
        };
        let provision_state = Arc::clone(&provisioned);

        let discovery = provision_and_prepare_worker_discovery(
            &runtime_state,
            &projection,
            &relay,
            Box::new(move || {
                provision_state.store(true, Ordering::SeqCst);
                Ok(())
            }),
        )
        .await
        .expect("hosted discovery should prepare after refreshing the owner token");
        fixture.join().expect("Cloud fixture should finish");

        assert_eq!(
            discovery.relay_token.as_deref(),
            Some("refreshed-owner-token")
        );
        assert_ne!(
            discovery.relay_token.as_deref(),
            Some(relay.relay_token.as_str()),
            "metadata discovery must not use the worker bootstrap token"
        );
        assert!(
            discovery
                .cloud_relay
                .as_ref()
                .and_then(|profile| profile.token_expires_at_ms)
                .is_some_and(|expires_at_ms| expires_at_ms > crate::session::unix_epoch_ms()),
            "the refreshed owner profile must stay available to later hosted operations"
        );
    }
}
