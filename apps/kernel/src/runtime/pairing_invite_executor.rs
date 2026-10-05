use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;

use crate::error::DaemonError;
use crate::local::{
    CreatePairingInviteRequest, CreateTerminalPairingLinkRequest, JoinPairingInviteRequest,
    JoinTerminalPairingLinkRequest, LocalDaemonRequest, LocalDaemonResponse, PairingInviteIntent,
    PairingInviteRecord, PairingJoinRecord, TerminalPairingLinkRecord, TerminalType,
};
use crate::runtime::cloud_api_client::{
    issue_cloud_terminal_client_token, CloudTerminalClientOptions,
};
use crate::runtime::cloud_relay_connection_executor::require_cloud_relay_token_key_binding;
use crate::runtime::cloud_relay_profile_store::clear_cloud_profile_if_stale;
use crate::runtime::invite_tokens::{
    decode_pairing_invite_token, encode_pairing_invite_token, encode_terminal_pairing_link,
    PairingInviteToken,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_pairings::{
    execute_list_paired_clients_request, execute_list_terminals_request,
    execute_record_paired_client_request, execute_revoke_paired_client_request,
    public_key_thumbprint, terminal_record, terminal_type_from_str,
};
use crate::session::unix_epoch_ms;

pub(crate) async fn execute_pairing_request(
    runtime_state: &KernelRuntimeState,
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
                runtime_state,
                config_projection,
                provider_catalog_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::CreateTerminalPairingLink(request) => {
            execute_create_terminal_pairing_link_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::JoinTerminalPairingLink(request) => {
            execute_join_terminal_pairing_link_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::ListTerminals(_)
            if super::self_host_terminal_grants::required(&config_projection.snapshot()) =>
        {
            Ok(LocalDaemonResponse::TerminalsListed {
                terminals: super::self_host_terminal_grants::entries(
                    &config_projection.snapshot(),
                )?
                .into_iter()
                .map(terminal_record)
                .collect(),
            })
        }
        LocalDaemonRequest::ListTerminals(_) => execute_list_terminals_request(),
        LocalDaemonRequest::ListPairedClients(_)
            if super::self_host_terminal_grants::required(&config_projection.snapshot()) =>
        {
            Ok(LocalDaemonResponse::PairedClientsListed {
                clients: super::self_host_terminal_grants::entries(&config_projection.snapshot())?
                    .into_iter()
                    .map(super::terminal_pairings::paired_client_record)
                    .collect(),
            })
        }
        LocalDaemonRequest::ListPairedClients(_) => execute_list_paired_clients_request(),
        LocalDaemonRequest::RecordPairedClient(request) => {
            execute_record_paired_client_request(request, unix_epoch_ms)
        }
        LocalDaemonRequest::RevokePairedClient(request)
            if super::self_host_terminal_grants::required(&config_projection.snapshot()) =>
        {
            Ok(LocalDaemonResponse::PairedClientRevoked {
                client: super::terminal_pairings::paired_client_record(
                    super::self_host_terminal_grants::revoke(
                        &config_projection.snapshot(),
                        &request.client_id,
                    )?,
                ),
            })
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
    if config
        .cloud_relay
        .as_ref()
        .is_some_and(|profile| profile.kernel_credential.is_some())
        || super::self_host_terminal_grants::pairing_transport_token(relay_token.clone())
            != relay_token
    {
        return Err(DaemonError::LocalTransport {
            operation: "create pairing invite",
            message: "scoped kernel credentials cannot be exported in an invite; use an operator-issued transport credential and a terminal pairing link".into(),
        });
    }
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
        issuer_machine_id: config.host_machine_id.clone(),
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
    runtime_state: &KernelRuntimeState,
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
    let mut terminal_id = format!("{}-{}", terminal_type.as_str(), random_hex_id());
    let target_daemon_id = config.daemon_id.clone();
    let target_daemon_alias = config.daemon_alias.clone().or(request.alias);
    // Enrollment can issue a CLIENT grant only after the receiving terminal
    // supplies its key. Bootstrap belongs to that terminal's own client profile.
    let relay_token = if config.cloud_relay.as_ref().is_some_and(|profile| {
        profile.relay_url == relay_url && profile.kernel_credential.is_some()
    }) {
        "cloud-client-token-required".into()
    } else if let Some(profile) = config.cloud_relay.clone().filter(|profile| {
        profile.relay_url == relay_url
            && (profile.cloud_session_token.is_some() || profile.machine_credential.is_some())
    }) {
        match issue_cloud_terminal_client_token(
            &profile,
            &terminal_id,
            &target_daemon_id,
            CloudTerminalClientOptions {
                pair_account_client: true,
                client_alias: Some(format!("{} terminal", terminal_type.as_str())),
                target_alias: target_daemon_alias.clone(),
                ..CloudTerminalClientOptions::default()
            },
        )
        .await
        {
            Ok((qualified_id, issued)) => {
                terminal_id = qualified_id;
                issued.token
            }
            Err(error) => {
                clear_cloud_profile_if_stale(runtime_state, &error).await?;
                return Err(error);
            }
        }
    } else {
        super::self_host_terminal_grants::pairing_transport_token(
            config
                .relay_token
                .clone()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "create terminal pairing link",
                    message:
                        "relay token must be configured before creating a terminal pairing link"
                            .to_string(),
                })?,
        )
    };
    let relay_token = if super::self_host_terminal_grants::required(&config) {
        super::self_host_terminal_grants::pairing_transport_token(relay_token)
    } else {
        relay_token
    };
    let token = PairingInviteToken {
        version: 1,
        intent: PairingInviteIntent::Client,
        invite_id: invite_id.clone(),
        relay_url: relay_url.clone(),
        relay_token,
        target_daemon_id,
        target_daemon_alias,
        issuer_machine_id: config.host_machine_id.clone(),
        issued_at_ms,
        expires_at_ms,
        terminal_type: Some(terminal_type.as_str().to_string()),
        pairing_code: Some(pairing_code.clone()),
        terminal_id: Some(terminal_id.clone()),
    };
    let pairing_link = encode_terminal_pairing_link(&token)?;
    if super::self_host_terminal_grants::required(&config) {
        super::self_host_terminal_grants::register(
            &config,
            &pairing_link,
            &terminal_id,
            expires_at_ms,
            terminal_type.as_str(),
            token.target_daemon_alias.clone(),
        )?;
    } else {
        let _ = crate::config::DaemonConfig::record_paired_terminal(
            terminal_id.clone(),
            format!("pairing-link:{invite_id}"),
            token.target_daemon_alias.clone(),
            issued_at_ms,
            terminal_type.as_str(),
        )?;
    }
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
    runtime_state: &KernelRuntimeState,
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
            runtime_state
                .configure_relay(Some(token.relay_url.clone()), Some(token.relay_token), true)
                .await?;
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
    runtime_state: &KernelRuntimeState,
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
    if config
        .cloud_relay
        .as_ref()
        .is_some_and(|profile| profile.kernel_credential.is_some())
        && (token.target_daemon_id != config.daemon_id
            || config.relay_url.as_deref() != Some(token.relay_url.as_str())
            || request.public_key_thumbprint.is_none())
    {
        return Err(DaemonError::LocalTransport {
            operation: "join terminal pairing link",
            message:
                "enrolled terminal pairing requires the receiving key and this exact kernel/relay"
                    .into(),
        });
    }
    let terminal_type = request
        .terminal_type
        .or_else(|| token.terminal_type.as_deref().map(terminal_type_from_str))
        .unwrap_or(TerminalType::Cli);
    let mut terminal_id = request
        .terminal_id
        .or(token.terminal_id.clone())
        .unwrap_or_else(|| format!("{}-{}", terminal_type.as_str(), random_hex_id()));
    let joined_thumbprint = request
        .public_key_thumbprint
        .clone()
        .unwrap_or_else(|| public_key_thumbprint(&config.relay_public_key));
    if super::self_host_terminal_grants::required(&config) {
        if token.target_daemon_id != config.daemon_id
            || config.relay_url.as_deref() != Some(token.relay_url.as_str())
        {
            return Err(DaemonError::LocalTransport {
                operation: "join terminal pairing link",
                message: "pairing link targets another kernel or relay".into(),
            });
        }
        if joined_thumbprint.len() != 64
            || !joined_thumbprint
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(DaemonError::LocalTransport {
                operation: "join terminal pairing link",
                message: "terminal pairing requires a SHA-256 key thumbprint".into(),
            });
        }
        let grant = super::self_host_terminal_grants::redeem(
            &config,
            &request.pairing_link,
            crate::config::PersistedClientPairing {
                client_id: terminal_id.clone(),
                public_key_thumbprint: joined_thumbprint.clone(),
                alias: request.alias.clone(),
                paired_at_ms: now_ms,
                terminal_type: terminal_type.as_str().into(),
                revoked: false,
            },
        )?;
        return Ok(LocalDaemonResponse::TerminalPairingLinkJoined {
            terminal: terminal_record(grant),
            pairing: PairingJoinRecord {
                intent: PairingInviteIntent::Client,
                subject_id: terminal_id,
                relay_url: token.relay_url,
                target_daemon_id: token.target_daemon_id,
                alias: request.alias,
                public_key_thumbprint: joined_thumbprint,
                paired_at_ms: now_ms,
            },
            relay_token: None,
            kernel_pairing: request.public_key_thumbprint.is_some(),
        });
    }
    let relay_token = if request.public_key_thumbprint.is_some() {
        if joined_thumbprint.len() != 64
            || !joined_thumbprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(DaemonError::LocalTransport {
                operation: "join terminal pairing link",
                message: "CLI public key thumbprint must be a lowercase SHA-256 hex value"
                    .to_string(),
            });
        }
        let profile = config
            .cloud_relay
            .clone()
            .filter(|profile| {
                profile.relay_url == token.relay_url
                    && (profile.kernel_credential.is_some()
                        || profile.cloud_session_token.is_some()
                        || profile.machine_credential.is_some())
            })
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "join terminal pairing link",
                message: "key-bound terminal pairing requires active Cloud relay credentials for the target relay".to_string(),
            })?;
        let issued = match issue_cloud_terminal_client_token(
            &profile,
            &terminal_id,
            &token.target_daemon_id,
            CloudTerminalClientOptions {
                target_alias: token.target_daemon_alias.clone(),
                public_key_thumbprint: Some(joined_thumbprint.clone()),
                ..CloudTerminalClientOptions::default()
            },
        )
        .await
        {
            Ok((qualified_id, issued)) => {
                terminal_id = qualified_id;
                issued
            }
            Err(error) => {
                clear_cloud_profile_if_stale(runtime_state, &error).await?;
                return Err(error);
            }
        };
        require_cloud_relay_token_key_binding(&issued.token, &joined_thumbprint)?;
        Some(issued.token)
    } else {
        None
    };
    let client = crate::config::DaemonConfig::record_paired_terminal(
        terminal_id.clone(),
        joined_thumbprint.clone(),
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
            public_key_thumbprint: joined_thumbprint,
            paired_at_ms: now_ms,
        },
        relay_token,
        kernel_pairing: false,
    })
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

#[cfg(test)]
mod self_host_invite_tests {
    use super::*;

    #[tokio::test]
    async fn cloud_generic_invites_cannot_export_kernel_transport_tokens() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://cloud-relay".into());
        config.relay_token = Some("chariox-scoped-v1.synthetic.signature".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            kernel_credential: Some("synthetic-kernel-credential".into()),
            relay_url: "wss://cloud-relay".into(),
            ..Default::default()
        });
        let projection = DaemonConfigProjectionStore::new(config);
        for intent in [PairingInviteIntent::Client, PairingInviteIntent::Machine] {
            let request = serde_json::from_value(serde_json::json!({"intent": intent})).unwrap();
            assert!(execute_create_pairing_invite_request(&projection, request)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn generic_invites_cannot_export_a_scoped_kernel_transport_credential() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("ws://operator-relay".into());
        config.relay_token = Some("chariox-scoped-v1.synthetic.signature".into());
        let projection = DaemonConfigProjectionStore::new(config);
        for intent in [PairingInviteIntent::Client, PairingInviteIntent::Machine] {
            let request: CreatePairingInviteRequest =
                serde_json::from_value(serde_json::json!({"intent": intent})).unwrap();
            let result = execute_create_pairing_invite_request(&projection, request).await;
            assert!(result.is_err());
        }
    }
}
