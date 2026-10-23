//! The home kernel as the self-hosted relay token issuer (M5.5 "Token
//! Issuers"). Without Cloud, the kernel signs short-lived CLIENT relay tokens
//! for its paired terminals with an Ed25519 key; the relay operator pins only
//! the public key. The private key never leaves the private runtime state.
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chariox_relay::auth::{
    encode_kernel_issued_client_token, encode_kernel_issuer_public_key, kernel_issuer_id,
    kernel_issuer_key_id, KernelIssuedClientClaims, RelayAction, DEFAULT_RELAY_REALM_ID,
};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::config::{DaemonConfig, PersistedClientPairing};
use crate::error::DaemonError;
use crate::local::{
    ConfigureRelayIssuerRequest, IssueRelayClientTokenRequest, LocalDaemonRequest,
    LocalDaemonResponse, RelayClientToken, RelayClientTokenScope, RelayIssuerKeyRecord,
    RelayIssuerRecord, RelayIssuerStep,
};
use crate::runtime::command::{KernelCaller, KernelCallerKind};

pub(crate) const DEFAULT_CLIENT_TOKEN_LIFETIME_MS: u64 = 15 * 60 * 1000;
const MIN_CLIENT_TOKEN_LIFETIME_MS: u64 = 60 * 1000;
pub(crate) const MAX_CLIENT_TOKEN_LIFETIME_MS: u64 = 60 * 60 * 1000;

static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct Store {
    active_seed: String,
    #[serde(default)]
    next_seed: Option<String>,
}

pub(crate) fn execute_relay_issuer_request(
    config: &DaemonConfig,
    caller: &KernelCaller,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::RelayIssuerStatus(_) => {
            require_local(caller, "show relay issuer")?;
            Ok(LocalDaemonResponse::RelayIssuer {
                issuer: read(config)?
                    .map(|store| record(config, &store))
                    .transpose()?,
            })
        }
        LocalDaemonRequest::ConfigureRelayIssuer(request) => {
            require_local(caller, "configure relay issuer")?;
            configure(config, request)
        }
        LocalDaemonRequest::IssueRelayClientToken(request) => {
            let token = issue_for_caller(config, caller, request)?;
            Ok(LocalDaemonResponse::RelayClientTokenIssued { token })
        }
        _ => Err(error(
            "relay issuer request",
            "unsupported relay issuer request",
        )),
    }
}

/// The unbound pairing token a self-hosted terminal pairing link carries: it
/// lets the receiving terminal reach this kernel only to redeem the link.
pub(crate) fn pairing_link_token(
    config: &DaemonConfig,
    terminal_id: &str,
    expires_at_ms: u64,
) -> Result<Option<RelayClientToken>, DaemonError> {
    let Some(store) = read(config)? else {
        return Ok(None);
    };
    let expires_at_ms = expires_at_ms.min(now_ms().saturating_add(MAX_CLIENT_TOKEN_LIFETIME_MS));
    sign(
        config,
        &store,
        terminal_id,
        None,
        RelayClientTokenScope::Attach,
        expires_at_ms,
    )
    .map(Some)
}

/// The key-bound token returned when a terminal redeems its pairing link.
pub(crate) fn paired_terminal_token(
    config: &DaemonConfig,
    grant: &PersistedClientPairing,
) -> Result<Option<String>, DaemonError> {
    let Some(store) = read(config)? else {
        return Ok(None);
    };
    let expires_at_ms = now_ms().saturating_add(DEFAULT_CLIENT_TOKEN_LIFETIME_MS);
    sign(
        config,
        &store,
        &grant.client_id,
        Some(&grant.public_key_thumbprint),
        RelayClientTokenScope::Attach,
        expires_at_ms,
    )
    .map(|token| Some(token.relay_token))
}

pub(crate) fn enabled(config: &DaemonConfig) -> Result<bool, DaemonError> {
    Ok(read(config)?.is_some())
}

fn issue_for_caller(
    config: &DaemonConfig,
    caller: &KernelCaller,
    request: IssueRelayClientTokenRequest,
) -> Result<RelayClientToken, DaemonError> {
    const OPERATION: &str = "issue relay client token";
    match caller.caller_kind {
        KernelCallerKind::LocalClient => {}
        // A paired terminal renews its own identity and key only.
        KernelCallerKind::RemoteClient
            if caller.client_id.as_deref() == Some(request.client_id.as_str())
                && caller.public_key_thumbprint.as_deref()
                    == Some(request.public_key_thumbprint.as_str()) => {}
        _ => {
            return Err(error(
                OPERATION,
                "a terminal can only renew its own relay token; issue other clients' tokens from the home terminal",
            ))
        }
    }
    if config.cloud_relay.is_some() {
        return Err(error(
            OPERATION,
            "this kernel uses a Cloud relay profile; its client tokens come from Cloud (IssueCloudRelayClientToken)",
        ));
    }
    if !super::self_host_terminal_grants::required(config) {
        return Err(error(
            OPERATION,
            "configure a self-hosted relay URL before issuing relay client tokens",
        ));
    }
    let lifetime_ms = request
        .lifetime_ms
        .unwrap_or(DEFAULT_CLIENT_TOKEN_LIFETIME_MS);
    if !(MIN_CLIENT_TOKEN_LIFETIME_MS..=MAX_CLIENT_TOKEN_LIFETIME_MS).contains(&lifetime_ms) {
        return Err(error(
            OPERATION,
            "lifetime_ms must be between 60000 (1 minute) and 3600000 (1 hour)",
        ));
    }
    let store = read(config)?.ok_or_else(|| {
        error(
            OPERATION,
            "the relay issuer is not enabled; run ConfigureRelayIssuer enable and pin its key on the relay",
        )
    })?;
    let grant = super::self_host_terminal_grants::grant(config, &request.client_id)?
        .filter(|grant| !grant.revoked)
        .ok_or_else(|| {
            error(
                OPERATION,
                "client is not paired or was revoked; pair it again with a terminal pairing link",
            )
        })?;
    if grant.public_key_thumbprint != request.public_key_thumbprint {
        return Err(error(
            OPERATION,
            "key thumbprint does not match the paired client's key",
        ));
    }
    sign(
        config,
        &store,
        &grant.client_id,
        Some(&grant.public_key_thumbprint),
        request.scope,
        now_ms().saturating_add(lifetime_ms),
    )
}

fn sign(
    config: &DaemonConfig,
    store: &Store,
    client_id: &str,
    public_key_thumbprint: Option<&str>,
    scope: RelayClientTokenScope,
    expires_at_ms: u64,
) -> Result<RelayClientToken, DaemonError> {
    const OPERATION: &str = "issue relay client token";
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| error(OPERATION, "relay URL is not configured"))?;
    let issued_at_ms = now_ms();
    // JWT times are whole seconds; report the expiry the relay will enforce.
    let expires_at_ms = expires_at_ms / 1000 * 1000;
    let mut token_id = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut token_id);
    let token_id = token_id
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let claims = KernelIssuedClientClaims {
        issuer: issuer_id(config),
        realm_id: DEFAULT_RELAY_REALM_ID.into(),
        client_id: client_id.into(),
        allowed_actions: match scope {
            RelayClientTokenScope::Attach => {
                vec![RelayAction::ClientConnect, RelayAction::PacketRoute]
            }
            RelayClientTokenScope::Metadata => vec![RelayAction::ClientMetadataRead],
        },
        target_daemon_id: config.daemon_id.clone(),
        issued_at_ms,
        expires_at_ms,
        token_id: token_id.clone(),
        public_key_thumbprint: public_key_thumbprint.map(str::to_string),
    };
    let relay_token = encode_kernel_issued_client_token(&claims, &signing_key(&store.active_seed)?)
        .map_err(|_| error(OPERATION, "failed to sign the relay client token"))?;
    Ok(RelayClientToken {
        relay_url,
        relay_token,
        target_daemon_id: config.daemon_id.clone(),
        token_id,
        expires_at_ms,
    })
}

fn configure(
    config: &DaemonConfig,
    request: ConfigureRelayIssuerRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    const OPERATION: &str = "configure relay issuer";
    let _lock = STORE_LOCK
        .lock()
        .map_err(|_| error(OPERATION, "relay issuer store lock is poisoned"))?;
    let current = read(config)?;
    let next = match (request.step, current) {
        (RelayIssuerStep::Enable, Some(store)) => Some(store),
        (RelayIssuerStep::Enable, None) => Some(Store {
            active_seed: new_seed(),
            next_seed: None,
        }),
        (RelayIssuerStep::Disable, _) => None,
        (_, None) => return Err(error(OPERATION, "the relay issuer is not enabled")),
        (RelayIssuerStep::StageRotation, Some(store)) if store.next_seed.is_some() => {
            return Err(error(
                OPERATION,
                "a rotation is already staged; pin both keys on the relay, then activate it",
            ))
        }
        (RelayIssuerStep::StageRotation, Some(store)) => Some(Store {
            next_seed: Some(new_seed()),
            ..store
        }),
        (RelayIssuerStep::ActivateRotation, Some(store)) => Some(Store {
            active_seed: store
                .next_seed
                .ok_or_else(|| error(OPERATION, "no rotation is staged; stage one first"))?,
            next_seed: None,
        }),
    };
    match &next {
        Some(store) => write(config, store)?,
        None => match std::fs::remove_file(path(config)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error(OPERATION, "failed to remove the relay issuer key")),
        },
    }
    Ok(LocalDaemonResponse::RelayIssuer {
        issuer: next.map(|store| record(config, &store)).transpose()?,
    })
}

fn record(config: &DaemonConfig, store: &Store) -> Result<RelayIssuerRecord, DaemonError> {
    let issuer_id = issuer_id(config);
    let active_key = key_record(&store.active_seed)?;
    let next_key = store.next_seed.as_deref().map(key_record).transpose()?;
    let relay_issuer_keys = std::iter::once(&active_key)
        .chain(next_key.as_ref())
        .map(|key| format!("{issuer_id}={}", key.public_key))
        .collect::<Vec<_>>()
        .join(",");
    Ok(RelayIssuerRecord {
        issuer_id,
        realm_id: DEFAULT_RELAY_REALM_ID.into(),
        active_key,
        next_key,
        relay_issuer_keys,
    })
}

fn key_record(seed: &str) -> Result<RelayIssuerKeyRecord, DaemonError> {
    let public_key = signing_key(seed)?.verifying_key();
    Ok(RelayIssuerKeyRecord {
        key_id: kernel_issuer_key_id(&public_key),
        public_key: encode_kernel_issuer_public_key(&public_key),
    })
}

fn issuer_id(config: &DaemonConfig) -> String {
    kernel_issuer_id(&config.daemon_id)
}

fn new_seed() -> String {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    STANDARD.encode(seed)
}

fn signing_key(seed: &str) -> Result<SigningKey, DaemonError> {
    let seed: [u8; 32] = STANDARD
        .decode(seed)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            error(
                "load relay issuer",
                "the relay issuer key is unreadable; disable and enable the issuer, then pin the new key on the relay",
            )
        })?;
    Ok(SigningKey::from_bytes(&seed))
}

fn require_local(caller: &KernelCaller, operation: &'static str) -> Result<(), DaemonError> {
    if caller.caller_kind == KernelCallerKind::LocalClient {
        return Ok(());
    }
    Err(error(
        operation,
        "the relay issuer is managed from a terminal on the kernel's own machine",
    ))
}

fn path(config: &DaemonConfig) -> std::path::PathBuf {
    config
        .private_runtime_state_root()
        .join("relay-issuer.json")
}

fn read(config: &DaemonConfig) -> Result<Option<Store>, DaemonError> {
    match std::fs::read(path(config)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| {
            error(
                "load relay issuer",
                "the relay issuer state is unreadable; disable and enable the issuer",
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error(
            "load relay issuer",
            "failed to read the relay issuer state",
        )),
    }
}

fn write(config: &DaemonConfig, store: &Store) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(store).map_err(|_| {
        error(
            "configure relay issuer",
            "failed to encode the relay issuer state",
        )
    })?;
    crate::config::write_private_file(&path(config), &bytes).map_err(|_| {
        error(
            "configure relay issuer",
            "failed to store the relay issuer key",
        )
    })
}

fn now_ms() -> u64 {
    crate::session::unix_epoch_ms()
}

fn error(operation: &'static str, message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
