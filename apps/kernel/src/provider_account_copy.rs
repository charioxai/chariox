//! Explicit, independent copies of provider accounts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

pub const PROVIDER_ACCOUNT_COPY_PULL_CAPABILITY: &str = "provider_account_copy_pull_v1";

pub const PROVIDER_ACCOUNT_COPY_CAPABILITY: &str = "provider_account_copy_v1";
/// A receiver that installs a native OpenCode login. Without it the source
/// refuses an OpenCode copy before it exports anything.
pub const PROVIDER_ACCOUNT_COPY_OPENCODE_CAPABILITY: &str = "provider_account_copy_opencode_v1";

/// The most credential text one copy carries, for every provider.
pub(crate) const MAX_COPY_CREDENTIAL_BYTES: usize = 16 * 1024;

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
        if !["claude", "codex", "opencode"].contains(&self.provider.as_str())
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
            || (self.source_kernel_id == self.target_kernel_id
                && self.source_key_thumbprint != self.target_key_thumbprint)
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

/// Whether a kernel that advertised these capabilities handles a copy of this
/// provider's login. Claude and Codex ride the base capability; a provider added
/// later needs its own, so an older kernel refuses before anything is exported.
pub(crate) fn kernel_handles_copy_of(provider: &str, capabilities: &[String]) -> bool {
    let has = |wanted: &str| capabilities.iter().any(|capability| capability == wanted);
    match provider {
        "opencode" => has(PROVIDER_ACCOUNT_COPY_OPENCODE_CAPABILITY),
        _ => true,
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

pub(crate) enum AccountCopyInstall {
    Fresh(ProviderAccountCopyReceipt),
    Replay(ProviderAccountCopyReceipt),
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
                | TRANSFER_SIGN_IN_REQUIRED
        ),
    }
}

/// Why the Vault gave no stored login: a locked Vault is the owner's to unlock.
/// Anything else (a missing record, an unreadable file) is a different problem

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
        "provider_account_copy_profile" => "Select an exact authenticated account on the source kernel. If it is missing or signed out, sign in there and retry the copy.",

        "provider_account_copy_unavailable" => "The target kernel is unavailable; reconnect it and retry the same operation.",
        "provider_account_copy_unsupported" => "Upgrade the target kernel to support provider account copy before retrying.",
        "provider_account_copy_vault_locked" => "Unlock the kernel Vault through Chariox, then retry the same account copy.",
        "provider_account_copy_target_vault_locked" => "The receiving kernel's Vault is locked; unlock it, then retry the same account copy. Nothing was copied.",
        "provider_account_copy_vault_unreadable" => "The kernel Vault could not be read; check that its file is present and readable, then retry the same account copy.",
        "provider_account_copy_target_vault_unreadable" => "The receiving kernel's Vault could not be read; check that its file is present and readable, then retry the same account copy. Nothing was copied.",
        "provider_account_copy_credential_unreadable" => "The account's stored login could not be read from the Vault; sign in to the account again, then retry.",
        "provider_account_copy_target_credential_unreadable" => "The receiving kernel could not read the copied login back from its Vault; retry the same account copy.",
        "provider_account_copy_cli_missing" => "The provider's command-line tool is not installed on the receiving kernel; install it there, then retry the same account copy. Nothing was copied.",
        "provider_account_copy_replace_differs" => "This operation was already used with a different replace setting; start a new operation to copy with that setting.",
        "provider_account_copy_credential_too_large" => "The account's login is larger than an account copy carries (16 KiB). Nothing was copied.",

        "provider_account_copy_cancelled" => "Account copy was cancelled or expired; no login was transferred.",
        "provider_account_copy_conflict" => "A target profile or operation already exists; use a new operation with explicit replacement to replace the profile.",
        "provider_account_copy_rejected" => "The official provider rejected the account; sign in to the source account again.",
        "provider_account_copy_verification_failed" => "The official provider could not check the account; check its availability and retry.",
        "provider_account_copy_install_failed" => "Account installation did not finish; retry the same operation to recover.",
        TRANSFER_TARGET_NOT_OWNED => "This worker is not enrolled as your own kernel, so the Claude account was not copied to it. Enroll it under Machines, then send the prompt again.",
        TRANSFER_WORKER_OUTDATED => "This worker runs an older Chariox that cannot hold a copied Claude login. Update it, then send the prompt again. Nothing was sent.",
        TRANSFER_SIGN_IN_REQUIRED => "Claude needs a sign-in. Use the sign-in card in Chariox; your prompt continues afterwards.",
        _ => "Invalid provider account copy request.",
    }
}

/// Row 577: the refusals a leased Claude launch can give, each naming one step.
pub(crate) const TRANSFER_TARGET_NOT_OWNED: &str = "provider_account_transfer_target_not_owned";
pub(crate) const TRANSFER_WORKER_OUTDATED: &str = "provider_account_transfer_worker_outdated";
pub(crate) const TRANSFER_SIGN_IN_REQUIRED: &str = "provider_account_transfer_sign_in_required";

pub(crate) fn is_public_code(code: &str) -> bool {
    matches!(
        code,
        TRANSFER_TARGET_NOT_OWNED
            | TRANSFER_WORKER_OUTDATED
            | TRANSFER_SIGN_IN_REQUIRED
            | "provider_account_copy_unauthorized"
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
            | "provider_account_copy_credential_too_large"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_kernel_with_the_opencode_capability_handles_an_opencode_copy() {
        let base = vec![PROVIDER_ACCOUNT_COPY_CAPABILITY.to_string()];
        let with = vec![
            PROVIDER_ACCOUNT_COPY_CAPABILITY.to_string(),
            PROVIDER_ACCOUNT_COPY_OPENCODE_CAPABILITY.to_string(),
        ];
        for provider in ["claude", "codex"] {
            assert!(kernel_handles_copy_of(provider, &base));
        }
        assert!(!kernel_handles_copy_of("opencode", &base));
        assert!(kernel_handles_copy_of("opencode", &with));
    }

    #[test]
    fn the_new_refusals_are_public_and_say_what_to_do() {
        for code in [
            "provider_account_copy_vault_unreadable",
            "provider_account_copy_target_vault_unreadable",
            "provider_account_copy_credential_unreadable",
            "provider_account_copy_target_credential_unreadable",
            "provider_account_copy_cli_missing",
            "provider_account_copy_replace_differs",
            "provider_account_copy_credential_too_large",
        ] {
            assert!(is_public_code(code), "{code}");
            assert_ne!(
                failure_message(code),
                failure_message("invalid_request"),
                "{code}"
            );
        }
        assert!(!failure_message("provider_account_copy_cli_missing").contains("Claude"));
    }

    #[test]
    fn an_opencode_copy_between_two_kernels_is_a_valid_binding() {
        let binding = ProviderAccountCopyBinding {
            operation_id: "op".into(),
            provider: "opencode".into(),
            source_profile_id: "profile".into(),
            source_kernel_id: "source".into(),
            source_key_thumbprint: "a".repeat(64),
            target_kernel_id: "target".into(),
            target_key_thumbprint: "b".repeat(64),
            replace: false,
        };
        binding.validate().unwrap();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountCopyWait {
    pub operation_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
    pub target_label: String,
    pub expires_at_ms: u64,
}

impl ProviderAccountCopyWait {
    pub(crate) fn matches(&self, binding: &ProviderAccountCopyBinding) -> bool {
        self.operation_id == binding.operation_id
            && self.source_kernel_id == binding.source_kernel_id
            && self.target_kernel_id == binding.target_kernel_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderAccountCopyProgress {
    WaitingForUnlock { wait: ProviderAccountCopyWait },
    Installed { receipt: ProviderAccountCopyReceipt },
    Failed { operation_id: String, code: String },
}

pub(crate) enum ProviderAccountCopyOutcome {
    Installed(ProviderAccountCopyReceipt),
    Waiting(ProviderAccountCopyWait),
}

impl ProviderAccountCopyOutcome {
    pub(crate) fn into_response(self) -> crate::local::LocalDaemonResponse {
        match self {
            Self::Installed(receipt) => {
                crate::local::LocalDaemonResponse::ProviderAccountCopied { receipt }
            }
            Self::Waiting(wait) => {
                crate::local::LocalDaemonResponse::ProviderAccountCopyWaiting { wait }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountCopyUnlockRequest {
    pub interaction_id: String,
    pub expires_at_ms: u64,
}
