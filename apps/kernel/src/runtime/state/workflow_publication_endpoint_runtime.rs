//! Runtime registration for served workflow publication endpoints.

use std::sync::Arc;

use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{LocalDaemonResponse, RegisterWorkflowPublicationEndpointRequest};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::transport::relay_client::{RelayClientState, RelayDisplayTunnelTarget};
use arroba_relay::protocol::{RelayDisplayTunnelRegistration, RelayEnvelope};

use super::KernelRuntimeState;

const PUBLICATION_TUNNEL_TTL_MS: u64 = 10 * 60 * 1_000;
const PUBLICATION_TUNNEL_CAPABILITIES: [&str; 3] = ["http", "websocket", "publication"];

pub(crate) fn restore_durable_workflow_publication_tunnels(
    relay_state: &mut RelayClientState,
    sessions: &crate::session::SessionService,
    now_ms: u64,
) -> usize {
    let mut restored = 0;
    for session in sessions.durable_sessions() {
        for publication in session.workflow_publications() {
            let Some(target) = durable_publication_tunnel_target(publication, now_ms) else {
                continue;
            };
            relay_state.upsert_display_tunnel(target);
            restored += 1;
        }
    }
    restored
}

fn durable_publication_tunnel_target(
    publication: &crate::session::WorkflowPublicationDefinition,
    now_ms: u64,
) -> Option<RelayDisplayTunnelTarget> {
    if !publication.enabled() {
        return None;
    }
    let deployment = publication.deployment()?.as_object()?;
    if deployment.get("kind")?.as_str()? != "tunnel" {
        return None;
    }
    let expires_at_ms = deployment.get("expires_at_ms")?.as_u64()?;
    if expires_at_ms <= now_ms {
        return None;
    }
    let local_url = parse_local_publication_url(deployment.get("local_url")?.as_str()?).ok()?;
    let local_base_url = local_publication_base_url(&local_url).ok()?;
    let open_url = deployment
        .get("url")
        .and_then(serde_json::Value::as_str)
        .or_else(|| publication.open_url())?;
    let tunnel_id = publication_tunnel_id(open_url)?;
    Some(RelayDisplayTunnelTarget {
        tunnel_id,
        slice_id: format!(
            "publication:{}:{}",
            publication.session_id(),
            publication.id()
        ),
        local_base_url,
        expires_at_ms,
        capabilities: publication_tunnel_capabilities(),
    })
}

pub(crate) async fn execute_register_workflow_publication_endpoint_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    request: RegisterWorkflowPublicationEndpointRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let publication = runtime_state
        .owned
        .session_store
        .read()
        .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
    if publication.created_by_user_id() != caller_user_id {
        return Err(super::KernelRuntimeOwnedState::deny_owner(
            caller_user_id,
            publication.created_by_user_id(),
            format!("workflow publication `{}`", request.publication_ref),
            "register workflow publication endpoint",
        ));
    }
    let preferred_tunnel_id = publication
        .deployment()
        .and_then(|deployment| deployment.pointer("/binding/deployment_id"))
        .and_then(serde_json::Value::as_str)
        .map(stable_deployment_tunnel_id);
    let served = served_publication_endpoint(
        config_projection,
        relay_state,
        &request,
        preferred_tunnel_id.as_deref(),
    )
    .await?;
    let mut deployment = publication
        .deployment()
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    deployment.insert("kind".to_string(), serde_json::json!(served.access.clone()));
    deployment.insert(
        "url".to_string(),
        serde_json::json!(served.open_url.clone()),
    );
    deployment.insert(
        "local_url".to_string(),
        serde_json::json!(request.local_url.clone()),
    );
    deployment.insert(
        "runtime_session_id".to_string(),
        serde_json::json!(request.runtime_session_id.clone()),
    );
    deployment.insert(
        "expires_at_ms".to_string(),
        serde_json::json!(served.expires_at_ms),
    );
    runtime_state.owned.workflow_register_publication_endpoint(
        request,
        caller_user_id,
        served.open_url,
        served.access,
        served.expires_at_ms,
        serde_json::Value::Object(deployment),
    )
}

struct ServedPublicationEndpoint {
    open_url: String,
    access: String,
    expires_at_ms: Option<u64>,
}

async fn served_publication_endpoint(
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    request: &RegisterWorkflowPublicationEndpointRequest,
    preferred_tunnel_id: Option<&str>,
) -> Result<ServedPublicationEndpoint, DaemonError> {
    let local_url = parse_local_publication_url(&request.local_url)?;
    if let Some(tunneled) = tunneled_publication_endpoint(
        config_projection,
        relay_state,
        request,
        &local_url,
        preferred_tunnel_id,
    )
    .await?
    {
        return Ok(tunneled);
    }
    Ok(ServedPublicationEndpoint {
        open_url: local_url.to_string(),
        access: "local".to_string(),
        expires_at_ms: None,
    })
}

async fn tunneled_publication_endpoint(
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    request: &RegisterWorkflowPublicationEndpointRequest,
    local_url: &url::Url,
    preferred_tunnel_id: Option<&str>,
) -> Result<Option<ServedPublicationEndpoint>, DaemonError> {
    let local_base_url = local_publication_base_url(local_url)?;
    let now_ms = crate::session::unix_epoch_ms();
    let ttl_ms = request.ttl_ms.unwrap_or(PUBLICATION_TUNNEL_TTL_MS);
    let expires_at_ms = now_ms.saturating_add(ttl_ms);
    let tunnel_id = preferred_tunnel_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("publication-{}", random_hex_id()));
    let (outgoing_tx, tunnel_url) = {
        let mut guard = relay_state.write().await;
        guard.prune_expired_display_tunnels(now_ms);
        let relay_url = guard
            .connected_relay_url()
            .or_else(|| config_projection.snapshot().relay_url);
        let Some(relay_url) = relay_url else {
            return Ok(None);
        };
        let Some(relay_base_url) = relay_display_base_url(&relay_url) else {
            return Ok(None);
        };
        let Some(outgoing_tx) = guard.outgoing_sender() else {
            return Ok(None);
        };
        let tunnel_url = relay_publication_endpoint_url(&relay_base_url, &tunnel_id, local_url)?;
        guard.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: tunnel_id.clone(),
            slice_id: format!(
                "publication:{}:{}",
                request.session_id, request.publication_ref
            ),
            local_base_url,
            expires_at_ms,
            capabilities: publication_tunnel_capabilities(),
        });
        (outgoing_tx, tunnel_url)
    };
    if outgoing_tx
        .try_send(RelayEnvelope::DaemonDisplayTunnelRegister {
            registration: RelayDisplayTunnelRegistration {
                tunnel_id: tunnel_id.clone(),
                expires_at_ms,
                capabilities: publication_tunnel_capabilities(),
            },
        })
        .is_err()
    {
        relay_state.write().await.remove_display_tunnel(&tunnel_id);
        return Ok(None);
    }
    Ok(Some(ServedPublicationEndpoint {
        open_url: tunnel_url,
        access: "tunnel".to_string(),
        expires_at_ms: Some(expires_at_ms),
    }))
}

fn parse_local_publication_url(local_url: &str) -> Result<url::Url, DaemonError> {
    let url = url::Url::parse(local_url).map_err(|error| DaemonError::LocalTransport {
        operation: "register workflow publication endpoint",
        message: format!("invalid publication local_url: {error}"),
    })?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err(DaemonError::LocalTransport {
            operation: "register workflow publication endpoint",
            message: "publication local_url must use http or https".to_string(),
        }),
    }
}

fn local_publication_base_url(local_url: &url::Url) -> Result<String, DaemonError> {
    let mut base = local_url.clone();
    base.set_path("");
    base.set_query(None);
    base.set_fragment(None);
    Ok(base.to_string())
}

fn relay_display_base_url(relay_url: &str) -> Option<url::Url> {
    let mut url = url::Url::parse(relay_url).ok()?;
    match url.scheme() {
        "ws" => {
            url.set_scheme("http").ok()?;
        }
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

fn relay_publication_endpoint_url(
    relay_base_url: &url::Url,
    tunnel_id: &str,
    local_url: &url::Url,
) -> Result<String, DaemonError> {
    let mut tunnel_url = relay_base_url.clone();
    let local_path = if local_url.path().is_empty() {
        "/"
    } else {
        local_url.path()
    };
    tunnel_url.set_path(&format!("/display/{tunnel_id}{local_path}"));
    tunnel_url.set_query(local_url.query());
    Ok(tunnel_url.to_string())
}

fn publication_tunnel_id(open_url: &str) -> Option<String> {
    let url = url::Url::parse(open_url).ok()?;
    let mut segments = url.path_segments()?;
    if segments.next()? != "display" {
        return None;
    }
    let tunnel_id = segments.next()?.trim();
    if tunnel_id.is_empty() {
        return None;
    }
    Some(tunnel_id.to_string())
}

fn publication_tunnel_capabilities() -> Vec<String> {
    PUBLICATION_TUNNEL_CAPABILITIES
        .iter()
        .map(|capability| (*capability).to_string())
        .collect()
}

fn stable_deployment_tunnel_id(deployment_id: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(deployment_id.as_bytes()));
    format!("publication-{}", &digest[..32])
}

fn random_hex_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::{relay_display_base_url, stable_deployment_tunnel_id};

    #[test]
    fn relay_display_base_url_maps_websocket_schemes_to_browser_schemes() {
        assert_eq!(
            relay_display_base_url("ws://127.0.0.1:43130/ws")
                .map(|url| url.to_string())
                .as_deref(),
            Some("http://127.0.0.1:43130/")
        );
        assert_eq!(
            relay_display_base_url("wss://relay.example.test/ws")
                .map(|url| url.to_string())
                .as_deref(),
            Some("https://relay.example.test/")
        );
        assert!(relay_display_base_url("http://relay.example.test").is_none());
    }

    #[test]
    fn deployment_tunnel_identity_is_stable_and_opaque() {
        let first = stable_deployment_tunnel_id("deployment-1");
        assert_eq!(first, stable_deployment_tunnel_id("deployment-1"));
        assert_ne!(first, stable_deployment_tunnel_id("deployment-2"));
        assert!(first.starts_with("publication-"));
        assert!(!first.contains("deployment-1"));
    }
}
