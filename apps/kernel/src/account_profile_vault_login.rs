//! MP-08 / MP-09 / MP-10 / MP-11: a Vault-backed login moves in a managed-context
//! transfer. The source seals it for the target's relay key; the target verifies it,
//! writes it into its own Vault under the same deterministic handle and publishes the
//! profile authenticated only afterwards.
use super::*;
use crate::config::DaemonConfig;
use crate::provider_vault_login::{self, VaultLoginBinding, VaultLoginMaterialization};

/// Who seals for whom: the source kernel's key, the target's public key and the transfer's identity.
pub(crate) struct VaultLoginSeal<'a> {
    pub source_private_key: &'a str,
    pub target_public_key: &'a str,
    pub context_id: &'a str,
    pub source_kernel_id: &'a str,
    pub target_kernel_id: &'a str,
}

/// What the target checks a seal against: its own expected binding and the source key it pinned.
pub(crate) struct VaultLoginOpen<'a> {
    pub target_private_key: &'a str,
    pub source_key_thumbprint: &'a str,
    pub source_kernel_id: &'a str,
    pub target_kernel_id: &'a str,
}

impl ProviderAccountProfileRegistry {
    /// Whether the profile's login is a Vault credential, which only the Vault can carry.
    pub(crate) fn login_is_vault_credential(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let profile = self.get(owner_user_id, provider, profile_id)?;
        crate::provider::provider_account_credential_uses_vault(
            owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )
    }

    pub(crate) fn export_vault_login(
        &self,
        config: &DaemonConfig,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        seal: &VaultLoginSeal<'_>,
    ) -> Result<VaultLoginMaterialization, DaemonError> {
        let profile = self.get(owner_user_id, provider, profile_id)?;
        if profile.auth_state != ProviderAccountAuthState::Authenticated
            || !crate::provider::provider_account_credential_uses_vault(
                owner_user_id,
                &profile.provider,
                &profile.profile_id,
            )?
        {
            return Err(registry_error(
                "export account Vault login",
                "the selected account is not signed in with a Vault credential",
            ));
        }
        let selected = crate::provider::resolve_provider_account_credentials(
            config,
            owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )?;
        let secret = selected
            .iter()
            .next()
            .map(|(_, value)| zeroize::Zeroizing::new(value.to_string()))
            .ok_or_else(|| {
                registry_error(
                    "export account Vault login",
                    "the selected account has no Vault credential",
                )
            })?;
        provider_vault_login::seal(
            seal.source_private_key,
            seal.target_public_key,
            &VaultLoginBinding {
                context_id: seal.context_id.to_string(),
                source_kernel_id: seal.source_kernel_id.to_string(),
                target_kernel_id: seal.target_kernel_id.to_string(),
                provider: profile.provider.clone(),
                profile_id: profile.profile_id.clone(),
            },
            ProviderAccountReplicaMetadata {
                owner_user_id: profile.owner_user_id,
                provider: profile.provider,
                profile_id: profile.profile_id,
                label: profile.label,
                origin: profile.origin,
                is_default: profile.is_default,
            },
            &secret,
        )
    }

    pub(crate) fn install_vault_login_replica(
        &self,
        config: &DaemonConfig,
        owner_user_id: &str,
        context_id: &str,
        package_sha256: &str,
        open: &VaultLoginOpen<'_>,
        login: &VaultLoginMaterialization,
    ) -> Result<ManagedContextProviderAccountReceipt, DaemonError> {
        let operation = "install account Vault login";
        if login.profile.owner_user_id != owner_user_id {
            return Err(registry_error(
                operation,
                "the Vault login owner does not match the transfer owner",
            ));
        }
        let provider = normalize_provider(&login.profile.provider)?;
        let profile_id = validate_profile_id(&login.profile.profile_id)?;
        let materialization_sha256 = provider_vault_login::materialization_sha256(login)?;
        let receipt = ManagedContextProviderAccountReceipt {
            context_id: context_id.to_string(),
            package_sha256: package_sha256.to_string(),
            materialization_sha256: materialization_sha256.clone(),
            provider: provider.to_string(),
            profile_id: profile_id.to_string(),
        };
        let intent = ManagedContextReplicaIntent {
            context_id,
            package_sha256,
            materialization_sha256: &materialization_sha256,
        };
        let existing = |document: &RegistryDocument| {
            find_stored_profile(document, owner_user_id, provider, profile_id).map(|stored| {
                stored
                    .managed_context_replica
                    .as_ref()
                    .is_some_and(|binding| binding.vault_login && binding.matches(intent))
            })
        };
        match existing(&*self.read_document()?) {
            Some(true) => return Ok(receipt),
            Some(false) => {
                return Err(registry_error(
                    operation,
                    "refusing to replace an existing provider account profile",
                ))
            }
            None => {}
        }
        let secret = provider_vault_login::open(
            open.target_private_key,
            open.source_key_thumbprint,
            &VaultLoginBinding {
                context_id: context_id.to_string(),
                source_kernel_id: open.source_kernel_id.to_string(),
                target_kernel_id: open.target_kernel_id.to_string(),
                provider: provider.to_string(),
                profile_id: profile_id.to_string(),
            },
            login,
        )?;
        let managed_root = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("provider-accounts")
            .join(safe_path_component(owner_user_id))
            .join(provider)
            .join(profile_id);
        let locator = ProviderAccountLocator::managed(provider, &managed_root)?;
        create_private_roots(&locator)?;
        crate::provider::verify_account_login(provider, &locator.environment(), &secret)?;
        // A login that fails the official check is never written. An orphan value left by a
        // crash after this point has no registered profile, so nothing can select it, and a
        // retry replaces it.
        crate::provider::store_provider_account_credential(
            config,
            owner_user_id,
            provider,
            profile_id,
            &secret,
            true,
        )?;
        let published = (|| {
            let mut document = self.write_document()?;
            if existing(&document).is_some() {
                return Err(registry_error(
                    operation,
                    "refusing to replace an existing provider account profile",
                ));
            }
            let mut next = document.clone();
            let mut public = new_public_profile(
                owner_user_id,
                provider,
                profile_id,
                &login.profile.label,
                ProviderAccountProfileOrigin::CharioxCreated,
                false,
            );
            public.auth_state = ProviderAccountAuthState::Authenticated;
            public.last_validated_at_ms = Some(crate::session::unix_epoch_ms());
            next.profiles.push(StoredProviderAccountProfile {
                public,
                locator: locator.clone(),
                label_source: Some(ProviderAccountLabelSource::Explicit),
                materialized_replica: false,
                pending_native_validation: false,
                legacy_unpinned_replica: false,
                replica_previous_default_profile_id: None,
                replica_root_identity: None,
                managed_context_replica: Some(ManagedContextReplicaBinding {
                    context_id: context_id.to_string(),
                    package_sha256: package_sha256.to_string(),
                    materialization_sha256: materialization_sha256.clone(),
                    replaced_profile: None,
                    previous_default_profile_id: None,
                    vault_login: true,
                }),
                transferred_from: None,
            });
            self.persist_locked(&next)?;
            *document = next;
            Ok(())
        })();
        if let Err(error) = published {
            let _ = crate::provider::remove_provider_account_credential(
                config,
                owner_user_id,
                provider,
                profile_id,
            );
            return Err(error);
        }
        Ok(receipt)
    }

    /// Undoes one account a managed-context import installed: a Vault login also loses its
    /// credential record and Vault value, a file login goes through the file rollback.
    pub(crate) fn rollback_managed_context_account(
        &self,
        config: &DaemonConfig,
        owner_user_id: &str,
        receipt: &ManagedContextProviderAccountReceipt,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(&receipt.provider)?;
        let profile_id = validate_profile_id(&receipt.profile_id)?;
        let mut document = self.write_document()?;
        let Some(index) = document.profiles.iter().position(|stored| {
            stored.public.owner_user_id == owner_user_id
                && stored.public.provider == provider
                && stored.public.profile_id == profile_id
        }) else {
            drop(document);
            return self.rollback_managed_context_replica(owner_user_id, receipt);
        };
        let Some(binding) = document.profiles[index]
            .managed_context_replica
            .clone()
            .filter(|binding| binding.vault_login)
        else {
            drop(document);
            return self.rollback_managed_context_replica(owner_user_id, receipt);
        };
        if binding.context_id != receipt.context_id
            || binding.package_sha256 != receipt.package_sha256
            || binding.materialization_sha256 != receipt.materialization_sha256
        {
            return Err(registry_error(
                "roll back account Vault login",
                "provider account belongs to another managed context",
            ));
        }
        let mut next = document.clone();
        let removed = next.profiles.remove(index);
        self.persist_locked(&next)?;
        *document = next;
        drop(document);
        crate::provider::remove_provider_account_credential(
            config,
            owner_user_id,
            provider,
            profile_id,
        )?;
        for root in removed.locator.roots() {
            let _ = fs::remove_dir_all(root);
        }
        Ok(())
    }
}
