//! Transaction boundary for an independent account copy. The receiving
//! profile and receipt are published in one registry write.
use super::*;
use crate::provider_account_copy::{
    failure, native_copy_credential_path, ProviderAccountCopyBinding,
    ProviderAccountCopyCredential, ProviderAccountCopyReceipt,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

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

struct NativeCopyWrite {
    file: replica_refresh::ReplicaFile,
    previous: Option<Zeroizing<Vec<u8>>>,
}

impl NativeCopyWrite {
    fn rollback(&self) -> Result<(), DaemonError> {
        self.file
            .replace(self.previous.as_ref().map(|bytes| bytes.as_slice()))
    }
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
        let native_bytes = native_copy_credential_path(&binding.provider)
            .map(|_| decode_native_copy_credential(credential))
            .transpose()?;
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
        if let Some(index) = operation {
            let recorded = &guard.account_copies[index];
            if recorded.binding != *binding || recorded.credential_sha256 != digest {
                return Err(recorded_operation_conflict(
                    &recorded.binding,
                    binding,
                    &recorded.credential_sha256,
                    &digest,
                ));
            }
            if recorded.phase == AccountCopyPhase::Installed {
                if let Some(bytes) = &native_bytes {
                    return installed_native_receipt(
                        binding,
                        &root,
                        &locator,
                        bytes,
                        existing.is_some(),
                    );
                }
                return installed_receipt(config, owner, binding, credential, existing.is_some());
            }
        }
        if existing.is_some() && !binding.replace {
            return Err(failure("provider_account_copy_conflict"));
        }
        if existing.is_some_and(|index| guard.profiles[index].locator != locator) {
            return Err(failure("provider_account_copy_conflict"));
        }
        if native_bytes.is_some()
            && operation.is_none()
            && existing.is_none()
            && path_entry_exists(&root).map_err(map_storage)?
        {
            return Err(failure("provider_account_copy_conflict"));
        }
        // Reserve the immutable operation before verification or credential writes.
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
        if native_bytes.is_none() {
            create_private_roots(&locator).map_err(map_storage)?;
            verify_claude_copy(&locator, credential)?;
        }
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
        if recorded.binding != *binding || recorded.credential_sha256 != digest {
            return Err(failure("provider_account_copy_conflict"));
        }
        let existing = guard.profiles.iter().position(|stored| {
            stored.public.owner_user_id == owner
                && stored.public.provider == binding.provider
                && stored.public.profile_id == profile_id
        });
        // An identical retry may have published while this verifier was running.
        if recorded.phase == AccountCopyPhase::Installed {
            if let Some(bytes) = &native_bytes {
                return installed_native_receipt(
                    binding,
                    &root,
                    &locator,
                    bytes,
                    existing.is_some(),
                );
            }
            return installed_receipt(config, owner, binding, credential, existing.is_some());
        }
        if existing.map(|index| &guard.profiles[index]) != expected_profile.as_ref() {
            return Err(failure("provider_account_copy_conflict"));
        }
        let native_write = if let Some(bytes) = &native_bytes {
            Some(write_native_copy(binding, &root, &locator, bytes).map_err(map_storage)?)
        } else {
            // A crash can leave an unregistered login. The same retry replaces it.
            crate::provider::store_provider_account_credential(
                config,
                owner,
                &binding.provider,
                &profile_id,
                credential.value(),
                true,
            )
            .map_err(map_storage)?;
            None
        };
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
        if native_bytes.is_some() {
            public.auth_state = ProviderAccountAuthState::Unknown;
            public.last_validated_at_ms = None;
            public.identity_summary = None;
            public.plan = None;
            public.services.clear();
            public.usage =
                ProviderAccountUsageSnapshot::unavailable(&profile_id, &binding.provider);
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
            });
        }
        next.account_copies[operation].phase = AccountCopyPhase::Installed;
        if let Err(error) = self.persist_locked(&next) {
            if let Some(write) = &native_write {
                // The registry write can fail after rename; restore both sides.
                let registry_rollback = self.persist_locked(&guard);
                let file_rollback = write.rollback();
                registry_rollback.and(file_rollback).map_err(map_storage)?;
            }
            return Err(map_storage(error));
        }
        *guard = next;
        Ok(ProviderAccountCopyReceipt::installed(binding))
    }
}

fn decode_native_copy_credential(
    credential: &ProviderAccountCopyCredential,
) -> Result<Zeroizing<Vec<u8>>, DaemonError> {
    let invalid = || failure("invalid_request");
    if credential.value().len() > MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES.div_ceil(3) * 4 {
        return Err(invalid());
    }
    let bytes = Zeroizing::new(
        base64::engine::general_purpose::STANDARD
            .decode(credential.value())
            .map_err(|_| invalid())?,
    );
    if bytes.is_empty() || bytes.len() > MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}

fn native_copy_file(
    binding: &ProviderAccountCopyBinding,
    root: &Path,
    locator: &ProviderAccountLocator,
) -> Result<replica_refresh::ReplicaFile, DaemonError> {
    let path =
        native_copy_credential_path(&binding.provider).ok_or_else(|| failure("invalid_request"))?;
    let destination = materialization_destination(locator, path)?;
    let relative = destination
        .strip_prefix(root)
        .map_err(|_| failure("invalid_request"))?;
    replica_refresh::ReplicaFile::open(root, relative)
}

fn write_native_copy(
    binding: &ProviderAccountCopyBinding,
    root: &Path,
    locator: &ProviderAccountLocator,
    bytes: &[u8],
) -> Result<NativeCopyWrite, DaemonError> {
    validate_materialization_root(root)?;
    for directory in locator.roots() {
        validate_materialization_root(directory)?;
    }
    create_private_roots(locator)?;
    let file = native_copy_file(binding, root, locator)?;
    let previous = file
        .read(MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES)?
        .map(Zeroizing::new);
    if let ProviderAccountLocator::Codex { codex_home } = locator {
        enforce_codex_file_credentials(codex_home)?;
    }
    let write = NativeCopyWrite { file, previous };
    if let Err(error) = write.file.replace(Some(bytes)) {
        write.rollback()?;
        return Err(error);
    }
    Ok(write)
}

fn installed_native_receipt(
    binding: &ProviderAccountCopyBinding,
    root: &Path,
    locator: &ProviderAccountLocator,
    expected: &[u8],
    profile_exists: bool,
) -> Result<ProviderAccountCopyReceipt, DaemonError> {
    let conflict = || failure("provider_account_copy_conflict");
    if !profile_exists {
        return Err(conflict());
    }
    let login = native_copy_file(binding, root, locator)
        .and_then(|file| file.read(MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES))
        .map_err(|_| conflict())?
        .map(Zeroizing::new);
    if login
        .as_ref()
        .is_none_or(|bytes| bytes.as_slice() != expected)
    {
        return Err(conflict());
    }
    Ok(ProviderAccountCopyReceipt::installed(binding))
}

/// The recorded operation differs from the retry; name the one difference an
/// owner can act on instead of the generic conflict.
fn recorded_operation_conflict(
    recorded: &ProviderAccountCopyBinding,
    retry: &ProviderAccountCopyBinding,
    recorded_digest: &str,
    retry_digest: &str,
) -> DaemonError {
    let mut same_but_replace = recorded.clone();
    same_but_replace.replace = retry.replace;
    if same_but_replace == *retry
        && recorded.replace != retry.replace
        && recorded_digest == retry_digest
    {
        failure("provider_account_copy_replace_differs")
    } else {
        failure("provider_account_copy_conflict")
    }
}

/// A receiver without the Claude CLI cannot verify the login; say so rather
/// than "verification did not complete".
fn claude_executable_failure(error: &DaemonError) -> DaemonError {
    if matches!(error, DaemonError::ProviderExecutableNotFound { .. }) {
        failure("provider_account_copy_cli_missing")
    } else {
        failure("provider_account_copy_verification_failed")
    }
}

fn verify_claude_copy(
    locator: &ProviderAccountLocator,
    credential: &ProviderAccountCopyCredential,
) -> Result<(), DaemonError> {
    let mut environment = locator.environment();
    environment.insert(
        crate::provider::CLAUDE_OAUTH_TOKEN_ENV.into(),
        credential.value().into(),
    );
    let checked = crate::provider::resolve_claude_executable()
        .map_err(|error| claude_executable_failure(&error))
        .and_then(|executable| {
            crate::provider::verify_claude_setup_token(&executable, &environment).map_err(|error| {
                match error {
                    crate::provider::ClaudeCredentialCheckError::Rejected => {
                        failure("provider_account_copy_rejected")
                    }
                    crate::provider::ClaudeCredentialCheckError::Inconclusive(_) => {
                        failure("provider_account_copy_verification_failed")
                    }
                }
            })
        });
    if let Some(value) = environment.get_mut(crate::provider::CLAUDE_OAUTH_TOKEN_ENV) {
        zeroize::Zeroize::zeroize(value);
    }
    checked.map(|_| ())
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
    let selected = crate::provider::resolve_provider_account_credentials(
        config,
        owner,
        &binding.provider,
        &binding.target_profile_id(),
    )
    .map_err(|error| crate::provider_account_copy::credential_read_failure(&error))?;
    if !selected.iter().any(|(name, value)| {
        name == crate::provider::CLAUDE_OAUTH_TOKEN_ENV && value == credential.value()
    }) {
        return Err(failure("provider_account_copy_conflict"));
    }
    Ok(ProviderAccountCopyReceipt::installed(binding))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_binding(operation: &str) -> ProviderAccountCopyBinding {
        ProviderAccountCopyBinding {
            operation_id: operation.into(),
            provider: "codex".into(),
            source_profile_id: "profile-1".into(),
            source_kernel_id: "source".into(),
            source_key_thumbprint: "a".repeat(64),
            target_kernel_id: "target".into(),
            target_key_thumbprint: "b".repeat(64),
            replace: false,
        }
    }

    fn native_credential() -> ProviderAccountCopyCredential {
        ProviderAccountCopyCredential::new(
            base64::engine::general_purpose::STANDARD
                .encode(br#"{"fixture":{"type":"api","key":"fixture-native-login"}}"#),
        )
    }

    fn code_of(error: DaemonError) -> &'static str {
        match error {
            DaemonError::ProviderAccountCopy { code, .. } => code,
            other => panic!("expected an account copy refusal, got {other}"),
        }
    }

    #[test]
    fn retrying_an_operation_with_only_replace_changed_names_that_difference() {
        let root = crate::test_support::TestWorktree::new("account-copy-replace-differs");
        let registry =
            ProviderAccountProfileRegistry::open(root.path().join("registry.json")).unwrap();
        let config = crate::config::DaemonConfig::for_tests();
        let credential = native_credential();
        let mut binding = native_binding("copy-replace");
        registry
            .install_account_copy(&config, "owner", &binding, &credential)
            .unwrap();
        binding.replace = true;
        let error = registry
            .install_account_copy(&config, "owner", &binding, &credential)
            .unwrap_err();
        assert_eq!(code_of(error), "provider_account_copy_replace_differs");
        // Another difference stays the generic conflict.
        binding.source_profile_id = "profile-2".into();
        let error = registry
            .install_account_copy(&config, "owner", &binding, &credential)
            .unwrap_err();
        assert_eq!(code_of(error), "provider_account_copy_conflict");
    }

    #[test]
    fn a_receiver_without_the_claude_cli_says_so() {
        let missing = DaemonError::ProviderExecutableNotFound {
            adapter_key: "claude".into(),
            executable: "claude".into(),
        };
        assert_eq!(
            code_of(claude_executable_failure(&missing)),
            "provider_account_copy_cli_missing"
        );
        let other = DaemonError::LocalTransport {
            operation: "resolve Claude",
            message: "unreadable".into(),
        };
        assert_eq!(
            code_of(claude_executable_failure(&other)),
            "provider_account_copy_verification_failed"
        );
    }

    #[test]
    fn only_a_locked_vault_is_reported_as_locked() {
        let locked = DaemonError::LocalTransport {
            operation: "credential_vault_locked",
            message: "locked".into(),
        };
        assert_eq!(
            code_of(crate::provider_account_copy::credential_read_failure(
                &locked
            )),
            "provider_account_copy_vault_locked"
        );
        let other = DaemonError::LocalTransport {
            operation: "read credential",
            message: "record missing".into(),
        };
        assert_eq!(
            code_of(crate::provider_account_copy::credential_read_failure(
                &other
            )),
            "provider_account_copy_credential_unreadable"
        );
    }

    #[test]
    fn native_account_copy_installs_retries_and_protects_the_receiving_profile() {
        let root = crate::test_support::TestWorktree::new("native-account-copy");
        let path = root.path().join("registry.json");
        let config = crate::config::DaemonConfig::for_tests();
        for (provider, auth_path) in [
            ("opencode", "data/opencode/auth.json"),
            ("codex", "codex/auth.json"),
        ] {
            let registry = ProviderAccountProfileRegistry::open(path.clone()).unwrap();
            let mut binding = ProviderAccountCopyBinding {
                operation_id: format!("copy-{provider}"),
                provider: provider.into(),
                source_profile_id: "profile-1".into(),
                source_kernel_id: "source".into(),
                source_key_thumbprint: "a".repeat(64),
                target_kernel_id: "target".into(),
                target_key_thumbprint: "b".repeat(64),
                replace: false,
            };
            let bytes = br#"{"fixture":{"type":"api","key":"fixture-native-login"}}"#;
            let credential = ProviderAccountCopyCredential::new(
                base64::engine::general_purpose::STANDARD.encode(bytes),
            );
            let receipt = registry
                .install_account_copy(&config, "owner", &binding, &credential)
                .unwrap();
            let target_root = root
                .path()
                .join("provider-accounts/owner")
                .join(provider)
                .join(&receipt.account_profile);
            assert_eq!(fs::read(target_root.join(auth_path)).unwrap(), bytes);
            assert_eq!(
                registry
                    .transferred_profile("owner", provider, "source", "profile-1")
                    .unwrap()
                    .unwrap()
                    .profile_id,
                receipt.account_profile,
            );
            drop(registry);
            let registry = ProviderAccountProfileRegistry::open(path.clone()).unwrap();
            assert_eq!(
                registry
                    .install_account_copy(&config, "owner", &binding, &credential)
                    .unwrap(),
                receipt
            );
            binding.operation_id.push_str("-new");
            assert!(matches!(
                registry.install_account_copy(&config, "owner", &binding, &credential),
                Err(DaemonError::ProviderAccountCopy {
                    code: "provider_account_copy_conflict",
                    ..
                })
            ));
            assert_eq!(fs::read(target_root.join(auth_path)).unwrap(), bytes);
        }
    }
}
