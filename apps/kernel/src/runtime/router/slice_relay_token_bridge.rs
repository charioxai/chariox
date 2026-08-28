use std::collections::BTreeSet;

use serde::Deserialize;

use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::{
    issue_cloud_slice_recovery_token, issue_cloud_slice_runtime_token,
    MANAGED_SLICE_RELAY_RECOVERY_ACTIONS, MANAGED_SLICE_RELAY_RUNTIME_ACTIONS,
};
use crate::runtime::cloud_relay_control::{
    cloud_relay_profile_has_runtime_credentials, relay_token_expiry_ms, relay_token_payload,
    CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS,
};

pub(crate) struct ManagedSliceRelayTokenInstallRequest {
    pub slice_id: String,
    pub owner_kernel_id: String,
    pub owner_machine_id: String,
    pub relay_token: String,
    pub expires_at_ms: u64,
    pub relay_recovery_token: String,
    pub recovery_expires_at_ms: u64,
    pub owner_public_key: String,
}

pub(crate) struct ManagedSliceRelayIdentity {
    pub(crate) slice_id: String,
    pub(crate) relay_subject: String,
    pub(crate) owner_kernel_id: String,
    pub(crate) owner_machine_id: String,
}

#[derive(Deserialize)]
struct ManagedSliceRelayTokenClaims {
    sub: String,
    subject_kind: String,
    allowed_actions: Vec<String>,
    allowed_targets: Option<Vec<String>>,
    machine_id: Option<String>,
    public_key_thumbprint: Option<String>,
}

impl CommandRouter {
    pub(crate) fn managed_slice_relay_identity(&self) -> Option<ManagedSliceRelayIdentity> {
        let slice_id = std::env::var("CHARIOX_SLICE_ID").ok()?;
        let owner_kernel_id = std::env::var("CHARIOX_SLICE_OWNER_KERNEL_ID").ok()?;
        let owner_machine_id = std::env::var("CHARIOX_SLICE_OWNER_MACHINE_ID").ok()?;
        let relay_subject = self.config_projection.snapshot().daemon_alias?;
        if slice_id.trim().is_empty()
            || owner_kernel_id.trim().is_empty()
            || owner_machine_id.trim().is_empty()
            || !relay_subject.starts_with("slice:")
        {
            return None;
        }
        Some(ManagedSliceRelayIdentity {
            slice_id,
            relay_subject,
            owner_kernel_id,
            owner_machine_id,
        })
    }

    pub(crate) fn managed_slice_relay_token_refresh_due(&self) -> bool {
        let Some(identity) = self.managed_slice_relay_identity() else {
            return false;
        };
        let config = self.config_projection.snapshot();
        let Some(token) = config.relay_token else {
            return true;
        };
        if validate_managed_slice_relay_token(&token, &identity, &config.relay_public_key).is_err()
        {
            return true;
        }
        relay_token_expiry_ms(&token).is_none_or(|expires_at_ms| {
            expires_at_ms <= crate::session::unix_epoch_ms() + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS
        })
    }

    pub(crate) fn managed_slice_recovery_connection_token(
        &self,
        now_ms: u64,
    ) -> Result<Option<String>, DaemonError> {
        let Some(identity) = self.managed_slice_relay_identity() else {
            return Ok(None);
        };
        let config = self.config_projection.snapshot();
        managed_slice_recovery_connection_token_for_config(&config, &identity, now_ms)
    }

    pub(crate) async fn activate_managed_slice_recovery_connection(
        &self,
        now_ms: u64,
    ) -> Result<bool, DaemonError> {
        let Some(recovery_token) = self.managed_slice_recovery_connection_token(now_ms)? else {
            return Ok(false);
        };
        let relay_url = self
            .config_projection
            .snapshot()
            .relay_url
            .ok_or_else(|| slice_token_error("slice relay URL is not configured"))?;
        self.runtime_state
            .configure_relay(Some(relay_url), Some(recovery_token), false)
            .await?;
        Ok(true)
    }

    pub(crate) fn managed_slice_relay_owner_public_key(&self) -> Option<String> {
        self.config_projection
            .snapshot()
            .managed_slice_relay_owner_public_key
    }

    pub(crate) async fn install_managed_slice_relay_token(
        &self,
        request: ManagedSliceRelayTokenInstallRequest,
    ) -> Result<(), DaemonError> {
        let ManagedSliceRelayTokenInstallRequest {
            slice_id,
            owner_kernel_id,
            owner_machine_id,
            relay_token,
            expires_at_ms,
            relay_recovery_token,
            recovery_expires_at_ms,
            owner_public_key,
        } = request;
        let Some(identity) = self.managed_slice_relay_identity() else {
            return Err(slice_token_error(
                "kernel is not a managed slice relay worker",
            ));
        };
        if identity.slice_id != slice_id
            || identity.owner_kernel_id != owner_kernel_id
            || identity.owner_machine_id != owner_machine_id
        {
            return Err(slice_token_error(
                "slice relay token owner does not match worker bootstrap identity",
            ));
        }
        let now_ms = crate::session::unix_epoch_ms();
        if expires_at_ms <= now_ms + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS {
            return Err(slice_token_error("slice relay token expires too soon"));
        }
        let parsed_expiry = relay_token_expiry_ms(&relay_token)
            .ok_or_else(|| slice_token_error("slice relay token has no valid expiry"))?;
        if parsed_expiry.abs_diff(expires_at_ms) > 1_000 {
            return Err(slice_token_error(
                "slice relay token expiry does not match its signed claims",
            ));
        }
        if recovery_expires_at_ms <= now_ms + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS {
            return Err(slice_token_error(
                "slice relay recovery credential expires too soon",
            ));
        }
        let parsed_recovery_expiry = relay_token_expiry_ms(&relay_recovery_token)
            .ok_or_else(|| slice_token_error("slice relay recovery credential has no expiry"))?;
        if parsed_recovery_expiry.abs_diff(recovery_expires_at_ms) > 1_000 {
            return Err(slice_token_error(
                "slice relay recovery expiry does not match its signed claims",
            ));
        }
        let config = self.config_projection.snapshot();
        validate_managed_slice_relay_token(&relay_token, &identity, &config.relay_public_key)?;
        validate_managed_slice_recovery_token(
            &relay_recovery_token,
            &identity,
            &config.relay_public_key,
        )?;
        if owner_public_key.trim().is_empty() {
            return Err(slice_token_error("slice relay owner key is empty"));
        }
        let relay_url = self
            .config_projection
            .snapshot()
            .relay_url
            .ok_or_else(|| slice_token_error("slice relay URL is not configured"))?;
        self.runtime_state
            .configure_managed_slice_relay(
                relay_url,
                relay_token,
                relay_recovery_token,
                owner_public_key,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn refresh_managed_slice_relay_token(
        &self,
        slice_id: &str,
        owner_kernel_id: &str,
        worker_kernel_id: &str,
        worker_relay_subject: &str,
        worker_public_key: &str,
    ) -> Result<(String, u64, String, u64), DaemonError> {
        let config = self.config_projection.snapshot();
        if config.daemon_id != owner_kernel_id {
            return Err(slice_token_error(
                "slice relay token request targeted the wrong owner kernel",
            ));
        }
        let slice = self.runtime_state.resolve_slice(slice_id)?;
        let worker_identity_matches = match slice.worker_kernel_id.as_deref() {
            Some(recorded) => recorded == worker_kernel_id,
            None => slice.status == crate::slice::SliceStatus::Starting,
        };
        if slice.owner_kernel_id != owner_kernel_id
            || !worker_identity_matches
            || slice.worker_kernel_ref != worker_relay_subject
            || slice.worker_kernel_ref.trim().is_empty()
        {
            return Err(slice_token_error(
                "slice relay token request does not match the recorded worker",
            ));
        }
        if slice.worker_kernel_id.is_none() {
            self.runtime_state
                .claim_slice_starting_worker_identity(slice_id, worker_kernel_id)?;
        }
        if !self
            .relay_state
            .write()
            .await
            .claim_peer_public_key(worker_kernel_id, worker_public_key)
        {
            return Err(slice_token_error(
                "slice worker relay key changed during token refresh",
            ));
        }
        let profile = config
            .cloud_relay
            .filter(cloud_relay_profile_has_runtime_credentials)
            .ok_or_else(|| slice_token_error("slice owner has no Cloud relay profile"))?;
        let issued = issue_cloud_slice_runtime_token(
            &profile,
            &slice.worker_kernel_ref,
            owner_kernel_id,
            Some(worker_public_key),
        )
        .await?;
        let expires_at_ms = relay_token_expiry_ms(&issued.token)
            .ok_or_else(|| slice_token_error("Cloud returned a relay token without an expiry"))?;
        let recovery = issue_cloud_slice_recovery_token(
            &profile,
            &slice.worker_kernel_ref,
            owner_kernel_id,
            worker_public_key,
        )
        .await?;
        let recovery_expires_at_ms = relay_token_expiry_ms(&recovery.token).ok_or_else(|| {
            slice_token_error("Cloud returned a slice recovery token without an expiry")
        })?;
        Ok((
            issued.token,
            expires_at_ms,
            recovery.token,
            recovery_expires_at_ms,
        ))
    }
}

fn managed_slice_recovery_connection_token_for_config(
    config: &crate::config::DaemonConfig,
    identity: &ManagedSliceRelayIdentity,
    now_ms: u64,
) -> Result<Option<String>, DaemonError> {
    if let Some(current_token) = config.relay_token.as_deref() {
        let current_is_usable = relay_token_expiry_ms(current_token)
            .is_some_and(|expires_at_ms| expires_at_ms > now_ms)
            && (validate_managed_slice_relay_token(
                current_token,
                identity,
                &config.relay_public_key,
            )
            .is_ok()
                || validate_managed_slice_recovery_token(
                    current_token,
                    identity,
                    &config.relay_public_key,
                )
                .is_ok());
        if current_is_usable {
            return Ok(None);
        }
        if config.managed_slice_relay_recovery_token.is_none()
            && relay_token_expiry_ms(current_token)
                .is_some_and(|expires_at_ms| expires_at_ms > now_ms)
        {
            return Ok(None);
        }
    }
    let recovery_token = config
        .managed_slice_relay_recovery_token
        .clone()
        .ok_or_else(|| slice_token_error("slice relay recovery credential is missing"))?;
    validate_managed_slice_recovery_token(&recovery_token, identity, &config.relay_public_key)?;
    let expires_at_ms = relay_token_expiry_ms(&recovery_token)
        .ok_or_else(|| slice_token_error("slice relay recovery credential has no expiry"))?;
    if expires_at_ms <= now_ms + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS {
        return Err(slice_token_error(
            "slice relay recovery credential is expired or expires too soon",
        ));
    }
    if config
        .managed_slice_relay_owner_public_key
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err(slice_token_error("slice relay owner key is missing"));
    }
    Ok(Some(recovery_token))
}

fn validate_managed_slice_relay_token(
    token: &str,
    identity: &ManagedSliceRelayIdentity,
    relay_public_key: &str,
) -> Result<(), DaemonError> {
    let claims: ManagedSliceRelayTokenClaims = serde_json::from_value(
        relay_token_payload(token)
            .ok_or_else(|| slice_token_error("slice relay token payload is invalid"))?,
    )
    .map_err(|_| slice_token_error("slice relay token claims are incomplete"))?;
    let expected_actions = MANAGED_SLICE_RELAY_RUNTIME_ACTIONS
        .iter()
        .map(|action| (*action).to_string())
        .collect::<BTreeSet<_>>();
    let actual_actions = claims
        .allowed_actions
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(relay_public_key);
    if claims.sub != identity.relay_subject
        || claims.subject_kind != "kernel"
        || claims.machine_id.as_deref() != Some(identity.owner_machine_id.as_str())
        || claims.allowed_targets.as_deref()
            != Some(std::slice::from_ref(&identity.owner_kernel_id))
        || claims.public_key_thumbprint.as_deref() != Some(expected_thumbprint.as_str())
        || claims.allowed_actions.len() != expected_actions.len()
        || actual_actions != expected_actions
    {
        return Err(slice_token_error(
            "slice relay token is not key-bound and owner-scoped",
        ));
    }
    Ok(())
}

fn validate_managed_slice_recovery_token(
    token: &str,
    identity: &ManagedSliceRelayIdentity,
    relay_public_key: &str,
) -> Result<(), DaemonError> {
    let claims: ManagedSliceRelayTokenClaims = serde_json::from_value(
        relay_token_payload(token)
            .ok_or_else(|| slice_token_error("slice relay recovery payload is invalid"))?,
    )
    .map_err(|_| slice_token_error("slice relay recovery claims are incomplete"))?;
    let expected_actions = MANAGED_SLICE_RELAY_RECOVERY_ACTIONS
        .iter()
        .map(|action| (*action).to_string())
        .collect::<BTreeSet<_>>();
    let actual_actions = claims
        .allowed_actions
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(relay_public_key);
    if claims.sub != identity.relay_subject
        || claims.subject_kind != "kernel"
        || claims.machine_id.as_deref() != Some(identity.owner_machine_id.as_str())
        || claims.allowed_targets.as_deref()
            != Some(std::slice::from_ref(&identity.owner_kernel_id))
        || claims.public_key_thumbprint.as_deref() != Some(expected_thumbprint.as_str())
        || claims.allowed_actions.len() != expected_actions.len()
        || actual_actions != expected_actions
    {
        return Err(slice_token_error(
            "slice relay recovery credential is not key-bound and owner-scoped",
        ));
    }
    Ok(())
}

fn slice_token_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice relay token",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;

    fn token(payload: serde_json::Value) -> String {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).expect("payload should encode"));
        format!("header.{encoded}.signature")
    }

    fn identity() -> ManagedSliceRelayIdentity {
        ManagedSliceRelayIdentity {
            slice_id: "slice-1".to_string(),
            relay_subject: "slice:dev".to_string(),
            owner_kernel_id: "kernel-owner".to_string(),
            owner_machine_id: "machine-owner".to_string(),
        }
    }

    fn active_claims(public_key: &str) -> serde_json::Value {
        serde_json::json!({
            "sub": "slice:dev",
            "subject_kind": "kernel",
            "allowed_actions": [
                "daemon.register",
                "daemon.heartbeat",
                "packet.route",
                "peer.request",
                "peer.event",
            ],
            "allowed_targets": ["kernel-owner"],
            "machine_id": "machine-owner",
            "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(public_key),
            "exp": 300,
        })
    }

    fn recovery_claims(public_key: &str) -> serde_json::Value {
        serde_json::json!({
            "sub": "slice:dev",
            "subject_kind": "kernel",
            "allowed_actions": [
                "daemon.register",
                "daemon.heartbeat",
                "peer.request",
            ],
            "allowed_targets": ["kernel-owner"],
            "machine_id": "machine-owner",
            "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(public_key),
            "exp": 2_592_000,
        })
    }

    #[test]
    fn managed_slice_token_requires_exact_subject_target_actions_machine_and_key() {
        let public_key = "worker-public";
        validate_managed_slice_relay_token(
            &token(active_claims(public_key)),
            &identity(),
            public_key,
        )
        .expect("narrow active token should pass");

        for field in [
            "sub",
            "subject_kind",
            "allowed_targets",
            "machine_id",
            "public_key_thumbprint",
        ] {
            let mut claims = active_claims(public_key);
            claims[field] = serde_json::json!("wrong");
            assert!(
                validate_managed_slice_relay_token(&token(claims), &identity(), public_key,)
                    .is_err()
            );
        }

        let mut broad = active_claims(public_key);
        broad["allowed_actions"] = serde_json::json!([
            "daemon.register",
            "daemon.heartbeat",
            "packet.route",
            "peer.request",
            "peer.event",
            "client.metadata.read",
        ]);
        assert!(
            validate_managed_slice_relay_token(&token(broad), &identity(), public_key,).is_err()
        );
    }

    #[test]
    fn expired_active_token_uses_persisted_same_key_recovery_after_restart() {
        let public_key = "worker-public";
        let recovery_token = token(recovery_claims(public_key));
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_public_key = public_key.to_string();
        config.relay_token = Some(token(active_claims(public_key)));
        config.managed_slice_relay_recovery_token = Some(recovery_token.clone());
        config.managed_slice_relay_owner_public_key = Some("owner-public".to_string());

        assert_eq!(
            managed_slice_recovery_connection_token_for_config(&config, &identity(), 600_000,)
                .expect("restart after active-token expiry should recover"),
            Some(recovery_token.clone()),
        );

        config.relay_token = Some(recovery_token);
        assert_eq!(
            managed_slice_recovery_connection_token_for_config(&config, &identity(), 600_000,)
                .expect("an already active recovery connection should be retained"),
            None,
        );
    }
}
