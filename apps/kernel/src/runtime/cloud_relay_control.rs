use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use base64::Engine;
use chariox_relay::protocol::DaemonRegistration;

pub(crate) const CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS: u64 = 300_000;
pub(crate) const CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudRuntimeTokenSubject {
    pub(crate) subject: String,
    pub(crate) subject_kind: &'static str,
    pub(crate) machine_id: Option<String>,
}

pub(crate) fn cloud_relay_profile_has_runtime_credentials(
    profile: &PersistedCloudRelayProfile,
) -> bool {
    profile.cloud_session_token.is_some() || profile.machine_credential.is_some()
}

pub(crate) fn cloud_relay_token_refresh_due(config: &DaemonConfig, now_ms: u64) -> bool {
    let Some(profile) = config.cloud_relay.as_ref() else {
        return false;
    };
    if !cloud_relay_profile_has_runtime_credentials(profile) {
        return false;
    }
    config.relay_url.as_deref() != Some(profile.relay_url.as_str())
        || config.relay_token.is_none()
        || !cloud_relay_token_matches_runtime_key(config)
        || profile
            .token_expires_at_ms
            .is_none_or(|expires_at| expires_at <= now_ms + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS)
}

pub(crate) fn cloud_relay_runtime_token_is_fresh(
    config: &DaemonConfig,
    profile: &PersistedCloudRelayProfile,
    now_ms: u64,
) -> bool {
    config.relay_url.as_deref() == Some(profile.relay_url.as_str())
        && config.relay_token.is_some()
        && cloud_relay_token_matches_runtime_key(config)
        && profile
            .token_expires_at_ms
            .is_some_and(|expires_at| expires_at > now_ms + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS)
}

fn cloud_relay_token_matches_runtime_key(config: &DaemonConfig) -> bool {
    let Some(token) = config.relay_token.as_deref() else {
        return false;
    };
    let Some(thumbprint) = relay_token_payload(token).and_then(|claims| {
        claims
            .get("public_key_thumbprint")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }) else {
        return false;
    };
    thumbprint == crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)
}

pub(crate) fn relay_token_payload(token: &str) -> Option<serde_json::Value> {
    if token.len() > 16_384 {
        return None;
    }
    let mut segments = token.trim().split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    let signature = segments.next()?;
    if header.is_empty() || payload.is_empty() || signature.is_empty() || segments.next().is_some()
    {
        return None;
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub(crate) fn relay_token_expiry_ms(token: &str) -> Option<u64> {
    relay_token_payload(token)?
        .get("exp")?
        .as_u64()?
        .checked_mul(1_000)
}

pub(crate) fn cloud_runtime_token_subject(
    config: &DaemonConfig,
    profile: &PersistedCloudRelayProfile,
) -> CloudRuntimeTokenSubject {
    if let Some(machine_id) = profile.machine_id.clone() {
        return CloudRuntimeTokenSubject {
            subject: machine_id.clone(),
            subject_kind: "machine",
            machine_id: Some(machine_id),
        };
    }
    CloudRuntimeTokenSubject {
        subject: config.daemon_id.clone(),
        subject_kind: "kernel",
        machine_id: None,
    }
}

pub(crate) fn cloud_kernel_presence_body(
    config: &DaemonConfig,
    profile: &PersistedCloudRelayProfile,
    online: bool,
    registration: Option<&DaemonRegistration>,
) -> Option<serde_json::Value> {
    let machine_id = profile.machine_id.clone()?;
    if !cloud_relay_profile_has_runtime_credentials(profile) {
        return None;
    }
    let mut body = serde_json::Map::new();
    if let Some(machine_credential) = profile.machine_credential.clone() {
        body.insert(
            "machineCredential".to_string(),
            serde_json::Value::String(machine_credential),
        );
    } else if let Some(session_token) = profile.cloud_session_token.clone() {
        body.insert(
            "sessionToken".to_string(),
            serde_json::Value::String(session_token),
        );
    }
    body.insert(
        "accountId".to_string(),
        serde_json::Value::String(profile.account_id.clone()),
    );
    body.insert(
        "realmId".to_string(),
        serde_json::Value::String(profile.realm_id.clone()),
    );
    body.insert(
        "machineId".to_string(),
        serde_json::Value::String(machine_id),
    );
    body.insert(
        "kernelId".to_string(),
        serde_json::Value::String(config.daemon_id.clone()),
    );
    if let Some(alias) = config.daemon_alias.clone() {
        body.insert("kernelAlias".to_string(), serde_json::Value::String(alias));
    }
    body.insert(
        "status".to_string(),
        serde_json::Value::String(if online { "ONLINE" } else { "OFFLINE" }.to_string()),
    );
    body.insert(
        "metadata".to_string(),
        serde_json::json!({
            "host": config.kernel_websocket_host,
            "port": config.kernel_websocket_port,
            "relay_public_key": config.relay_public_key,
            "local_daemon_protocol_version": crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            "managed_context_source_protocol_version":
                crate::managed_context::MANAGED_CONTEXT_SOURCE_PROTOCOL_VERSION,
            "kernel_started_at_ms": registration.map(|registration| registration.kernel_started_at_ms),
            "available_providers": registration
                .map(|registration| registration.available_providers.clone())
                .unwrap_or_default(),
            "provider_accounts": registration
                .map(|registration| registration.provider_accounts.clone())
                .unwrap_or_default(),
            "accepting_remote_leases": registration
                .map(|registration| registration.accepting_remote_leases)
                .unwrap_or(config.accept_remote_leases),
        }),
    );
    Some(serde_json::Value::Object(body))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::CharioxUserConfig;

    use super::*;

    fn profile() -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url: "https://cloud.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "acct".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_alias: None,
            machine_id: Some("machine-1".to_string()),
            machine_alias: None,
            machine_credential: Some("machine-secret".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: Some(200_000),
        }
    }

    fn config(profile: Option<PersistedCloudRelayProfile>) -> DaemonConfig {
        DaemonConfig {
            user_config_path: PathBuf::from("user.toml"),
            user_config: CharioxUserConfig::default(),
            daemon_id: "kernel-1".to_string(),
            host_machine_id: "host-1".to_string(),
            host_machine_alias: None,
            os_name: "test-os".to_string(),
            daemon_alias: Some("dev kernel".to_string()),
            relay_url: Some("wss://relay.test".to_string()),
            relay_token: Some(bound_token("public")),
            managed_slice_relay_recovery_token: None,
            managed_slice_relay_owner_public_key: None,
            cloud_relay: profile,
            relay_public_key: "public".to_string(),
            relay_private_key: "private".to_string(),
            relay_heartbeat_ms: 1_000,
            relay_request_timeout_ms: 2_000,
            accept_remote_leases: true,
            event_delivery_url: None,
            event_delivery_token: None,
            event_delivery_environment_id: "kernel-1".to_string(),
            event_registry_url: None,
            event_generator_management_targets: std::collections::BTreeMap::new(),
            os_user: "tester".to_string(),
            local_socket_path: PathBuf::from("kernel.sock"),
            kernel_websocket_host: "127.0.0.1".to_string(),
            kernel_websocket_port: 43118,
            kernel_websocket_queue_capacity: 128,
            kernel_websocket_write_delay_ms: 0,
            runtime_mcp_host: "127.0.0.1".to_string(),
            runtime_mcp_port: 43119,
            session_history_root: PathBuf::from("history"),
            session_history_read_delay_ms: 0,
            operational_history_read_delay_ms: 0,
            provider_catalog_read_delay_ms: 0,
            provider_process_list_delay_ms: 0,
            provider_process_idle_ttl_ms: 300_000,
            provider_process_orphan_ttl_ms: 30_000,
            provider_runtime_init_delay_ms: 0,
        }
    }

    fn bound_token(public_key: &str) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(public_key),
                "exp": 300,
            })
            .to_string(),
        );
        format!("header.{payload}.signature")
    }

    #[test]
    fn relay_token_payload_requires_exactly_three_nonempty_bounded_segments() {
        let token = bound_token("public");
        assert_eq!(relay_token_expiry_ms(&token), Some(300_000));
        assert!(relay_token_payload(&format!("{token}.extra")).is_none());
        assert!(relay_token_payload("header..signature").is_none());
        assert!(relay_token_payload(&"x".repeat(16_385)).is_none());
    }

    #[test]
    fn relay_token_refresh_due_requires_credentials_and_fresh_token() {
        assert!(!cloud_relay_token_refresh_due(&config(None), 100_000));

        let mut no_credentials = profile();
        no_credentials.machine_credential = None;
        no_credentials.cloud_session_token = None;
        assert!(!cloud_relay_token_refresh_due(
            &config(Some(no_credentials)),
            100_000
        ));

        let mut stale = profile();
        stale.token_expires_at_ms = Some(159_999);
        assert!(cloud_relay_token_refresh_due(&config(Some(stale)), 100_000));

        let fresh = profile();
        assert!(!cloud_relay_token_refresh_due(
            &config(Some(fresh.clone())),
            100_000
        ));
        assert!(cloud_relay_runtime_token_is_fresh(
            &config(Some(fresh.clone())),
            &fresh,
            100_000
        ));

        let mut wrong_key = config(Some(fresh.clone()));
        wrong_key.relay_token = Some(bound_token("different-public-key"));
        assert!(cloud_relay_token_refresh_due(&wrong_key, 100_000));
        assert!(!cloud_relay_runtime_token_is_fresh(
            &wrong_key, &fresh, 100_000
        ));
    }

    #[test]
    fn runtime_token_subject_prefers_machine_identity() {
        let machine_profile = profile();
        let subject =
            cloud_runtime_token_subject(&config(Some(machine_profile.clone())), &machine_profile);
        assert_eq!(subject.subject, "machine-1");
        assert_eq!(subject.subject_kind, "machine");
        assert_eq!(subject.machine_id.as_deref(), Some("machine-1"));

        let mut kernel_profile = profile();
        kernel_profile.machine_id = None;
        let subject =
            cloud_runtime_token_subject(&config(Some(kernel_profile.clone())), &kernel_profile);
        assert_eq!(subject.subject, "kernel-1");
        assert_eq!(subject.subject_kind, "kernel");
        assert_eq!(subject.machine_id, None);
    }

    #[test]
    fn presence_body_requires_machine_and_uses_machine_credentials_first() {
        let body = cloud_kernel_presence_body(&config(Some(profile())), &profile(), true, None)
            .expect("presence body should build");
        assert_eq!(body["machineCredential"], "machine-secret");
        assert_eq!(body["status"], "ONLINE");
        assert_eq!(body["kernelAlias"], "dev kernel");
        assert_eq!(body["metadata"]["host"], "127.0.0.1");
        assert_eq!(body["metadata"]["port"], 43118);
        assert_eq!(body["metadata"]["relay_public_key"], "public");
        assert_eq!(
            body["metadata"]["local_daemon_protocol_version"],
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
        );
        assert_eq!(
            body["metadata"]["managed_context_source_protocol_version"],
            crate::managed_context::MANAGED_CONTEXT_SOURCE_PROTOCOL_VERSION
        );
        assert_eq!(
            body["metadata"]["kernel_started_at_ms"],
            serde_json::Value::Null
        );
        assert_eq!(body["metadata"]["accepting_remote_leases"], true);

        let mut session_profile = profile();
        session_profile.machine_credential = None;
        session_profile.cloud_session_token = Some("session-token".to_string());
        let body = cloud_kernel_presence_body(
            &config(Some(session_profile.clone())),
            &session_profile,
            false,
            None,
        )
        .expect("session token presence body should build");
        assert_eq!(body["sessionToken"], "session-token");
        assert_eq!(body["status"], "OFFLINE");

        let mut no_machine = profile();
        no_machine.machine_id = None;
        assert_eq!(
            cloud_kernel_presence_body(&config(Some(no_machine.clone())), &no_machine, true, None),
            None
        );
    }

    #[test]
    fn presence_body_includes_sanitized_provider_account_metadata() {
        let registration = DaemonRegistration {
            auth_token: "relay-token".to_string(),
            daemon_id: "kernel-1".to_string(),
            machine_id: "machine-1".to_string(),
            machine_alias: None,
            os_name: None,
            kernel_started_at_ms: 0,
            daemon_alias: None,
            kernel_alias: None,
            public_key: "public".to_string(),
            capabilities: Vec::new(),
            available_providers: vec!["opencode".to_string()],
            provider_accounts: vec![chariox_relay::protocol::RelayProviderAccountSummary {
                provider: "opencode:openai".to_string(),
                state: "configured".to_string(),
                auth_type: Some("oauth".to_string()),
                account_id: Some("acct-1".to_string()),
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                alias: Some("worker-openai".to_string()),
            }],
            accepting_remote_leases: true,
            leased_agent_count: 0,
            local_session_count: 0,
        };
        let body = cloud_kernel_presence_body(
            &config(Some(profile())),
            &profile(),
            true,
            Some(&registration),
        )
        .expect("presence body should build");

        assert_eq!(body["metadata"]["available_providers"][0], "opencode");
        assert_eq!(
            body["metadata"]["provider_accounts"][0]["alias"],
            "worker-openai"
        );
        assert_eq!(
            body["metadata"]["provider_accounts"][0]["account_id"],
            "acct-1"
        );
        assert_eq!(body["metadata"]["relay_public_key"], "public");
        assert_eq!(body["metadata"]["kernel_started_at_ms"], 0);
        assert_eq!(body["metadata"]["accepting_remote_leases"], true);
    }
}
