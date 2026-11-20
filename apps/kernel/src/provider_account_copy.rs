//! Explicit, independent copies of provider accounts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

pub const PROVIDER_ACCOUNT_COPY_PULL_CAPABILITY: &str = "provider_account_copy_pull_v1";

pub const PROVIDER_ACCOUNT_COPY_CAPABILITY: &str = "provider_account_copy_v1";
pub const PROVIDER_ACCOUNT_NATIVE_COPY_CAPABILITY: &str = "provider_account_native_copy_v1";

pub(crate) fn native_copy_credential_path(provider: &str) -> Option<&'static str> {
    match provider {
        "codex" => Some("auth.json"),
        "opencode" => Some("data/opencode/auth.json"),
        _ => None,
    }
}

pub(crate) fn copy_provider_supported(provider: &str) -> bool {
    provider == "claude" || native_copy_credential_path(provider).is_some()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAccountCopyBinding {
    pub operation_id: String,
    pub provider: String,
    pub source_profile_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub replace: bool,
}

impl ProviderAccountCopyBinding {
    pub(crate) fn validate(&self) -> Result<(), crate::error::DaemonError> {
        if !copy_provider_supported(&self.provider)
            || ![
                &self.operation_id,
                &self.source_profile_id,
                &self.source_kernel_id,
                &self.target_kernel_id,
            ]
            .into_iter()
            .all(|value| valid_id(value))
            || ![&self.source_key_thumbprint, &self.target_key_thumbprint]
                .into_iter()
                .all(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
            || self.source_kernel_id == self.target_kernel_id
        {
            return Err(failure("invalid_request"));
        }
        Ok(())
    }

    pub(crate) fn target_profile_id(&self) -> String {
        copy_profile_id(
            &self.source_kernel_id,
            &self.source_key_thumbprint,
            &self.provider,
            &self.source_profile_id,
            &self.target_kernel_id,
        )
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountCopyReceipt {
    pub operation_id: String,
    pub provider: String,
    pub source_profile_id: String,
    pub target_kernel_id: String,
    pub account_profile: String,
}

impl ProviderAccountCopyReceipt {
    pub(crate) fn installed(binding: &ProviderAccountCopyBinding) -> Self {
        Self {
            operation_id: binding.operation_id.clone(),
            provider: binding.provider.clone(),
            source_profile_id: binding.source_profile_id.clone(),
            target_kernel_id: binding.target_kernel_id.clone(),
            account_profile: binding.target_profile_id(),
        }
    }
}

/// Serialized only inside the encrypted kernel-to-kernel request. The binding's
/// provider identifies a Claude setup token or a base64-encoded native auth file.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderAccountCopyCredential(String);

impl ProviderAccountCopyCredential {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
    pub(crate) fn value(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for ProviderAccountCopyCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl Drop for ProviderAccountCopyCredential {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub(crate) fn failure(code: &'static str) -> crate::error::DaemonError {
    crate::error::DaemonError::ProviderAccountCopy {
        code,
        message: failure_message(code),
        retryable: matches!(
            code,
            "provider_account_copy_unavailable"
                | "provider_account_copy_vault_locked"
                | "provider_account_copy_target_vault_locked"
                | "provider_account_copy_vault_unreadable"
                | "provider_account_copy_target_vault_unreadable"
                | "provider_account_copy_target_credential_unreadable"
                | "provider_account_copy_cli_missing"
                | "provider_account_copy_install_failed"
        ),
    }
}

/// Why the Vault gave no stored login: a locked Vault is the owner's to unlock,
/// anything else (a missing record, an unreadable file) is a different problem
/// and must not be reported as a lock.
pub(crate) fn credential_read_failure(
    error: &crate::error::DaemonError,
) -> crate::error::DaemonError {
    if crate::secret::is_chariox_vault_locked_error(error) {
        failure("provider_account_copy_vault_locked")
    } else {
        failure("provider_account_copy_credential_unreadable")
    }
}

pub(crate) fn failure_message(code: &str) -> &'static str {
    match code {
        "provider_account_copy_unauthorized" => "Account copy requires enrolled kernels with the same owner, realm and pinned keys.",
        "provider_account_copy_profile" => "Select an authenticated OpenCode or Codex profile with its native login, or a Claude profile with a Vault-backed setup token, on the source kernel.",
        "provider_account_copy_unavailable" => "The target kernel is unavailable; reconnect it and retry the same operation.",
        "provider_account_copy_unsupported" => "Upgrade the target kernel to support provider account copy before retrying.",
        "provider_account_copy_vault_locked" => "Unlock the kernel Vault through Chariox, then retry the same account copy.",
        "provider_account_copy_target_vault_locked" => "The receiving kernel's Vault is locked; unlock it, then retry the same account copy. Nothing was copied.",
        "provider_account_copy_vault_unreadable" => "The kernel Vault could not be read; check that its file is present and readable, then retry the same account copy.",
        "provider_account_copy_target_vault_unreadable" => "The receiving kernel's Vault could not be read; check that its file is present and readable, then retry the same account copy. Nothing was copied.",
        "provider_account_copy_credential_unreadable" => "The account's stored login could not be read from the Vault; sign in to the account again, then retry.",
        "provider_account_copy_target_credential_unreadable" => "The receiving kernel could not read the copied login back from its Vault; retry the same account copy.",
        "provider_account_copy_cli_missing" => "The Claude CLI is not installed on the receiving kernel; install it there, then retry the same account copy. Nothing was copied.",
        "provider_account_copy_replace_differs" => "This operation was already used with a different replace setting; start a new operation to copy with that setting.",
        "provider_account_copy_cancelled" => "Account copy was cancelled or expired; no login was transferred.",
        "provider_account_copy_conflict" => "A target profile or operation already exists; use a new operation with explicit replacement to replace the profile.",
        "provider_account_copy_rejected" => "Official Claude verification rejected the account; authorize the source account again.",
        "provider_account_copy_verification_failed" => "Official Claude verification did not complete; check Claude availability and retry.",
        "provider_account_copy_install_failed" => "Account installation did not finish; retry the same operation to recover.",
        _ => "Invalid provider account copy request.",
    }
}

pub(crate) fn is_public_code(code: &str) -> bool {
    matches!(
        code,
        "provider_account_copy_unauthorized"
            | "provider_account_copy_profile"
            | "provider_account_copy_cancelled"
            | "provider_account_copy_unavailable"
            | "provider_account_copy_unsupported"
            | "provider_account_copy_vault_locked"
            | "provider_account_copy_target_vault_locked"
            | "provider_account_copy_vault_unreadable"
            | "provider_account_copy_target_vault_unreadable"
            | "provider_account_copy_credential_unreadable"
            | "provider_account_copy_target_credential_unreadable"
            | "provider_account_copy_cli_missing"
            | "provider_account_copy_replace_differs"
            | "provider_account_copy_conflict"
            | "provider_account_copy_rejected"
            | "provider_account_copy_verification_failed"
            | "provider_account_copy_install_failed"
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountCopyOffer {
    pub source_kernel_id: String,
    pub source_machine_id: String,
    pub source_label: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub provider: String,
    pub account_profile: String,
    pub label: String,
}

pub(crate) fn copy_profile_id(
    source_kernel: &str,
    source_key: &str,
    provider: &str,
    source_profile: &str,
    target_kernel: &str,
) -> String {
    let digest = Sha256::digest(format!(
        "{source_kernel}\0{source_key}\0{provider}\0{source_profile}\0{target_kernel}"
    ));
    format!("copy-{digest:x}")[..45].to_string()
}
