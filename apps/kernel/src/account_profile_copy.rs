//! Transaction boundary for an independent account copy. Receipts and profile
//! publication share one atomic registry write; credential bytes stay in Vault.
use super::*;
use crate::provider_account_copy::{
    failure, AccountCopyInstall, ProviderAccountCopyBinding, ProviderAccountCopyCredential,
    ProviderAccountCopyReceipt,
};
use sha2::{Digest, Sha256};

#[path = "account_profile_copy/native.rs"]
mod native;

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
    pub(crate) fn export_account_copy_credential(
        &self,
        config: &crate::config::DaemonConfig,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountCopyCredential, DaemonError> {
        let source = self.credential_source_profile_id(owner, provider, profile_id)?;
        if let Some(relative_path) =
            crate::provider_account_copy::native_copy_credential_path(provider)
        {
            let materialization = self
                .export_managed_context_materialization(owner, provider, &source)
                .map_err(|_| failure("provider_account_copy_profile"))?;
            let file = materialization
                .files
                .iter()
                .find(|file| file.relative_path == relative_path)
                .ok_or_else(|| failure("provider_account_copy_profile"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&file.contents_base64)
                .map_err(|_| failure("provider_account_copy_profile"))?;
            let value =
                String::from_utf8(bytes).map_err(|_| failure("provider_account_copy_profile"))?;
            if value.is_empty() || value.len() > 16 * 1024 {
                return Err(failure("provider_account_copy_profile"));
            }
            return Ok(ProviderAccountCopyCredential::new(value));
        }
        let selected =
            crate::provider::resolve_provider_account_credentials(config, owner, provider, &source)
                .map_err(|_| failure("provider_account_copy_vault_locked"))?;
        let value = selected
            .iter()
            .find(|(name, _)| *name == crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| failure("provider_account_copy_profile"))?;
        Ok(ProviderAccountCopyCredential::new(value))
    }

    pub(crate) fn install_account_copy(
        &self,
        config: &crate::config::DaemonConfig,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
        credential: &ProviderAccountCopyCredential,
    ) -> Result<AccountCopyInstall, DaemonError> {
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
            if recorded.binding != *binding {
                return Err(failure("provider_account_copy_conflict"));
            }
            if recorded.phase == AccountCopyPhase::Installed {
                return installed_receipt(config, owner, binding, credential, existing.is_some())
                    .map(AccountCopyInstall::Replay);
            }
            if recorded.credential_sha256 != digest {
                return Err(failure("provider_account_copy_conflict"));
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
        // Reserve the immutable operation before verification or Vault writes.
        // A crash must not let the same operation name a different account.
        if operation.is_none() {
            let mut prepared = guard.clone();
            prepared.account_copies.push(AccountCopyRecord {
                binding: binding.clone(),
                credential_sha256: digest.clone(),
                phase: AccountCopyPhase::Prepared,
            });
            self.persist_locked(&prepared).map_err(map_storage)?;
            *guard = prepared;
        }
        let expected_profile = existing.map(|index| guard.profiles[index].clone());
        drop(guard);
        let native_auth = if binding.provider == "codex" {
            Some(native::verify_codex_copy(&root, credential)?)
        } else if binding.provider == "opencode" {
            Some(native::check_opencode_copy(credential)?)
        } else {
            create_private_roots(&locator).map_err(map_storage)?;
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
            None
        };
        let mut guard = self.write_document().map_err(map_storage)?;
        let operation = guard
            .account_copies
            .iter()
            .position(|copy| {
                copy.binding.source_kernel_id == binding.source_kernel_id
                    && copy.binding.source_key_thumbprint == binding.source_key_thumbprint
                    && copy.binding.operation_id == binding.operation_id
            })
            .ok_or_else(|| failure("provider_account_copy_conflict"))?;
        let recorded = &guard.account_copies[operation];
        if recorded.binding != *binding {
            return Err(failure("provider_account_copy_conflict"));
        }
        let existing = guard.profiles.iter().position(|stored| {
            stored.public.owner_user_id == owner
                && stored.public.provider == binding.provider
                && stored.public.profile_id == profile_id
        });
        // An identical retry may have published while this verifier was running.
        if recorded.phase == AccountCopyPhase::Installed {
            return installed_receipt(config, owner, binding, credential, existing.is_some())
                .map(AccountCopyInstall::Replay);
        }
        if recorded.credential_sha256 != digest {
            return Err(failure("provider_account_copy_conflict"));
        }
        if existing.map(|index| &guard.profiles[index]) != expected_profile.as_ref() {
            return Err(failure("provider_account_copy_conflict"));
        }
        // Never publish an authenticated account until its own credential exists.
        // An orphan credential after a crash is overwritten by the same retry; it
        // has no registered profile and therefore cannot be selected for launch.
        // OpenCode keeps its login only in its own native store, as it does when it signs in.
        if binding.provider != "opencode" {
            crate::provider::store_provider_account_copy_credential(
                config,
                owner,
                &binding.provider,
                &profile_id,
                credential.value(),
            )
            .map_err(map_storage)?;
        }
        if let Some(auth) = native_auth {
            create_private_roots(&locator).map_err(map_storage)?;
            match &locator {
                ProviderAccountLocator::Codex { codex_home } => {
                    enforce_codex_file_credentials(codex_home).map_err(map_storage)?;
                    atomic_write_private(&codex_home.join("auth.json"), auth.as_bytes())
                        .map_err(map_storage)?;
                }
                ProviderAccountLocator::Opencode { .. } => {
                    let path = materialization_destination(&locator, "data/opencode/auth.json")
                        .map_err(map_storage)?;
                    let parent = path
                        .parent()
                        .ok_or_else(|| failure("provider_account_copy_install_failed"))?;
                    validate_materialization_root(parent).map_err(map_storage)?;
                    atomic_write_private(&path, auth.as_bytes()).map_err(map_storage)?;
                }
                ProviderAccountLocator::Claude { .. } => unreachable!(),
            }
        }
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
        public.auth_error = None;
        public.identity_summary = None;
        public.plan = None;
        public.usage = ProviderAccountUsageSnapshot::unavailable(&profile_id, &binding.provider);
        if binding.provider == "opencode" {
            // Nothing here can check an OpenCode login; its state is read when it is first used.
            public.auth_state = ProviderAccountAuthState::Unknown;
            public.last_validated_at_ms = None;
        } else {
            public.auth_state = ProviderAccountAuthState::Authenticated;
            public.last_validated_at_ms = Some(crate::session::unix_epoch_ms());
        }
        // A remote launch from the source kernel with this account runs on this copy.
        let transferred_from = Some(TransferredAccountOrigin {
            source_kernel_id: binding.source_kernel_id.clone(),
            source_profile_id: binding.source_profile_id.clone(),
        });
        if let Some(index) = existing {
            next.profiles[index].public = public;
            next.profiles[index].transferred_from = transferred_from;
            next.profiles[index].local_credential_source = None;
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
                transferred_from,
                local_credential_source: None,
            });
        }
        next.account_copies[operation].phase = AccountCopyPhase::Installed;
        self.persist_locked(&next).map_err(map_storage)?;
        *guard = next;
        Ok(AccountCopyInstall::Fresh(
            ProviderAccountCopyReceipt::installed(binding),
        ))
    }

    pub(crate) fn install_account_link(
        &self,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
    ) -> Result<ProviderAccountCopyReceipt, DaemonError> {
        binding.validate()?;
        let mut document = self.write_document()?;
        let source = find_stored_profile(
            &document,
            owner,
            &binding.provider,
            &binding.source_profile_id,
        )
        .filter(|source| {
            source.public.profile_id == binding.source_profile_id
                && source.public.auth_state == ProviderAccountAuthState::Authenticated
        })
        .cloned()
        .ok_or_else(|| failure("provider_account_copy_profile"))?;
        let id = binding.target_profile_id();
        let existing = document.profiles.iter().position(|stored| {
            stored.public.owner_user_id == owner
                && stored.public.provider == binding.provider
                && stored.public.profile_id == id
        });
        if let Some(recorded) = document.account_copies.iter().find(|recorded| {
            recorded.binding.source_kernel_id == binding.source_kernel_id
                && recorded.binding.source_key_thumbprint == binding.source_key_thumbprint
                && recorded.binding.operation_id == binding.operation_id
        }) {
            if recorded.binding == *binding
                && recorded.phase == AccountCopyPhase::Installed
                && existing.is_some_and(|index| document.profiles[index].locator == source.locator)
            {
                return Ok(ProviderAccountCopyReceipt::installed(binding));
            }
            return Err(failure("provider_account_copy_conflict"));
        }
        if existing.is_some() && !binding.replace {
            return Err(failure("provider_account_copy_conflict"));
        }
        let mut linked = source;
        linked.public.profile_id = id.clone();
        linked.public.label = format!("Linked account {}", &id[5..13]);
        linked.public.origin = ProviderAccountProfileOrigin::Linked;
        linked.public.is_default = false;
        linked.public.usage.profile_id = id;
        linked.public.materializations.clear();
        linked.label_source = Some(ProviderAccountLabelSource::Explicit);
        linked.materialized_replica = false;
        linked.pending_native_validation = false;
        linked.legacy_unpinned_replica = false;
        linked.replica_previous_default_profile_id = None;
        linked.replica_root_identity = None;
        linked.managed_context_replica = None;
        linked.local_credential_source = Some(binding.source_profile_id.clone());
        linked.transferred_from = Some(TransferredAccountOrigin {
            source_kernel_id: binding.source_kernel_id.clone(),
            source_profile_id: binding.source_profile_id.clone(),
        });
        if let Some(index) = existing {
            document.profiles[index] = linked;
        } else {
            document.profiles.push(linked);
        }
        document.account_copies.push(AccountCopyRecord {
            binding: binding.clone(),
            credential_sha256: String::new(),
            phase: AccountCopyPhase::Installed,
        });
        self.persist_locked(&document)?;
        Ok(ProviderAccountCopyReceipt::installed(binding))
    }

    pub(crate) fn credential_source_profile_id(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<String, DaemonError> {
        let document = self.read_document()?;
        let provider = normalize_provider(provider)?;
        let mut source = resolve_stored_profile(&document, owner, provider, profile_id)?;
        let mut visited = std::collections::BTreeSet::new();
        while let Some(source_id) = &source.local_credential_source {
            if !visited.insert(source.public.profile_id.as_str()) {
                return Err(registry_error(
                    "resolve linked account",
                    "the linked account forms a cycle; remove the link and create it again",
                ));
            }
            source = find_stored_profile(&document, owner, provider, source_id)
                .filter(|candidate| candidate.locator.same_credential_scope(&source.locator))
                .ok_or_else(|| registry_error("resolve linked account", format!(
                    "source account `{}` is unavailable; sign in again to that account on this machine, or remove this link and create it again", source_id,
                )))?;
        }
        Ok(source.public.profile_id.clone())
    }
}

fn installed_receipt(
    config: &crate::config::DaemonConfig,
    owner: &str,
    binding: &ProviderAccountCopyBinding,
    credential: &ProviderAccountCopyCredential,
    profile_exists: bool,
) -> Result<ProviderAccountCopyReceipt, DaemonError> {
    if !profile_exists {
        return Err(failure("provider_account_copy_conflict"));
    }
    if crate::provider_account_copy::native_copy_credential_path(&binding.provider).is_some() {
        // Installed receipts are immutable. The official provider or a fresh
        // device login may already have replaced the transferred credential.
        return Ok(ProviderAccountCopyReceipt::installed(binding));
    }
    let selected = crate::provider::resolve_provider_account_credentials(
        config,
        owner,
        &binding.provider,
        &binding.target_profile_id(),
    )
    .map_err(|_| failure("provider_account_copy_vault_locked"))?;
    if !selected.iter().any(|(name, value)| {
        name == crate::provider::CLAUDE_OAUTH_TOKEN_ENV && value == credential.value()
    }) {
        return Err(failure("provider_account_copy_conflict"));
    }
    Ok(ProviderAccountCopyReceipt::installed(binding))
}
