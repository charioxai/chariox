//! Cloud terminal clients use account pairing or a capability scoped to one machine.

use sha2::{Digest, Sha256};

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;

use super::{
    issue_cloud_account_client_runtime_token, issue_cloud_runtime_token, post_cloud_json,
    request_account_pairing_token, CloudRuntimeTokenRequestOptions, CloudRuntimeTokenResponse,
};

#[derive(Default)]
pub(crate) struct CloudTerminalClientOptions {
    pub(crate) pair_account_client: bool,
    pub(crate) client_alias: Option<String>,
    pub(crate) target_alias: Option<String>,
    pub(crate) ttl_ms: Option<u64>,
    pub(crate) session_id: Option<String>,
    pub(crate) public_key_thumbprint: Option<String>,
}

pub(crate) async fn issue_cloud_terminal_client_token(
    profile: &PersistedCloudRelayProfile,
    requested_client_id: &str,
    target_daemon_id: &str,
    options: CloudTerminalClientOptions,
) -> Result<(String, CloudRuntimeTokenResponse), DaemonError> {
    if profile.kernel_credential.is_some() {
        let kernel_id = profile
            .kernel_id
            .as_deref()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| client_scope_error("kernel enrollment identity missing"))?;
        let key = options
            .public_key_thumbprint
            .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| client_scope_error("terminal pivot requires a public key binding"))?;
        if requested_client_id.trim().is_empty() || target_daemon_id.trim().is_empty() {
            return Err(client_scope_error(
                "terminal and exact target identities are required",
            ));
        }
        let client_id = kernel_client_subject(kernel_id, requested_client_id)?;
        let issued = issue_cloud_runtime_token(
            profile,
            &client_id,
            "client",
            CloudRuntimeTokenRequestOptions {
                client_id: Some(client_id.clone()),
                machine_id: profile.machine_id.clone(),
                ttl_ms: Some(options.ttl_ms.unwrap_or(300_000).min(300_000)),
                allow_unpaired_client_subject: true,
                allowed_targets: Some(vec![target_daemon_id.to_string()]),
                public_key_thumbprint: Some(key),
                ..Default::default()
            },
        )
        .await?;
        return Ok((client_id, issued));
    }
    let account_authority = profile
        .cloud_session_token
        .as_deref()
        .is_some_and(|token| !token.trim().is_empty());
    let client_id = if account_authority {
        requested_client_id.to_string()
    } else {
        if profile
            .machine_credential
            .as_deref()
            .is_none_or(|credential| credential.trim().is_empty())
        {
            return Err(client_scope_error(
                "Cloud terminal pairing requires active credentials",
            ));
        }
        machine_client_subject(
            profile.machine_id.as_deref().unwrap_or_default(),
            requested_client_id,
        )?
    };
    if account_authority && options.pair_account_client {
        let pairing = request_account_pairing_token(profile, "client").await?;
        let mut body = serde_json::json!({
            "accountId": profile.account_id,
            "token": pairing.token,
            "clientId": client_id,
            "userId": profile.user_id,
        });
        if let Some(alias) = options.client_alias {
            body["alias"] = serde_json::Value::String(alias);
        }
        post_cloud_json::<serde_json::Value>(profile.api_url.clone(), "/clients/pair", body)
            .await?;
    }
    let mut targets = vec![target_daemon_id.to_string()];
    if account_authority {
        if let Some(alias) = options.target_alias {
            if !targets.contains(&alias) {
                targets.push(alias);
            }
        }
    }
    let token_options = CloudRuntimeTokenRequestOptions {
        ttl_ms: options.ttl_ms,
        client_id: Some(client_id.clone()),
        machine_id: if account_authority {
            None
        } else {
            profile.machine_id.clone()
        },
        allow_unpaired_client_subject: !account_authority,
        allowed_targets: Some(targets),
        session_id: options.session_id,
        public_key_thumbprint: options.public_key_thumbprint,
        ..CloudRuntimeTokenRequestOptions::default()
    };
    let issued = if account_authority {
        issue_cloud_account_client_runtime_token(profile, &client_id, token_options).await?
    } else {
        issue_cloud_runtime_token(profile, &client_id, "client", token_options).await?
    };
    Ok((client_id, issued))
}

fn kernel_client_subject(kernel_id: &str, original_client_id: &str) -> Result<String, DaemonError> {
    let prefix = format!("kernel-client:{:x}:", Sha256::digest(kernel_id.as_bytes()));
    if original_client_id.starts_with("kernel-client:") {
        let valid = original_client_id
            .strip_prefix(&prefix)
            .is_some_and(|suffix| {
                suffix.len() == 64
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            });
        if valid {
            return Ok(original_client_id.to_string());
        }
        return Err(client_scope_error(
            "Terminal client identity is not owned by this kernel",
        ));
    }
    Ok(format!(
        "{prefix}{:x}",
        Sha256::digest(original_client_id.as_bytes())
    ))
}

pub(crate) fn machine_client_subject(
    machine_id: &str,
    original_client_id: &str,
) -> Result<String, DaemonError> {
    if machine_id.trim().is_empty() || original_client_id.trim().is_empty() {
        return Err(client_scope_error(
            "Machine and client identities are required",
        ));
    }
    let prefix = format!(
        "machine-client:{:x}:",
        Sha256::digest(machine_id.as_bytes())
    );
    if original_client_id.starts_with("machine-client:") {
        let valid = original_client_id
            .strip_prefix(&prefix)
            .is_some_and(|suffix| {
                suffix.len() == 64
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            });
        if valid {
            return Ok(original_client_id.to_string());
        }
        return Err(client_scope_error(
            "Terminal client identity is not owned by this machine",
        ));
    }
    Ok(format!(
        "{prefix}{:x}",
        Sha256::digest(original_client_id.as_bytes())
    ))
}

fn client_scope_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "issue Cloud terminal client token",
        message: message.into(),
    }
}

#[cfg(test)]
#[path = "terminal_client_tests.rs"]
mod tests;
