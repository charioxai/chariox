//! Recovery codes are a sibling Vault credential, never part of TOTP metadata.
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::DaemonError;

use super::{secret_error, RuntimeSecretService};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct BackupCode {
    value: String,
    used: bool,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct BackupCodes {
    codes: Vec<BackupCode>,
}

fn invalid_input() -> DaemonError {
    secret_error(
        "credential_backup_codes",
        "backup codes must be distinct non-empty lines or a JSON array of strings (maximum 100 codes, 256 bytes each)".into(),
    )
}

pub(super) fn normalize(input: &str) -> Result<Zeroizing<String>, DaemonError> {
    if input.len() > 32_768 {
        return Err(invalid_input());
    }
    // The internal ledger is accepted here so repeated normalization is idempotent.
    if let Ok(ledger) = serde_json::from_str::<BackupCodes>(input) {
        validate(&ledger)?;
        return serde_json::to_string(&ledger)
            .map(Zeroizing::new)
            .map_err(|_| invalid_input());
    }
    let values: Zeroizing<Vec<String>> = Zeroizing::new(if input.trim_start().starts_with('[') {
        serde_json::from_str(input).map_err(|_| invalid_input())?
    } else {
        input
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    });
    let ledger = BackupCodes {
        codes: values
            .iter()
            .map(|value| BackupCode {
                value: value.clone(),
                used: false,
            })
            .collect(),
    };
    validate(&ledger)?;
    serde_json::to_string(&ledger)
        .map(Zeroizing::new)
        .map_err(|_| invalid_input())
}

fn validate(ledger: &BackupCodes) -> Result<(), DaemonError> {
    if ledger.codes.is_empty() || ledger.codes.len() > 100 {
        return Err(invalid_input());
    }
    let mut seen = std::collections::BTreeSet::new();
    for code in &ledger.codes {
        if code.value.trim().is_empty()
            || code.value.len() > 256
            || code.value.chars().any(char::is_control)
            || !seen.insert(code.value.as_str())
        {
            return Err(invalid_input());
        }
    }
    Ok(())
}

pub(super) fn take(service: &RuntimeSecretService, key: &str) -> Result<String, DaemonError> {
    // A kernel owns this Vault. Serialize read/consume/write across service instances.
    // Persist consumption before dispatch: a failed/ambiguous delivery must never
    // cause an already submitted recovery code to be sent a second time.
    static CONSUME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = CONSUME.lock().map_err(|_| {
        secret_error(
            "credential_backup_codes",
            "backup code consumption lock failed; restart the kernel".into(),
        )
    })?;
    let input = Zeroizing::new(
        service
            .vault_store
            .get_secret(service.vault_service_name()?, key)?,
    );
    let normalized = normalize(&input)?;
    let mut ledger: BackupCodes = serde_json::from_str(&normalized).map_err(|_| invalid_input())?;
    let available = ledger
        .codes
        .iter_mut()
        .find(|code| !code.used)
        .ok_or_else(|| {
            secret_error(
                "credential_backup_codes",
                "no unused backup codes remain; enroll a new set".into(),
            )
        })?;
    available.used = true;
    let result = Zeroizing::new(available.value.clone());
    let updated = Zeroizing::new(serde_json::to_string(&ledger).map_err(|_| invalid_input())?);
    service.set_vault_secret(key, &updated)?;
    Ok(result.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_without_echoing_codes() {
        for input in ["", "private-code\nprivate-code", "[\"private-code\\n\"]"] {
            assert!(!normalize(input)
                .unwrap_err()
                .to_string()
                .contains("private-code"));
        }
        assert!(normalize("first-code\nsecond-code").is_ok());
        assert!(normalize("[\"first-code\",\"second-code\"]").is_ok());
    }
}
