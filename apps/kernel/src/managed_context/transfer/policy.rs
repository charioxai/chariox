use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

use super::{
    ArmManagedContextTransfer, ManagedContextTransferCaller, ManagedContextTransferPhase,
    ManagedContextTransferStatus, PersistedTransfer, PersistedTransferState,
    COMPLETED_TRANSFER_RETENTION_MS, MAX_ARCHIVE_BYTES, MAX_DESTINATION_BYTES,
    MAX_IDENTIFIER_BYTES, MAX_IMPORT_RECEIPT_BYTES, MAX_TRANSFER_RECORDS, MAX_TRANSFER_TTL_MS,
};

pub(super) fn authorize_entry<'a>(
    state: &'a mut PersistedTransferState,
    transfer_id: &str,
    capability: &str,
    caller: &ManagedContextTransferCaller,
    now_ms: u64,
) -> Result<&'a mut PersistedTransfer, DaemonError> {
    let entry = state.entries.get_mut(transfer_id).ok_or_else(|| {
        authorization_error("managed context transfer is unavailable or unauthorized")
    })?;
    let expired = match entry.phase {
        ManagedContextTransferPhase::Importing => false,
        ManagedContextTransferPhase::Failed | ManagedContextTransferPhase::Consumed => entry
            .completed_at_ms
            .map(|completed_at_ms| {
                completed_at_ms.saturating_add(COMPLETED_TRANSFER_RETENTION_MS) <= now_ms
            })
            .unwrap_or(true),
        _ => entry.expires_at_ms <= now_ms,
    };
    if expired
        || entry.capability_sha256 != sha256_bytes(capability.as_bytes())
        || entry.source_kernel_id != caller.kernel_id
        || entry.source_key_thumbprint != caller.key_thumbprint
        || entry.owner_user_id != caller.owner_user_id
        || entry.realm_id != caller.realm_id
        || entry.target_environment_id != caller.target_environment_id
        || entry.target_kernel_id != caller.target_kernel_id
        || entry.target_key_thumbprint != caller.target_key_thumbprint
    {
        return Err(authorization_error(
            "managed context transfer is unavailable or unauthorized",
        ));
    }
    Ok(entry)
}

pub(super) fn status(transfer_id: &str, entry: &PersistedTransfer) -> ManagedContextTransferStatus {
    ManagedContextTransferStatus {
        transfer_id: transfer_id.to_string(),
        phase: entry.phase,
        accepted_bytes: entry.accepted_bytes,
        archive_size_bytes: entry.archive_size_bytes,
        expires_at_ms: entry.expires_at_ms,
        import_receipt_sha256: entry.import_receipt_sha256.clone(),
        import_receipt_json: entry.import_receipt_json.clone(),
        failure_code: entry.failure_code.clone(),
    }
}

pub(super) fn validate_arm_request(
    request: &ArmManagedContextTransfer,
    now_ms: u64,
) -> Result<(), DaemonError> {
    for (label, value) in [
        ("context", request.context_id.as_str()),
        ("target environment", request.target_environment_id.as_str()),
        ("target kernel", request.target_kernel_id.as_str()),
        ("target key", request.target_key_thumbprint.as_str()),
        ("source kernel", request.source_kernel_id.as_str()),
        ("source key", request.source_key_thumbprint.as_str()),
        ("owner", request.owner_user_id.as_str()),
        ("realm", request.realm_id.as_str()),
        ("project", request.project_id.as_str()),
    ] {
        validate_identifier(value, label)?;
    }
    validate_sha256(&request.target_key_thumbprint, "target key thumbprint")?;
    validate_sha256(&request.source_key_thumbprint, "source key thumbprint")?;
    validate_sha256(&request.archive_sha256, "archive")?;
    if request.archive_size_bytes == 0 || request.archive_size_bytes > MAX_ARCHIVE_BYTES {
        return Err(transfer_error("managed context archive size is invalid"));
    }
    if request.expires_at_ms <= now_ms
        || request.expires_at_ms > now_ms.saturating_add(MAX_TRANSFER_TTL_MS)
    {
        return Err(transfer_error(
            "managed context transfer expiry must be within the allowed window",
        ));
    }
    validate_destination(&request.destination_parent)?;
    Ok(())
}

pub(super) fn validate_persisted_state(state: &PersistedTransferState) -> Result<(), DaemonError> {
    if state.entries.len() > MAX_TRANSFER_RECORDS {
        return Err(transfer_error(
            "managed context transfer state exceeds its record limit",
        ));
    }
    for (transfer_id, entry) in &state.entries {
        if !valid_transfer_id(transfer_id) {
            return Err(transfer_error(
                "managed context transfer state contains an invalid transfer ID",
            ));
        }
        for (label, value) in [
            ("context", entry.context_id.as_str()),
            ("target environment", entry.target_environment_id.as_str()),
            ("target kernel", entry.target_kernel_id.as_str()),
            ("source kernel", entry.source_kernel_id.as_str()),
            ("owner", entry.owner_user_id.as_str()),
            ("realm", entry.realm_id.as_str()),
            ("project", entry.project_id.as_str()),
        ] {
            validate_identifier(value, label)?;
        }
        validate_sha256(&entry.capability_sha256, "capability")?;
        validate_sha256(&entry.target_key_thumbprint, "target key thumbprint")?;
        validate_sha256(&entry.source_key_thumbprint, "source key thumbprint")?;
        validate_sha256(&entry.archive_sha256, "archive")?;
        if let Some(receipt) = &entry.import_receipt_sha256 {
            validate_sha256(receipt, "import receipt")?;
        }
        if let Some(receipt) = &entry.import_receipt_json {
            if receipt.is_empty()
                || receipt.len() > MAX_IMPORT_RECEIPT_BYTES
                || serde_json::from_str::<serde_json::Value>(receipt).is_err()
                || entry.import_receipt_sha256.as_deref()
                    != Some(sha256_bytes(receipt.as_bytes()).as_str())
            {
                return Err(transfer_error(
                    "managed context transfer state contains an invalid import receipt",
                ));
            }
        }
        validate_destination(&entry.destination_root)?;
        if entry.archive_size_bytes == 0
            || entry.archive_size_bytes > MAX_ARCHIVE_BYTES
            || entry.accepted_bytes > entry.archive_size_bytes
            || entry.expires_at_ms == 0
        {
            return Err(transfer_error(
                "managed context transfer state contains invalid bounds",
            ));
        }
        let phase_is_valid = match entry.phase {
            ManagedContextTransferPhase::Armed => {
                entry.accepted_bytes == 0
                    && entry.import_receipt_sha256.is_none()
                    && entry.import_receipt_json.is_none()
                    && entry.completed_at_ms.is_none()
                    && entry.import_started_at_ms.is_none()
                    && entry.failure_code.is_none()
            }
            ManagedContextTransferPhase::Receiving => {
                entry.import_receipt_sha256.is_none()
                    && entry.import_receipt_json.is_none()
                    && entry.completed_at_ms.is_none()
                    && entry.import_started_at_ms.is_none()
                    && entry.failure_code.is_none()
            }
            ManagedContextTransferPhase::ReadyToImport => {
                entry.accepted_bytes == entry.archive_size_bytes
                    && entry.import_receipt_sha256.is_none()
                    && entry.import_receipt_json.is_none()
                    && entry.completed_at_ms.is_none()
                    && entry.import_started_at_ms.is_none()
                    && entry.failure_code.is_none()
            }
            ManagedContextTransferPhase::Importing => {
                entry.accepted_bytes == entry.archive_size_bytes
                    && entry.import_receipt_sha256.is_none()
                    && entry.import_receipt_json.is_none()
                    && entry.completed_at_ms.is_none()
                    && entry.import_started_at_ms.is_some()
                    && entry.failure_code.is_none()
            }
            ManagedContextTransferPhase::Failed => {
                entry.accepted_bytes == entry.archive_size_bytes
                    && entry.import_receipt_sha256.is_none()
                    && entry.import_receipt_json.is_none()
                    && entry.completed_at_ms.is_some()
                    && entry.import_started_at_ms.is_some()
                    && entry.failure_code.as_ref().is_some_and(|code| {
                        !code.is_empty() && code.len() <= 128 && !code.contains('\0')
                    })
            }
            ManagedContextTransferPhase::Consumed => {
                entry.accepted_bytes == entry.archive_size_bytes
                    && entry.import_receipt_sha256.is_some()
                    && entry.import_receipt_json.is_some()
                    && entry.completed_at_ms.is_some()
                    && entry.import_started_at_ms.is_some()
                    && entry.failure_code.is_none()
            }
        };
        if !phase_is_valid {
            return Err(transfer_error(
                "managed context transfer state contains an invalid phase",
            ));
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.trim().is_empty() || value.len() > MAX_IDENTIFIER_BYTES || value.contains('\0') {
        return Err(transfer_error(format!(
            "managed context {label} is invalid"
        )));
    }
    Ok(())
}

fn validate_destination(destination_root: &Path) -> Result<(), DaemonError> {
    let destination = destination_root.to_string_lossy();
    if !destination_root.is_absolute()
        || destination.len() > MAX_DESTINATION_BYTES
        || destination.contains('\0')
        || destination_root.file_name().is_none()
        || destination_root
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(transfer_error("managed context destination is invalid"));
    }
    Ok(())
}

fn valid_transfer_id(transfer_id: &str) -> bool {
    transfer_id.len() == 47
        && transfer_id.starts_with("ctx_")
        && transfer_id[4..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(super) fn validate_sha256(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(transfer_error(format!(
            "managed context {label} digest is invalid"
        )));
    }
    Ok(())
}

pub(super) fn prune_expired(state: &mut PersistedTransferState, now_ms: u64) -> Vec<String> {
    let expired = state
        .entries
        .iter()
        .filter(|(_, entry)| match entry.phase {
            ManagedContextTransferPhase::Importing => false,
            ManagedContextTransferPhase::Failed | ManagedContextTransferPhase::Consumed => entry
                .completed_at_ms
                .map(|completed_at_ms| {
                    completed_at_ms.saturating_add(COMPLETED_TRANSFER_RETENTION_MS) <= now_ms
                })
                .unwrap_or(true),
            _ => entry.expires_at_ms <= now_ms,
        })
        .map(|(transfer_id, _)| transfer_id.clone())
        .collect::<Vec<_>>();
    for transfer_id in &expired {
        state.entries.remove(transfer_id);
    }
    expired
}

pub(super) fn random_identifier(prefix: &str) -> String {
    format!("{prefix}_{}", random_secret())
}

pub(super) fn current_time_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

pub(super) fn random_secret() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn transfer_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "invalid_request",
        operation: "managed context transfer",
        message: message.into(),
        retryable: false,
    }
}

pub(super) fn authorization_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "unauthorized",
        operation: "managed context transfer",
        message: message.into(),
        retryable: false,
    }
}
