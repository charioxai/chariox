use rand::RngCore;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::error::DaemonError;
use crate::local::{GetSliceDisplayEndpointRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCaller;
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::slice::{SliceDisplayEndpoint, SliceDisplayEndpointAccess, SliceDisplayEndpointKind};
use crate::transport::relay_client::{
    RelayClientState, RelayDisplayTunnelTarget, RelayDisplayTunnelTargetKind,
};
use chariox_relay::protocol::{RelayDisplayTunnelRegistration, RelayEnvelope};

const DISPLAY_TUNNEL_TTL_MS: u64 = 60_000;
const DISPLAY_TUNNEL_RENEWAL_WINDOW_MS: u64 = 10_000;
const DISPLAY_TUNNEL_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) async fn execute_get_slice_display_endpoint_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    caller: &KernelCaller,
    request: GetSliceDisplayEndpointRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let endpoint = runtime_state.slice_display_endpoint(&request.slice_ref)?;
    if endpoint.kind == SliceDisplayEndpointKind::Selkies {
        let _relay_state = relay_state.ok_or_else(|| {
            display_tunnel_error("open Room Selkies display", "relay state is unavailable")
        })?;
        let endpoint = runtime_state
            .open_room_selkies_display(caller, request)
            .await?;
        return Ok(LocalDaemonResponse::SliceDisplayEndpoint { endpoint });
    }
    let endpoint = match relay_state {
        Some(relay_state) => tunneled_display_endpoint(
            endpoint.clone(),
            config_projection.snapshot().relay_url,
            relay_state,
        )
        .await?
        .unwrap_or(endpoint),
        None => endpoint,
    };
    Ok(LocalDaemonResponse::SliceDisplayEndpoint { endpoint })
}

async fn tunneled_display_endpoint(
    local_endpoint: SliceDisplayEndpoint,
    config_relay_url: Option<String>,
    relay_state: Arc<RwLock<RelayClientState>>,
) -> Result<Option<SliceDisplayEndpoint>, DaemonError> {
    if local_endpoint.kind == SliceDisplayEndpointKind::Selkies {
        return Err(display_tunnel_error(
            "register slice display tunnel",
            "Selkies requires encrypted kernel display transport; the legacy HTTP display tunnel is not permitted",
        ));
    }
    if local_endpoint.access != SliceDisplayEndpointAccess::Local {
        return Ok(None);
    }
    let Some(local_base_url) = local_display_base_url(&local_endpoint.url) else {
        return Ok(None);
    };
    let now_ms = crate::session::unix_epoch_ms();
    let expires_at_ms = now_ms.saturating_add(DISPLAY_TUNNEL_TTL_MS);
    let (outgoing_tx, relay_base_url, tunnel_id, previous_target, registration_rx) = {
        let mut guard = relay_state.write().await;
        let previous_target =
            guard.display_tunnel_for_slice(&local_endpoint.slice_id, &local_base_url);
        guard.prune_expired_display_tunnels(now_ms);
        let Some(relay_url) = guard.connected_relay_url().or(config_relay_url) else {
            return Ok(None);
        };
        let Some(relay_base_url) = relay_display_base_url(&relay_url) else {
            return Ok(None);
        };
        if let Some(target) = previous_target.as_ref() {
            if target.expires_at_ms > now_ms.saturating_add(DISPLAY_TUNNEL_RENEWAL_WINDOW_MS) {
                return Ok(relay_tunneled_endpoint(
                    &relay_base_url,
                    target,
                    local_endpoint,
                ));
            }
            if guard.display_tunnel_registration_pending(&target.tunnel_id) {
                return Err(display_tunnel_error(
                    "renew slice display tunnel",
                    "display tunnel renewal is already in progress",
                ));
            }
        }
        let tunnel_id = previous_target
            .as_ref()
            .map(|target| target.tunnel_id.clone())
            .unwrap_or_else(|| format!("display-{}", random_hex_id()));
        let Some(outgoing_tx) = guard.outgoing_sender() else {
            return Ok(None);
        };
        let (registration_tx, registration_rx) = oneshot::channel();
        guard.insert_pending_display_tunnel_registration(tunnel_id.clone(), registration_tx);
        (
            outgoing_tx,
            relay_base_url,
            tunnel_id,
            previous_target,
            registration_rx,
        )
    };
    if outgoing_tx
        .try_send(RelayEnvelope::DaemonDisplayTunnelRegister {
            registration: RelayDisplayTunnelRegistration {
                tunnel_id: tunnel_id.clone(),
                expires_at_ms,
                capabilities: local_endpoint.capabilities.clone(),
            },
        })
        .is_err()
    {
        let mut guard = relay_state.write().await;
        guard.cancel_display_tunnel_registration(&tunnel_id);
        return Err(display_tunnel_error(
            "register slice display tunnel",
            "relay connection is not accepting display registrations",
        ));
    }
    let registration_error = match timeout(DISPLAY_TUNNEL_REGISTRATION_TIMEOUT, registration_rx)
        .await
    {
        Ok(Ok(None)) => None,
        Ok(Ok(Some(error))) => Some(error.message),
        Ok(Err(_)) => Some("relay closed the display registration acknowledgment".to_string()),
        Err(_) => Some("relay did not acknowledge the display registration in time".to_string()),
    };
    if let Some(message) = registration_error {
        let mut guard = relay_state.write().await;
        guard.cancel_display_tunnel_registration(&tunnel_id);
        if previous_target.is_none() {
            let _ = outgoing_tx.try_send(RelayEnvelope::DaemonDisplayTunnelRevoke {
                tunnel_id: tunnel_id.clone(),
            });
        }
        return Err(display_tunnel_error(
            "register slice display tunnel",
            message,
        ));
    }
    let target = RelayDisplayTunnelTarget {
        tunnel_id,
        slice_id: local_endpoint.slice_id.clone(),
        kind: RelayDisplayTunnelTargetKind::HttpProxy { local_base_url },
        expires_at_ms,
        capabilities: local_endpoint.capabilities.clone(),
    };
    relay_state
        .write()
        .await
        .upsert_display_tunnel(target.clone());
    Ok(relay_tunneled_endpoint(
        &relay_base_url,
        &target,
        local_endpoint,
    ))
}

fn relay_tunneled_endpoint(
    relay_base_url: &url::Url,
    target: &RelayDisplayTunnelTarget,
    local_endpoint: SliceDisplayEndpoint,
) -> Option<SliceDisplayEndpoint> {
    let tunnel_url =
        relay_display_endpoint_url(relay_base_url, &target.tunnel_id, &local_endpoint)?;
    Some(SliceDisplayEndpoint {
        slice_id: local_endpoint.slice_id,
        kind: local_endpoint.kind,
        url: tunnel_url,
        access: SliceDisplayEndpointAccess::Tunnel,
        expires_at_ms: Some(target.expires_at_ms),
        capabilities: local_endpoint.capabilities,
        stream_protocol: None,
        stream_id: None,
        peer_public_key: None,
    })
}

fn display_tunnel_error(operation: &'static str, message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: message.into(),
    }
}

pub(crate) async fn register_room_selkies_display_endpoint(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_relay_url: Option<String>,
    slice_id: &str,
    viewer_public_key: String,
    worker_public_key: String,
) -> Result<SliceDisplayEndpoint, DaemonError> {
    crate::transport::relay_crypto::decode_public_key(&viewer_public_key).map_err(|_| {
        display_tunnel_error(
            "register Room Selkies display",
            "viewer public key is invalid",
        )
    })?;
    let now_ms = crate::session::unix_epoch_ms();
    let expires_at_ms = now_ms.saturating_add(DISPLAY_TUNNEL_TTL_MS);
    let tunnel_id = format!("display-{}", random_hex_id());
    let capabilities = vec![
        "view".to_string(),
        "websocket".to_string(),
        "h264".to_string(),
        "software_encoding".to_string(),
        "encrypted".to_string(),
        "single_use".to_string(),
    ];
    let (outgoing_tx, mut relay_base_url, registration_rx) = {
        let mut guard = relay_state.write().await;
        let relay_url = guard
            .connected_relay_url()
            .or(config_relay_url)
            .ok_or_else(|| {
                display_tunnel_error(
                    "register Room Selkies display",
                    "hosted relay is not connected",
                )
            })?;
        let relay_base_url = relay_display_stream_base_url(&relay_url).ok_or_else(|| {
            display_tunnel_error(
                "register Room Selkies display",
                "Selkies display requires wss, except for a loopback ws relay",
            )
        })?;
        let outgoing_tx = guard.outgoing_sender().ok_or_else(|| {
            display_tunnel_error(
                "register Room Selkies display",
                "hosted relay is not accepting display registrations",
            )
        })?;
        let (registration_tx, registration_rx) = oneshot::channel();
        guard.insert_pending_display_tunnel_registration(tunnel_id.clone(), registration_tx);
        (outgoing_tx, relay_base_url, registration_rx)
    };
    if outgoing_tx
        .try_send(RelayEnvelope::DaemonDisplayTunnelRegister {
            registration: RelayDisplayTunnelRegistration {
                tunnel_id: tunnel_id.clone(),
                expires_at_ms,
                capabilities: capabilities.clone(),
            },
        })
        .is_err()
    {
        relay_state
            .write()
            .await
            .cancel_display_tunnel_registration(&tunnel_id);
        return Err(display_tunnel_error(
            "register Room Selkies display",
            "relay connection is not accepting display registrations",
        ));
    }
    let registration_error = match timeout(DISPLAY_TUNNEL_REGISTRATION_TIMEOUT, registration_rx)
        .await
    {
        Ok(Ok(None)) => None,
        Ok(Ok(Some(error))) => Some(error.message),
        Ok(Err(_)) => Some("relay closed the display registration acknowledgment".to_string()),
        Err(_) => Some("relay did not acknowledge the display registration in time".to_string()),
    };
    if let Some(message) = registration_error {
        relay_state
            .write()
            .await
            .cancel_display_tunnel_registration(&tunnel_id);
        let _ = outgoing_tx.try_send(RelayEnvelope::DaemonDisplayTunnelRevoke {
            tunnel_id: tunnel_id.clone(),
        });
        return Err(display_tunnel_error(
            "register Room Selkies display",
            message,
        ));
    }
    relay_state
        .write()
        .await
        .upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: tunnel_id.clone(),
            slice_id: slice_id.to_string(),
            kind: RelayDisplayTunnelTargetKind::Selkies {
                viewer_public_key,
                command_program: "/opt/chariox-selkies/bin/python".to_string(),
                command_args: vec!["/opt/chariox-slice/slice-selkies-stream.py".to_string()],
            },
            expires_at_ms,
            capabilities: capabilities.clone(),
        });
    relay_base_url.set_path(&format!("/display/{tunnel_id}/stream"));
    Ok(SliceDisplayEndpoint {
        slice_id: slice_id.to_string(),
        kind: SliceDisplayEndpointKind::Selkies,
        url: relay_base_url.to_string(),
        access: SliceDisplayEndpointAccess::Tunnel,
        expires_at_ms: Some(expires_at_ms),
        capabilities,
        stream_protocol: Some("chariox-display-v1".to_string()),
        stream_id: Some(tunnel_id),
        peer_public_key: Some(worker_public_key),
    })
}

pub(super) async fn revoke_display_tunnels_for_slice(
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    slice_id: &str,
) {
    let Some(relay_state) = relay_state else {
        return;
    };
    let (outgoing_tx, tunnel_ids) = {
        let mut guard = relay_state.write().await;
        (
            guard.outgoing_sender(),
            guard.remove_display_tunnels_for_slice(slice_id),
        )
    };
    let Some(outgoing_tx) = outgoing_tx else {
        return;
    };
    for tunnel_id in tunnel_ids {
        let _ = outgoing_tx.try_send(RelayEnvelope::DaemonDisplayTunnelRevoke { tunnel_id });
    }
}

fn relay_display_base_url(relay_url: &str) -> Option<url::Url> {
    let mut url = url::Url::parse(relay_url).ok()?;
    match url.scheme() {
        "wss" => {
            url.set_scheme("https").ok()?;
        }
        _ => return None,
    }
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Some(url)
}

fn relay_display_stream_base_url(relay_url: &str) -> Option<url::Url> {
    let mut url = url::Url::parse(relay_url).ok()?;
    match url.scheme() {
        "wss" => {}
        "ws" if url.host_str().is_some_and(is_loopback_relay_host) => {}
        _ => return None,
    }
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Some(url)
}

fn is_loopback_relay_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn local_display_base_url(local_url: &str) -> Option<String> {
    let mut url = url::Url::parse(local_url).ok()?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn relay_display_endpoint_url(
    relay_base_url: &url::Url,
    tunnel_id: &str,
    local_endpoint: &SliceDisplayEndpoint,
) -> Option<String> {
    let local_url = url::Url::parse(&local_endpoint.url).ok()?;
    let mut tunnel_url = relay_base_url.clone();
    let local_path = if local_url.path().is_empty() {
        "/"
    } else {
        local_url.path()
    };
    tunnel_url.set_path(&format!("/display/{tunnel_id}{local_path}"));
    if local_endpoint.kind == SliceDisplayEndpointKind::Novnc {
        let host = relay_base_url.host_str()?;
        let port = relay_base_url
            .port_or_known_default()
            .map(|port| port.to_string())?;
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("host", host);
        query.append_pair("port", &port);
        query.append_pair("path", &format!("display/{tunnel_id}/websockify"));
        query.append_pair("autoconnect", "true");
        query.append_pair("resize", "scale");
        tunnel_url.set_query(Some(&query.finish()));
    } else {
        tunnel_url.set_query(local_url.query());
    }
    Some(tunnel_url.to_string())
}

fn random_hex_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn room_selkies_registration_is_key_bound_single_use_over_wss() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        let relay_state = Arc::new(RwLock::new(state));
        let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
        let viewer_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
                .expect("viewer public key should derive");
        let task = tokio::spawn(register_room_selkies_display_endpoint(
            Arc::clone(&relay_state),
            Some("wss://relay.example.test".to_string()),
            "slice-1",
            viewer_public.clone(),
            "worker-public-key".to_string(),
        ));
        let registration = match priority_rx
            .recv()
            .await
            .expect("Selkies display registration should be queued")
        {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => registration,
            other => panic!("unexpected relay envelope: {other:?}"),
        };
        assert!(registration.capabilities.contains(&"encrypted".to_string()));
        assert!(registration
            .capabilities
            .contains(&"single_use".to_string()));
        relay_state
            .write()
            .await
            .resolve_display_tunnel_registration(&registration.tunnel_id, None);
        let endpoint = task
            .await
            .expect("registration task should finish")
            .expect("registration should succeed");
        assert_eq!(
            endpoint.url,
            format!(
                "wss://relay.example.test/display/{}/stream",
                registration.tunnel_id
            )
        );
        assert_eq!(endpoint.stream_id, Some(registration.tunnel_id.clone()));
        assert_eq!(
            endpoint.peer_public_key.as_deref(),
            Some("worker-public-key")
        );
        let target = relay_state
            .write()
            .await
            .claim_display_tunnel_for_open(&registration.tunnel_id, crate::session::unix_epoch_ms())
            .expect("single-use target should be claimable once");
        assert!(matches!(
            target.kind,
            RelayDisplayTunnelTargetKind::Selkies {
                viewer_public_key: key,
                ..
            } if key == viewer_public
        ));
        assert!(
            relay_state
                .write()
                .await
                .claim_display_tunnel_for_open(
                    &registration.tunnel_id,
                    crate::session::unix_epoch_ms(),
                )
                .is_none()
        );
    }

    #[tokio::test]
    async fn room_selkies_registration_accepts_a_loopback_ws_relay_for_local_drills() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "ws://127.0.0.1:43130");
        let relay_state = Arc::new(RwLock::new(state));
        let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
        let viewer_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
                .expect("viewer public key should derive");
        let task = tokio::spawn(register_room_selkies_display_endpoint(
            Arc::clone(&relay_state),
            Some("ws://127.0.0.1:43130".to_string()),
            "slice-1",
            viewer_public,
            "worker-public-key".to_string(),
        ));
        let registration = match timeout(Duration::from_millis(100), priority_rx.recv())
            .await
            .expect("loopback relay should accept a display registration promptly")
            .expect("local display registration should be queued")
        {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => registration,
            other => panic!("unexpected relay envelope: {other:?}"),
        };
        relay_state
            .write()
            .await
            .resolve_display_tunnel_registration(&registration.tunnel_id, None);
        let endpoint = task
            .await
            .expect("registration task should finish")
            .expect("loopback ws relay should be accepted");
        assert_eq!(
            endpoint.url,
            format!(
                "ws://127.0.0.1:43130/display/{}/stream",
                registration.tunnel_id
            )
        );
    }

    #[tokio::test]
    async fn room_selkies_registration_rejects_remote_plaintext_ws() {
        let (outgoing_tx, _priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "ws://relay.example.test");
        let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
        let viewer_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
                .expect("viewer public key should derive");
        let error = register_room_selkies_display_endpoint(
            Arc::new(RwLock::new(state)),
            Some("ws://relay.example.test".to_string()),
            "slice-1",
            viewer_public,
            "worker-public-key".to_string(),
        )
        .await
        .expect_err("remote plaintext display relays must be rejected");
        assert!(error
            .to_string()
            .contains("requires wss, except for a loopback ws relay"));
    }

    #[tokio::test]
    async fn display_endpoint_returns_tunnel_when_wss_relay_is_connected() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        let relay_state = Arc::new(RwLock::new(state));
        let local = local_novnc_endpoint();

        let endpoint_task = tokio::spawn(tunneled_display_endpoint(
            local,
            Some("wss://relay.example.test".to_string()),
            Arc::clone(&relay_state),
        ));

        let registration = priority_rx
            .recv()
            .await
            .expect("display tunnel registration should be queued");
        let tunnel_id = match registration {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => {
                assert!(registration.tunnel_id.starts_with("display-"));
                assert!(registration.expires_at_ms > crate::session::unix_epoch_ms());
                registration.tunnel_id
            }
            other => panic!("unexpected relay envelope: {other:?}"),
        };
        assert!(!endpoint_task.is_finished());
        relay_state
            .write()
            .await
            .resolve_display_tunnel_registration(&tunnel_id, None);

        let endpoint = endpoint_task
            .await
            .expect("display endpoint task should finish")
            .expect("display endpoint request should succeed")
            .expect("acknowledged wss relay should produce tunnel endpoint");

        assert_eq!(endpoint.access, SliceDisplayEndpointAccess::Tunnel);
        assert!(endpoint
            .url
            .starts_with("https://relay.example.test/display/display-"));
        assert!(endpoint.url.contains("/vnc.html?"));
        assert!(endpoint.url.contains("path=display%2Fdisplay-"));
        assert_eq!(endpoint.expires_at_ms.is_some(), true);
    }

    #[tokio::test]
    async fn display_endpoint_does_not_return_tunnel_when_relay_rejects_registration() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        let relay_state = Arc::new(RwLock::new(state));

        let endpoint_task = tokio::spawn(tunneled_display_endpoint(
            local_novnc_endpoint(),
            Some("wss://relay.example.test".to_string()),
            Arc::clone(&relay_state),
        ));
        let tunnel_id = match priority_rx
            .recv()
            .await
            .expect("display tunnel registration should be queued")
        {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => registration.tunnel_id,
            other => panic!("unexpected relay envelope: {other:?}"),
        };
        relay_state
            .write()
            .await
            .resolve_display_tunnel_registration(
                &tunnel_id,
                Some(chariox_relay::protocol::RelayError {
                    code: "invalid_display_tunnel".to_string(),
                    message: "registration rejected".to_string(),
                    retryable: false,
                }),
            );

        let error = endpoint_task
            .await
            .expect("display endpoint task should finish")
            .expect_err("rejected registration should fail the endpoint request");
        assert!(error.to_string().contains("registration rejected"));
        assert!(relay_state
            .read()
            .await
            .display_tunnel(&tunnel_id, crate::session::unix_epoch_ms())
            .is_none());
        assert_eq!(
            priority_rx
                .recv()
                .await
                .expect("rejected registration should be revoked"),
            RelayEnvelope::DaemonDisplayTunnelRevoke { tunnel_id },
        );
    }

    #[tokio::test]
    async fn display_endpoint_does_not_tunnel_without_hosted_wss_relay() {
        let (outgoing_tx, _priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "ws://127.0.0.1:43130");
        let relay_state = Arc::new(RwLock::new(state));

        assert!(tunneled_display_endpoint(
            local_novnc_endpoint(),
            Some("ws://127.0.0.1:43130".to_string()),
            relay_state,
        )
        .await
        .expect("local relay lookup should succeed")
        .is_none());
    }

    #[tokio::test]
    async fn display_tunnel_renewal_keeps_the_same_endpoint_url() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        let relay_state = Arc::new(RwLock::new(state));

        let first_task = tokio::spawn(tunneled_display_endpoint(
            local_novnc_endpoint(),
            Some("wss://relay.example.test".to_string()),
            Arc::clone(&relay_state),
        ));
        let first_registration = match priority_rx
            .recv()
            .await
            .expect("initial display registration should be queued")
        {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => registration,
            other => panic!("unexpected relay envelope: {other:?}"),
        };
        relay_state
            .write()
            .await
            .resolve_display_tunnel_registration(&first_registration.tunnel_id, None);
        let first_endpoint = first_task
            .await
            .expect("initial endpoint task should finish")
            .expect("initial endpoint request should succeed")
            .expect("initial endpoint should be tunneled");

        let expiring_at_ms = crate::session::unix_epoch_ms().saturating_add(1_000);
        relay_state
            .write()
            .await
            .upsert_display_tunnel(RelayDisplayTunnelTarget {
                tunnel_id: first_registration.tunnel_id.clone(),
                slice_id: "slice-1".to_string(),
                kind: RelayDisplayTunnelTargetKind::HttpProxy {
                    local_base_url: "http://127.0.0.1:5901/".to_string(),
                },
                expires_at_ms: expiring_at_ms,
                capabilities: first_registration.capabilities.clone(),
            });
        let renewal_task = tokio::spawn(tunneled_display_endpoint(
            local_novnc_endpoint(),
            Some("wss://relay.example.test".to_string()),
            Arc::clone(&relay_state),
        ));
        let renewal_registration = match priority_rx
            .recv()
            .await
            .expect("display renewal should be queued")
        {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => registration,
            other => panic!("unexpected relay envelope: {other:?}"),
        };
        assert_eq!(
            renewal_registration.tunnel_id, first_registration.tunnel_id,
            "renewal must extend the existing tunnel identity"
        );
        relay_state
            .write()
            .await
            .resolve_display_tunnel_registration(&renewal_registration.tunnel_id, None);
        let renewed_endpoint = renewal_task
            .await
            .expect("renewal endpoint task should finish")
            .expect("renewal endpoint request should succeed")
            .expect("renewed endpoint should remain tunneled");

        assert_eq!(renewed_endpoint.url, first_endpoint.url);
        assert!(renewed_endpoint.expires_at_ms > Some(expiring_at_ms));
    }

    #[tokio::test]
    async fn healthy_display_tunnel_is_returned_without_reregistering() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        state.upsert_display_tunnel(test_tunnel("display-stable", "slice-1"));
        let relay_state = Arc::new(RwLock::new(state));

        let endpoint = tunneled_display_endpoint(
            local_novnc_endpoint(),
            Some("wss://relay.example.test".to_string()),
            relay_state,
        )
        .await
        .expect("cached endpoint lookup should succeed")
        .expect("cached endpoint should remain tunneled");

        assert!(endpoint.url.contains("/display/display-stable/vnc.html"));
        assert!(priority_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn revoking_slice_display_tunnels_keeps_other_slices_registered() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        state.upsert_display_tunnel(test_tunnel("display-a", "slice-1"));
        state.upsert_display_tunnel(test_tunnel("display-b", "slice-1"));
        state.upsert_display_tunnel(test_tunnel("display-c", "slice-2"));
        let relay_state = Arc::new(RwLock::new(state));

        revoke_display_tunnels_for_slice(Some(Arc::clone(&relay_state)), "slice-1").await;

        let mut revoked = Vec::new();
        for _ in 0..2 {
            match priority_rx
                .recv()
                .await
                .expect("slice tunnel revoke should be queued")
            {
                RelayEnvelope::DaemonDisplayTunnelRevoke { tunnel_id } => revoked.push(tunnel_id),
                other => panic!("unexpected relay envelope: {other:?}"),
            }
        }
        revoked.sort();
        assert_eq!(revoked, vec!["display-a", "display-b"]);
        assert!(relay_state
            .read()
            .await
            .display_tunnel("display-c", crate::session::unix_epoch_ms())
            .is_some());
    }

    fn local_novnc_endpoint() -> SliceDisplayEndpoint {
        SliceDisplayEndpoint {
            slice_id: "slice-1".to_string(),
            kind: SliceDisplayEndpointKind::Novnc,
            url: "http://127.0.0.1:5901/vnc.html?host=127.0.0.1&port=5901&autoconnect=true"
                .to_string(),
            access: SliceDisplayEndpointAccess::Local,
            expires_at_ms: None,
            capabilities: vec![
                "view".to_string(),
                "keyboard".to_string(),
                "mouse".to_string(),
            ],
            stream_protocol: None,
            stream_id: None,
            peer_public_key: None,
        }
    }

    fn test_tunnel(tunnel_id: &str, slice_id: &str) -> RelayDisplayTunnelTarget {
        RelayDisplayTunnelTarget {
            tunnel_id: tunnel_id.to_string(),
            slice_id: slice_id.to_string(),
            kind: RelayDisplayTunnelTargetKind::HttpProxy {
                local_base_url: "http://127.0.0.1:5901/".to_string(),
            },
            expires_at_ms: crate::session::unix_epoch_ms().saturating_add(60_000),
            capabilities: vec!["view".to_string()],
        }
    }
}
