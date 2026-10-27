//! MP-08/MP-09/MP-10/MP-11: explicit, independent copies of Vault-backed accounts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

pub const PROVIDER_ACCOUNT_COPY_CAPABILITY: &str = "provider_account_copy_v1";

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
        if self.provider != "claude"
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
        let digest = Sha256::digest(format!(
            "{}\0{}\0{}\0{}\0{}",
            self.source_kernel_id,
            self.source_key_thumbprint,
            self.provider,
            self.source_profile_id,
            self.target_kernel_id
        ));
        format!("copy-{digest:x}")[..45].to_string()
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

/// Serialized only inside the encrypted kernel-to-kernel request.
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
                | "provider_account_copy_install_failed"
        ),
    }
}

pub(crate) fn failure_message(code: &str) -> &'static str {
    match code {
        "provider_account_copy_unauthorized" => "Account copy requires enrolled kernels with the same owner, realm and pinned keys.",
        "provider_account_copy_profile" => "Select an exact authenticated Claude profile with a Vault-backed credential on the source kernel.",
        "provider_account_copy_unavailable" => "The target kernel is unavailable; reconnect it and retry the same operation.",
        "provider_account_copy_unsupported" => "Upgrade the target kernel to support provider account copy before retrying.",
        "provider_account_copy_vault_locked" => "Unlock the kernel Vault through Chariox, then retry the same account copy.",
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
            | "provider_account_copy_unavailable"
            | "provider_account_copy_unsupported"
            | "provider_account_copy_vault_locked"
            | "provider_account_copy_conflict"
            | "provider_account_copy_rejected"
            | "provider_account_copy_verification_failed"
            | "provider_account_copy_install_failed"
    )
}
