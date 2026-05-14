use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    CreatePairingInviteRequest, CreateTerminalPairingLinkRequest, JoinPairingInviteRequest,
    JoinTerminalPairingLinkRequest, LocalDaemonRequest, LocalDaemonResponse, PairingInviteIntent,
    PairingInviteRecord, PairingJoinRecord, TerminalPairingLinkRecord, TerminalType,
};
use crate::runtime::cloud_api_client::{
    is_stale_cloud_link_error, issue_cloud_runtime_token, post_cloud_json,
    CloudPairingTokenResponse,
};
use crate::runtime::invite_tokens::{
    decode_pairing_invite_token, encode_pairing_invite_token, encode_terminal_pairing_link,
    PairingInviteToken,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::terminal_pairings::{
    execute_list_paired_clients_request, execute_list_terminals_request,
    execute_record_paired_client_request, execute_revoke_paired_client_request,
    public_key_thumbprint, terminal_record, terminal_type_from_str,
};
use crate::session::unix_epoch_ms;

pub(crate) async fn execute_pairing_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::CreatePairingInvite(request) => {
            execute_create_pairing_invite_request(config_projection, request).await
        }
        LocalDaemonRequest::JoinPairingInvite(request) => {
            execute_join_pairing_invite_request(
                app,
                config_projection,
                provider_catalog_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::CreateTerminalPairingLink(request) => {
            execute_create_terminal_pairing_link_request(app, config_projection, request).await
        }
        LocalDaemonRequest::JoinTerminalPairingLink(request) => {
            execute_join_terminal_pairing_link_request(config_projection, request).await
        }
        LocalDaemonRequest::ListTerminals(_) => execute_list_terminals_request(),
        LocalDaemonRequest::ListPairedClients(_) => execute_list_paired_clients_request(),
        LocalDaemonRequest::RecordPairedClient(request) => {
            execute_record_paired_client_request(request, unix_epoch_ms)
        }
        LocalDaemonRequest::RevokePairedClient(request) => {
            execute_revoke_paired_client_request(request)
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "pairing request",
            message: "unsupported pairing request".to_string(),
        }),
    }
}

pub(crate) async fn execute_create_pairing_invite_request(
    config_projection: &DaemonConfigProjectionStore,
    request: CreatePairingInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "create pairing invite",
            message: "relay URL must be configured before creating an invite".to_string(),
        })?;
    let relay_token = config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "create pairing invite",
            message: "relay token must be configured before creating an invite".to_string(),
        })?;
    let issued_at_ms = current_unix_ms();
    let expires_at_ms =
        issued_at_ms.saturating_add(request.expires_in_ms.unwrap_or(15 * 60 * 1000));
    let invite_id = random_hex_id();
    let token = PairingInviteToken {
        version: 1,
        intent: request.intent,
        invite_id: invite_id.clone(),
        relay_url: relay_url.clone(),
        relay_token,
        target_daemon_id: config.daemon_id.clone(),
        target_daemon_alias: config.daemon_alias.clone().or(request.alias),
        issuer_machine_id: config.host_machine_id,
        issued_at_ms,
        expires_at_ms,
        terminal_type: request
            .terminal_type
            .map(|terminal_type| terminal_type.as_str().to_string()),
        pairing_code: request.terminal_type.map(|_| random_pairing_code()),
        terminal_id: None,
    };
    let invite_token = encode_pairing_invite_token(&token)?;
    Ok(LocalDaemonResponse::PairingInviteCreated {
        invite: PairingInviteRecord {
            intent: token.intent,
            invite_id,
            invite_token,
            relay_url,
            target_daemon_id: token.target_daemon_id,
            target_daemon_alias: token.target_daemon_alias,
            issued_at_ms,
            expires_at_ms,
        },
    })
}

pub(crate) async fn execute_create_terminal_pairing_link_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: CreateTerminalPairingLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let terminal_type = request.terminal_type.unwrap_or(TerminalType::Cli);
    let config = config_projection.snapshot();
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "create terminal pairing link",
            message: "relay URL must be configured before creating a terminal pairing link"
                .to_string(),
        })?;
    let issued_at_ms = current_unix_ms();
    let expires_at_ms =
        issued_at_ms.saturating_add(request.expires_in_ms.unwrap_or(15 * 60 * 1000));
    let invite_id = random_hex_id();
    let pairing_code = random_pairing_code();
    let terminal_id = format!("{}-{}", terminal_type.as_str(), random_hex_id());
    let target_daemon_id = config.daemon_id.clone();
    let target_daemon_alias = config.daemon_alias.clone().or(request.alias);
    let relay_token = if let Some(profile) = config.cloud_relay.clone().filter(|profile| {
        profile.relay_url == relay_url
            && (profile.cloud_session_token.is_some() || profile.machine_credential.is_some())
    }) {
        let pairing: CloudPairingTokenResponse = match post_cloud_json(
            profile.api_url.clone(),
            "/pairing-tokens",
            serde_json::json!({
                "accountId": profile.account_id,
                "createdByUserId": profile.user_id,
                "subjectKind": "client",
            }),
        )
        .await
        {
            Ok(pairing) => pairing,
            Err(error) => {
                clear_cloud_profile_if_stale(app, config_projection, &error).await?;
                return Err(error);
            }
        };
        if let Err(error) = post_cloud_json::<serde_json::Value>(
            profile.api_url.clone(),
            "/clients/pair",
            serde_json::json!({
                "accountId": profile.account_id,
                "token": pairing.token,
                "clientId": terminal_id,
                "userId": profile.user_id,
                "alias": format!("{} terminal", terminal_type.as_str()),
            }),
        )
        .await
        {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
        let mut allowed_targets = vec![target_daemon_id.clone()];
        if let Some(alias) = target_daemon_alias.clone() {
            if !allowed_targets.iter().any(|target| target == &alias) {
                allowed_targets.push(alias);
            }
        }
        match issue_cloud_runtime_token(
            &profile,
            &terminal_id,
            "client",
            Some(allowed_targets),
            Some(terminal_id.clone()),
            profile
                .machine_credential
                .as_ref()
                .and(profile.machine_id.clone()),
            None,
        )
        .await
        {
            Ok(issued) => issued.token,
            Err(error) => {
                clear_cloud_profile_if_stale(app, config_projection, &error).await?;
                return Err(error);
            }
        }
    } else {
        config
            .relay_token
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "create terminal pairing link",
                message: "relay token must be configured before creating a terminal pairing link"
                    .to_string(),
            })?
    };
    let token = PairingInviteToken {
        version: 1,
        intent: PairingInviteIntent::Client,
        invite_id: invite_id.clone(),
        relay_url: relay_url.clone(),
        relay_token,
        target_daemon_id,
        target_daemon_alias,
        issuer_machine_id: config.host_machine_id,
        issued_at_ms,
        expires_at_ms,
        terminal_type: Some(terminal_type.as_str().to_string()),
        pairing_code: Some(pairing_code.clone()),
        terminal_id: Some(terminal_id.clone()),
    };
    let pairing_link = encode_terminal_pairing_link(&token)?;
    let _ = crate::config::DaemonConfig::record_paired_terminal(
        terminal_id.clone(),
        format!("pairing-link:{invite_id}"),
        token.target_daemon_alias.clone(),
        issued_at_ms,
        terminal_type.as_str(),
    )?;
    Ok(LocalDaemonResponse::TerminalPairingLinkCreated {
        pairing: TerminalPairingLinkRecord {
            terminal_id,
            pairing_link,
            pairing_code,
            invite_id,
            relay_url,
            target_daemon_id: token.target_daemon_id,
            target_daemon_alias: token.target_daemon_alias,
            terminal_type,
            issued_at_ms,
            expires_at_ms,
        },
    })
}

pub(crate) async fn execute_join_pairing_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: JoinPairingInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let token = decode_pairing_invite_token(&request.invite_token)?;
    let now_ms = current_unix_ms();
    if token.expires_at_ms <= now_ms {
        return Err(DaemonError::LocalTransport {
            operation: "join pairing invite",
            message: "pairing invite is expired".to_string(),
        });
    }
    let config = config_projection.snapshot();
    let subject_id = request.subject_id.unwrap_or_else(|| match token.intent {
        PairingInviteIntent::Client => token
            .terminal_id
            .clone()
            .unwrap_or_else(|| format!("client-{}", random_hex_id())),
        PairingInviteIntent::Machine => config.host_machine_id.clone(),
    });
    let public_key_thumbprint = request
        .public_key_thumbprint
        .unwrap_or_else(|| public_key_thumbprint(&config.relay_public_key));
    match token.intent {
        PairingInviteIntent::Client => {
            crate::config::DaemonConfig::record_paired_terminal(
                subject_id.clone(),
                public_key_thumbprint.clone(),
                request.alias.clone(),
                now_ms,
                token.terminal_type.as_deref().unwrap_or("cli"),
            )?;
        }
        PairingInviteIntent::Machine => {
            crate::config::DaemonConfig::pair_remote_machine(
                subject_id.clone(),
                public_key_thumbprint.clone(),
                now_ms,
            )?;
            {
                let mut app = app.lock().await;
                app.configure_relay(Some(token.relay_url.clone()), Some(token.relay_token))?;
                app.invalidate_provider_catalog_cache();
                config_projection.update(app.config().clone());
            }
            provider_catalog_projection.invalidate();
        }
    }
    Ok(LocalDaemonResponse::PairingInviteJoined {
        pairing: PairingJoinRecord {
            intent: token.intent,
            subject_id,
            relay_url: token.relay_url,
            target_daemon_id: token.target_daemon_id,
            alias: request.alias,
            public_key_thumbprint,
            paired_at_ms: now_ms,
        },
    })
}

pub(crate) async fn execute_join_terminal_pairing_link_request(
    config_projection: &DaemonConfigProjectionStore,
    request: JoinTerminalPairingLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let token = decode_pairing_invite_token(&request.pairing_link)?;
    let now_ms = current_unix_ms();
    if token.expires_at_ms <= now_ms {
        return Err(DaemonError::LocalTransport {
            operation: "join terminal pairing link",
            message: "terminal pairing link is expired".to_string(),
        });
    }
    if token.intent != PairingInviteIntent::Client {
        return Err(DaemonError::LocalTransport {
            operation: "join terminal pairing link",
            message: "pairing link is not for a terminal".to_string(),
        });
    }
    let config = config_projection.snapshot();
    let terminal_type = request
        .terminal_type
        .or_else(|| token.terminal_type.as_deref().map(terminal_type_from_str))
        .unwrap_or(TerminalType::Cli);
    let terminal_id = request
        .terminal_id
        .or(token.terminal_id.clone())
        .unwrap_or_else(|| format!("{}-{}", terminal_type.as_str(), random_hex_id()));
    let public_key_thumbprint = public_key_thumbprint(&config.relay_public_key);
    let client = crate::config::DaemonConfig::record_paired_terminal(
        terminal_id.clone(),
        public_key_thumbprint.clone(),
        request.alias.clone(),
        now_ms,
        terminal_type.as_str(),
    )?;
    let terminal = terminal_record(client);
    Ok(LocalDaemonResponse::TerminalPairingLinkJoined {
        terminal,
        pairing: PairingJoinRecord {
            intent: PairingInviteIntent::Client,
            subject_id: terminal_id,
            relay_url: token.relay_url,
            target_daemon_id: token.target_daemon_id,
            alias: request.alias,
            public_key_thumbprint,
            paired_at_ms: now_ms,
        },
    })
}

async fn clear_cloud_profile_if_stale(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    error: &DaemonError,
) -> Result<(), DaemonError> {
    if !is_stale_cloud_link_error(error) {
        return Ok(());
    }
    {
        let mut app = app.lock().await;
        app.persist_cloud_relay_profile(None)?;
    }
    config_projection.update({
        let app = app.lock().await;
        app.config().clone()
    });
    Ok(())
}

fn random_hex_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn random_pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let value: String = bytes
        .iter()
        .map(|byte| ALPHABET[(*byte as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &value[..4], &value[4..])
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
