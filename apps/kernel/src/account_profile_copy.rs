//! Transaction boundary for an independent account copy. Receipts and profile
//! publication share one atomic registry write; credential bytes stay in Vault.
use super::*;
use crate::provider_account_copy::{
    failure, ProviderAccountCopyBinding, ProviderAccountCopyCredential, ProviderAccountCopyReceipt,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AccountCopyRecord {
    binding: ProviderAccountCopyBinding,
    credential_sha256: String,
    #[serde(default)]
    phase: AccountCopyPhase,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AccountCopyPhase {
    Prepared,
    #[default]
    Installed,
}

impl ProviderAccountProfileRegistry {
    pub(crate) fn install_account_copy(
        &self,
        config: &crate::config::DaemonConfig,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
        credential: &ProviderAccountCopyCredential,
    ) -> Result<ProviderAccountCopyReceipt, DaemonError> {
        binding.validate()?;
        let map_storage = |_| failure("provider_account_copy_install_failed");
        let mut guard = self.write_document().map_err(map_storage)?;
        let digest = format!("{:x}", Sha256::digest(credential.value().as_bytes()));
        let profile_id = binding.target_profile_id();
        let existing = guard.profiles.iter().position(|stored| {
            stored.public.owner_user_id == owner
                && stored.public.provider == binding.provider
                && stored.public.profile_id == profile_id
        });
        let operation = guard.account_copies.iter().position(|copy| {
            copy.binding.source_kernel_id == binding.source_kernel_id
                && copy.binding.source_key_thumbprint == binding.source_key_thumbprint
                && copy.binding.operation_id == binding.operation_id
        });
        if let Some(index) = operation {
            let recorded = &guard.account_copies[index];
            if recorded.binding != *binding || recorded.credential_sha256 != digest {
                return Err(failure("provider_account_copy_conflict"));
            }
            if recorded.phase == AccountCopyPhase::Installed {
                if existing.is_none() {
                    return Err(failure("provider_account_copy_conflict"));
                }
                let selected = crate::provider::resolve_provider_account_credentials(
                    config,
                    owner,
                    &binding.provider,
                    &profile_id,
                )
                .map_err(|_| failure("provider_account_copy_vault_locked"))?;
                if !selected.iter().any(|(name, value)| {
                    name == crate::provider::CLAUDE_OAUTH_TOKEN_ENV && value == credential.value()
                }) {
                    return Err(failure("provider_account_copy_conflict"));
                }
                return Ok(ProviderAccountCopyReceipt::installed(binding));
            }
        }
        if existing.is_some() && !binding.replace {
            return Err(failure("provider_account_copy_conflict"));
        }
        let root = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("provider-accounts")
            .join(safe_path_component(owner))
            .join(&binding.provider)
            .join(&profile_id);
        let locator =
            ProviderAccountLocator::managed(&binding.provider, &root).map_err(map_storage)?;
        if existing.is_some_and(|index| guard.profiles[index].locator != locator) {
            return Err(failure("provider_account_copy_conflict"));
        }
        // A deterministic root is also the recovery point for an interrupted install.
        create_private_roots(&locator).map_err(map_storage)?;
        // Reserve the immutable operation before verification or Vault writes.
        // A crash must not let the same operation name a different account.
        let operation = match operation {
            Some(index) => index,
            None => {
                let mut prepared = guard.clone();
                let index = prepared.account_copies.len();
                prepared.account_copies.push(AccountCopyRecord {
                    binding: binding.clone(),
                    credential_sha256: digest,
                    phase: AccountCopyPhase::Prepared,
                });
                self.persist_locked(&prepared).map_err(map_storage)?;
                *guard = prepared;
                index
            }
        };
        let mut environment = locator.environment();
        environment.insert(
            crate::provider::CLAUDE_OAUTH_TOKEN_ENV.into(),
            credential.value().into(),
        );
        let checked = crate::provider::resolve_claude_executable()
            .map_err(|_| failure("provider_account_copy_verification_failed"))
            .and_then(|executable| {
                crate::provider::verify_claude_setup_token(&executable, &environment).map_err(
                    |error| match error {
                        crate::provider::ClaudeCredentialCheckError::Rejected => {
                            failure("provider_account_copy_rejected")
                        }
                        crate::provider::ClaudeCredentialCheckError::Inconclusive(_) => {
                            failure("provider_account_copy_verification_failed")
                        }
                    },
                )
            });
        if let Some(value) = environment.get_mut(crate::provider::CLAUDE_OAUTH_TOKEN_ENV) {
            zeroize::Zeroize::zeroize(value);
        }
        checked?;
        // Never publish an authenticated account until its own credential exists.
        // An orphan credential after a crash is overwritten by the same retry; it
        // has no registered profile and therefore cannot be selected for launch.
        crate::provider::store_provider_account_credential(
            config,
            owner,
            &binding.provider,
            &profile_id,
            credential.value(),
            true,
        )
        .map_err(map_storage)?;
        let mut next = guard.clone();
        let mut public = existing
            .map(|index| next.profiles[index].public.clone())
            .unwrap_or_else(|| {
                new_public_profile(
                    owner,
                    &binding.provider,
                    &profile_id,
                    &format!("Copied account {}", &profile_id[5..13]),
                    ProviderAccountProfileOrigin::CharioxCreated,
                    false,
                )
            });
        public.auth_state = ProviderAccountAuthState::Authenticated;
        public.last_validated_at_ms = Some(crate::session::unix_epoch_ms());
        if let Some(index) = existing {
            next.profiles[index].public = public;
        } else {
            next.profiles.push(StoredProviderAccountProfile {
                public,
                locator,
                label_source: Some(ProviderAccountLabelSource::Explicit),
                materialized_replica: false,
                pending_native_validation: false,
                legacy_unpinned_replica: false,
                replica_previous_default_profile_id: None,
                replica_root_identity: None,
                managed_context_replica: None,
            });
        }
        next.account_copies[operation].phase = AccountCopyPhase::Installed;
        self.persist_locked(&next).map_err(map_storage)?;
        *guard = next;
        Ok(ProviderAccountCopyReceipt::installed(binding))
    }
}
