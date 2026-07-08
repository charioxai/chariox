use rand::RngCore;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{LocalDaemonResponse, SliceRefRequest};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::slice::{SliceDisplayEndpoint, SliceDisplayEndpointAccess, SliceDisplayEndpointKind};
use crate::transport::relay_client::{RelayClientState, RelayDisplayTunnelTarget};
use arroba_relay::protocol::{RelayDisplayTunnelRegistration, RelayEnvelope};

const DISPLAY_TUNNEL_TTL_MS: u64 = 60_000;

pub(super) async fn execute_get_slice_display_endpoint_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let endpoint = runtime_state.slice_display_endpoint(&request.slice_ref)?;
    let endpoint = match relay_state {
        Some(relay_state) => tunneled_display_endpoint(
            endpoint.clone(),
            config_projection.snapshot().relay_url,
            relay_state,
        )
        .await
        .unwrap_or(endpoint),
        None => endpoint,
    };
    Ok(LocalDaemonResponse::SliceDisplayEndpoint { endpoint })
}

async fn tunneled_display_endpoint(
    local_endpoint: SliceDisplayEndpoint,
    config_relay_url: Option<String>,
    relay_state: Arc<RwLock<RelayClientState>>,
) -> Option<SliceDisplayEndpoint> {
    if local_endpoint.access != SliceDisplayEndpointAccess::Local {
        return None;
    }
    let local_base_url = local_display_base_url(&local_endpoint.url)?;
    let now_ms = crate::session::unix_epoch_ms();
    let expires_at_ms = now_ms.saturating_add(DISPLAY_TUNNEL_TTL_MS);
    let tunnel_id = format!("display-{}", random_hex_id());
    let (outgoing_tx, tunnel_url) = {
        let mut guard = relay_state.write().await;
        guard.prune_expired_display_tunnels(now_ms);
        let relay_url = guard.connected_relay_url().or(config_relay_url)?;
        let relay_base_url = relay_display_base_url(&relay_url)?;
        let tunnel_url = relay_display_endpoint_url(&relay_base_url, &tunnel_id, &local_endpoint)?;
        let outgoing_tx = guard.outgoing_sender()?;
        guard.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: tunnel_id.clone(),
            slice_id: local_endpoint.slice_id.clone(),
            local_base_url,
            expires_at_ms,
        });
        (outgoing_tx, tunnel_url)
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
        relay_state.write().await.remove_display_tunnel(&tunnel_id);
        return None;
    }
    Some(SliceDisplayEndpoint {
        slice_id: local_endpoint.slice_id,
        kind: local_endpoint.kind,
        url: tunnel_url,
        access: SliceDisplayEndpointAccess::Tunnel,
        expires_at_ms: Some(expires_at_ms),
        capabilities: local_endpoint.capabilities,
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
    async fn display_endpoint_returns_tunnel_when_wss_relay_is_connected() {
        let (outgoing_tx, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        let mut state = RelayClientState::default();
        state.test_set_connected_sender(outgoing_tx, "wss://relay.example.test");
        let relay_state = Arc::new(RwLock::new(state));
        let local = local_novnc_endpoint();

        let endpoint = tunneled_display_endpoint(
            local,
            Some("wss://relay.example.test".to_string()),
            Arc::clone(&relay_state),
        )
        .await
        .expect("connected wss relay should produce tunnel endpoint");

        assert_eq!(endpoint.access, SliceDisplayEndpointAccess::Tunnel);
        assert!(endpoint
            .url
            .starts_with("https://relay.example.test/display/display-"));
        assert!(endpoint.url.contains("/vnc.html?"));
        assert!(endpoint.url.contains("path=display%2Fdisplay-"));
        assert_eq!(endpoint.expires_at_ms.is_some(), true);

        let registration = priority_rx
            .recv()
            .await
            .expect("display tunnel registration should be queued");
        match registration {
            RelayEnvelope::DaemonDisplayTunnelRegister { registration } => {
                assert!(registration.tunnel_id.starts_with("display-"));
                assert!(registration.expires_at_ms > crate::session::unix_epoch_ms());
            }
            other => panic!("unexpected relay envelope: {other:?}"),
        }
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
        .is_none());
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
        }
    }

    fn test_tunnel(tunnel_id: &str, slice_id: &str) -> RelayDisplayTunnelTarget {
        RelayDisplayTunnelTarget {
            tunnel_id: tunnel_id.to_string(),
            slice_id: slice_id.to_string(),
            local_base_url: "http://127.0.0.1:5901/".to_string(),
            expires_at_ms: crate::session::unix_epoch_ms().saturating_add(60_000),
        }
    }
}
