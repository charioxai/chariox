use sha2::{Digest, Sha256};

use crate::config::PersistedClientPairing;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, PairedClientRecord, RecordPairedClientRequest, RevokePairedClientRequest,
    TerminalRecord, TerminalType,
};

pub(crate) fn execute_list_terminals_request() -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::TerminalsListed {
        terminals: paired_terminal_records(),
    })
}

pub(crate) fn execute_list_paired_clients_request() -> Result<LocalDaemonResponse, DaemonError> {
    let clients = crate::config::DaemonConfig::client_pairing_entries()
        .into_iter()
        .map(paired_client_record)
        .collect();
    Ok(LocalDaemonResponse::PairedClientsListed { clients })
}

pub(crate) fn execute_record_paired_client_request(
    request: RecordPairedClientRequest,
    default_paired_at_ms: impl FnOnce() -> u64,
) -> Result<LocalDaemonResponse, DaemonError> {
    let paired_at_ms = request.paired_at_ms.unwrap_or_else(default_paired_at_ms);
    let client = crate::config::DaemonConfig::record_paired_terminal(
        request.client_id,
        request.public_key_thumbprint,
        request.alias,
        paired_at_ms,
        request.terminal_type.unwrap_or(TerminalType::Cli).as_str(),
    )?;
    Ok(LocalDaemonResponse::PairedClientRecorded {
        client: paired_client_record(client),
    })
}

pub(crate) fn execute_revoke_paired_client_request(
    request: RevokePairedClientRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let client = crate::config::DaemonConfig::revoke_paired_client(request.client_id)?;
    Ok(LocalDaemonResponse::PairedClientRevoked {
        client: paired_client_record(client),
    })
}

pub(crate) fn paired_client_record(client: PersistedClientPairing) -> PairedClientRecord {
    let terminal_type = terminal_type_from_str(&client.terminal_type);
    PairedClientRecord {
        client_id: client.client_id,
        alias: client.alias,
        terminal_type: Some(terminal_type),
        public_key_thumbprint: client.public_key_thumbprint,
        paired_at_ms: client.paired_at_ms,
        revoked: client.revoked,
    }
}

pub(crate) fn paired_terminal_records() -> Vec<TerminalRecord> {
    crate::config::DaemonConfig::client_pairing_entries()
        .into_iter()
        .map(terminal_record)
        .collect()
}

pub(crate) fn terminal_record(client: PersistedClientPairing) -> TerminalRecord {
    TerminalRecord {
        terminal_id: client.client_id,
        terminal_type: terminal_type_from_str(&client.terminal_type),
        alias: client.alias,
        paired_at_ms: client.paired_at_ms,
        revoked: client.revoked,
    }
}

pub(crate) fn terminal_type_from_str(value: &str) -> TerminalType {
    match value.trim().to_ascii_lowercase().as_str() {
        "web" | "web_terminal" | "web-terminal" => TerminalType::Web,
        "ios" | "ios_terminal" | "ios-terminal" => TerminalType::Ios,
        "android" | "android_terminal" | "android-terminal" => TerminalType::Android,
        _ => TerminalType::Cli,
    }
}

pub(crate) fn public_key_thumbprint(public_key: &str) -> String {
    let digest = Sha256::digest(public_key.as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_type_projection_accepts_legacy_spellings() {
        assert_eq!(terminal_type_from_str("web_terminal"), TerminalType::Web);
        assert_eq!(terminal_type_from_str("ios-terminal"), TerminalType::Ios);
        assert_eq!(terminal_type_from_str("android"), TerminalType::Android);
        assert_eq!(terminal_type_from_str("unknown"), TerminalType::Cli);
    }

    #[test]
    fn paired_client_record_projects_terminal_type_and_revocation() {
        let record = paired_client_record(PersistedClientPairing {
            client_id: "client-1".to_string(),
            alias: Some("Work web".to_string()),
            terminal_type: "web-terminal".to_string(),
            public_key_thumbprint: "thumbprint".to_string(),
            paired_at_ms: 100,
            revoked: true,
        });

        assert_eq!(record.client_id, "client-1");
        assert_eq!(record.alias.as_deref(), Some("Work web"));
        assert_eq!(record.terminal_type, Some(TerminalType::Web));
        assert_eq!(record.public_key_thumbprint, "thumbprint");
        assert_eq!(record.paired_at_ms, 100);
        assert!(record.revoked);
    }

    #[test]
    fn terminal_record_projects_waiting_room_terminal_shape() {
        let record = terminal_record(PersistedClientPairing {
            client_id: "cli-1".to_string(),
            alias: None,
            terminal_type: "cli".to_string(),
            public_key_thumbprint: "thumbprint".to_string(),
            paired_at_ms: 200,
            revoked: false,
        });

        assert_eq!(record.terminal_id, "cli-1");
        assert_eq!(record.terminal_type, TerminalType::Cli);
        assert_eq!(record.paired_at_ms, 200);
        assert!(!record.revoked);
    }

    #[test]
    fn public_key_thumbprint_is_stable_sha256_hex() {
        assert_eq!(
            public_key_thumbprint("public-key"),
            "43a46f1d081d270130e2210a1de59f9715de033307d068edc65a335b27e95d3d",
        );
    }
}
