//! Cloud-free terminal admission: the kernel grants an exact local target to a
//! terminal key. The relay's operator token grants transport, never this authority.
use super::terminal_pairings::public_key_thumbprint;
use crate::{
    config::{DaemonConfig, PersistedClientPairing},
    error::DaemonError,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

static STORE_LOCK: Mutex<()> = Mutex::new(());
#[derive(Default, Serialize, Deserialize)]
struct Store {
    pending: Vec<Pending>,
    grants: Vec<PersistedClientPairing>,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    link_hash: String,
    terminal_id: String,
    expires_at_ms: u64,
    terminal_type: String,
    alias: Option<String>,
    issued_at_ms: u64,
}
fn path(config: &DaemonConfig) -> std::path::PathBuf {
    config
        .private_runtime_state_root()
        .join("terminal-grants.json")
}
fn read(config: &DaemonConfig) -> Result<Store, DaemonError> {
    match std::fs::read(path(config)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| denied()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(_) => Err(denied()),
    }
}
fn write(config: &DaemonConfig, store: &Store) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(store).map_err(|_| denied())?;
    crate::config::write_private_file(&path(config), &bytes).map_err(|_| denied())
}
/// A scoped kernel transport credential must never be copied to a terminal.
/// Operators provide the separate CLIENT transport token through the CLI environment.
pub(crate) fn pairing_transport_token(token: String) -> String {
    use base64::Engine;
    let parts: Vec<_> = token.trim().split('.').collect();
    let scoped = parts.len() == 3
        && (parts[0] == "chariox-scoped-v1"
            || base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(parts[0])
                .ok()
                .and_then(|header| serde_json::from_slice::<serde_json::Value>(&header).ok())
                .is_some_and(|header| header.get("alg").is_some()));
    if scoped {
        "operator-client-token-required".into()
    } else {
        token
    }
}

pub(crate) fn required(config: &DaemonConfig) -> bool {
    config.cloud_relay.is_none() && config.relay_url.is_some()
}
pub(crate) fn register(
    config: &DaemonConfig,
    link: &str,
    terminal_id: &str,
    expires_at_ms: u64,
    terminal_type: &str,
    alias: Option<String>,
) -> Result<(), DaemonError> {
    let _lock = STORE_LOCK.lock().map_err(|_| denied())?;
    let mut store = read(config)?;
    store
        .pending
        .retain(|p| p.expires_at_ms > crate::session::unix_epoch_ms());
    store.pending.push(Pending {
        link_hash: public_key_thumbprint(link),
        terminal_id: terminal_id.into(),
        expires_at_ms,
        terminal_type: terminal_type.into(),
        alias,
        issued_at_ms: crate::session::unix_epoch_ms(),
    });
    write(config, &store)
}
pub(crate) fn redeem(
    config: &DaemonConfig,
    link: &str,
    grant: PersistedClientPairing,
) -> Result<PersistedClientPairing, DaemonError> {
    let _lock = STORE_LOCK.lock().map_err(|_| denied())?;
    let mut store = read(config)?;
    let hash = public_key_thumbprint(link);
    if !store.pending.iter().any(|p| {
        p.link_hash == hash
            && p.terminal_id == grant.client_id
            && p.expires_at_ms > crate::session::unix_epoch_ms()
    }) {
        return Err(denied());
    }
    if let Some(existing) = store.grants.iter().find(|g| g.client_id == grant.client_id) {
        if existing.revoked || existing.public_key_thumbprint != grant.public_key_thumbprint {
            return Err(denied());
        }
        return Ok(existing.clone()); // Retry by the same key, never takeover.
    }
    store.grants.push(grant.clone());
    write(config, &store)?;
    Ok(grant)
}
pub(crate) fn entries(config: &DaemonConfig) -> Result<Vec<PersistedClientPairing>, DaemonError> {
    let store = read(config)?;
    let mut grants = store.grants;
    for pending in store.pending {
        if pending.expires_at_ms > crate::session::unix_epoch_ms()
            && !grants.iter().any(|g| g.client_id == pending.terminal_id)
        {
            grants.push(PersistedClientPairing {
                client_id: pending.terminal_id,
                alias: pending.alias,
                terminal_type: pending.terminal_type,
                public_key_thumbprint: String::new(),
                paired_at_ms: pending.issued_at_ms,
                revoked: false,
            });
        }
    }
    Ok(grants)
}
pub(crate) fn admit(
    config: &DaemonConfig,
    public_key: &str,
) -> Result<Option<PersistedClientPairing>, DaemonError> {
    if !required(config) {
        return Ok(None);
    }
    let thumbprint = public_key_thumbprint(public_key);
    read(config)?
        .grants
        .into_iter()
        .find(|g| !g.revoked && g.public_key_thumbprint == thumbprint)
        .map(Some)
        .ok_or_else(denied)
}
pub(crate) fn revoke(
    config: &DaemonConfig,
    terminal_id: &str,
) -> Result<PersistedClientPairing, DaemonError> {
    let _lock = STORE_LOCK.lock().map_err(|_| denied())?;
    let mut store = read(config)?;
    if !store.grants.iter().any(|g| g.client_id == terminal_id) {
        let pending = store
            .pending
            .iter()
            .find(|p| p.terminal_id == terminal_id)
            .ok_or_else(denied)?;
        let revoked = PersistedClientPairing {
            client_id: terminal_id.into(),
            alias: pending.alias.clone(),
            terminal_type: pending.terminal_type.clone(),
            revoked: true,
            ..Default::default()
        };
        store.pending.retain(|p| p.terminal_id != terminal_id);
        write(config, &store)?;
        return Ok(revoked);
    }
    let grant = store
        .grants
        .iter_mut()
        .find(|g| g.client_id == terminal_id)
        .ok_or_else(denied)?;
    grant.revoked = true;
    let grant = grant.clone();
    store.pending.retain(|p| p.terminal_id != terminal_id);
    write(config, &store)?;
    Ok(grant)
}
fn denied() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "authorize terminal pairing",
        message:
            "terminal key is unpaired, revoked, or does not match this kernel's issued pairing link"
                .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_kernel_transport_is_not_exported_in_pairing_links() {
        assert_eq!(
            pairing_transport_token("chariox-scoped-v1.synthetic.signature".into()),
            "operator-client-token-required"
        );
        assert_eq!(
            pairing_transport_token("synthetic-shared-realm".into()),
            "synthetic-shared-realm"
        );
        assert_eq!(
            pairing_transport_token("operator.shared.token".into()),
            "operator.shared.token"
        );
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        assert_eq!(
            pairing_transport_token(format!("{header}.synthetic.signature")),
            "operator-client-token-required"
        );
    }
    #[test]
    fn pairing_link_hash_prevents_forgery_takeover_and_cross_kernel_admission() {
        let root = std::env::temp_dir().join(format!(
            "chariox-terminal-grants-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.join("one/state.db").display().to_string());
        config.relay_url = Some("ws://relay".into());
        config.cloud_relay = None;
        let pin = public_key_thumbprint("terminal-key");
        let grant = PersistedClientPairing {
            client_id: "terminal".into(),
            public_key_thumbprint: pin,
            paired_at_ms: 0,
            ..Default::default()
        };
        register(
            &config,
            "synthetic-issued-link",
            "terminal",
            crate::session::unix_epoch_ms() + 60_000,
            "cli",
            None,
        )
        .unwrap();
        assert!(redeem(&config, "forged-link", grant.clone()).is_err());
        redeem(&config, "synthetic-issued-link", grant.clone()).unwrap();
        assert!(admit(&config, "terminal-key").is_ok()); // No fixed grant lifetime.
        let mut wrong = grant.clone();
        wrong.public_key_thumbprint = public_key_thumbprint("other-key");
        assert!(redeem(&config, "synthetic-issued-link", wrong).is_err());
        assert!(admit(&config, "other-key").is_err());
        let mut sibling = config.clone();
        sibling.user_config.state.path = Some(root.join("two/state.db").display().to_string());
        assert!(admit(&sibling, "terminal-key").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path(&config))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        revoke(&config, "terminal").unwrap();
        assert!(redeem(&config, "synthetic-issued-link", grant).is_err());
        assert!(admit(&config, "terminal-key").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
