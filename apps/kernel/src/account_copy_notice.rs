//! MP-08/MP-10/MP-11: local metadata only; provider CLIs own the credential state.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CredentialCopyNotice {
    owner_user_id: String,
    provider: String,
    profile_id: String,
    source_machine: String,
    #[serde(default)]
    notified_machines: Vec<String>,
}

fn nonempty(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

pub(crate) fn renewable_login(provider: &str, value: &serde_json::Value) -> bool {
    match provider {
        "codex" => {
            value.get("auth_mode").and_then(serde_json::Value::as_str) != Some("apikey")
                && !nonempty(value.get("OPENAI_API_KEY"))
                && nonempty(value.pointer("/tokens/refresh_token"))
        }
        "claude" => nonempty(value.pointer("/claudeAiOauth/refreshToken")),
        "opencode" => value.as_object().is_some_and(|entries| {
            entries.values().any(|auth| {
                auth.get("type").and_then(serde_json::Value::as_str) == Some("oauth")
                    && nonempty(auth.get("refresh"))
            })
        }),
        _ => false,
    }
}

/// Safe, in-memory expectations from the home-issued transfer; never serialized.
#[derive(Debug, Clone)]
pub(crate) struct ProviderAccountCopyExpectation {
    pub provider: String,
    pub source: ProviderAccountCopySource,
    pub source_account_id: String,
    pub renewable_services: Vec<String>,
    pub generated_at_ms: u64,
}

impl ProviderAccountCopyExpectation {
    pub(crate) fn from_materialization(
        materialization: &ProviderAccountMaterialization,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            provider: normalize_provider(&materialization.profile.provider)?.into(),
            source: materialization.copy_source.clone().ok_or_else(|| {
                registry_error("record account copy", "copy source identity is absent")
            })?,
            source_account_id: materialization.profile.profile_id.clone(),
            renewable_services: renewable_services(materialization),
            generated_at_ms: materialization.generated_at_ms,
        })
    }
    fn matches(&self, copy: &ProviderAccountCopyMetadata) -> bool {
        copy.source_machine_id == self.source.machine_id
            && copy.source_kernel_id == self.source.kernel_id
            && copy.source_account_id == self.source_account_id
            && copy.copied_at_ms == self.generated_at_ms
            && copy.renewable_services == self.renewable_services
    }
}

pub(crate) fn validate_copy_source_kernel(
    materialization: &ProviderAccountMaterialization,
    authenticated_kernel: &str,
) -> Result<(), DaemonError> {
    match materialization.copy_source.as_ref() {
        Some(source)
            if source.kernel_id == authenticated_kernel && !source.machine_id.trim().is_empty() =>
        {
            Ok(())
        }
        None if materialization.files.is_empty() => Ok(()), // Separate metadata-only Vault launch path.
        _ => Err(registry_error(
            "materialize account copy",
            "copy provenance does not match the authenticated source kernel",
        )),
    }
}

fn same_copy_generation(
    left: &ProviderAccountCopyMetadata,
    right: &ProviderAccountCopyMetadata,
) -> bool {
    let mut left = left.clone();
    left.auth_state = right.auth_state.clone();
    left.warning_seen = right.warning_seen;
    left == *right
}

fn renewable_services(materialization: &ProviderAccountMaterialization) -> Vec<String> {
    let mut services = BTreeSet::new();
    for file in &materialization.files {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
        else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let provider =
            crate::provider::canonical_provider_family(&materialization.profile.provider)
                .unwrap_or_default();
        if !renewable_login(provider, &value) {
            continue;
        }
        if provider == "opencode" {
            for (service, auth) in value.as_object().into_iter().flatten() {
                if auth.get("type").and_then(serde_json::Value::as_str) == Some("oauth")
                    && nonempty(auth.get("refresh"))
                {
                    services.insert(service.clone());
                }
            }
        } else {
            services.insert(provider.into());
        }
    }
    services.into_iter().collect()
}

impl ProviderAccountProfileRegistry {
    /// Consume authorized slice-import generations before any credential mutation,
    /// including requests that preserve an existing receiving login. The journal
    /// is private, credential-free local state and outlives profile removal.
    pub(crate) fn admit_slice_copy_generation(
        &self,
        owner: &str,
        materialization: &ProviderAccountMaterialization,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(&materialization.profile.provider)?;
        let source = materialization.copy_source.as_ref().ok_or_else(|| {
            registry_error("admit account copy", "copy source identity is absent")
        })?;
        let target = self.copy_identity.as_ref().ok_or_else(|| {
            registry_error("admit account copy", "receiving machine identity is absent")
        })?;
        let document = self.write_document()?;
        let matches_source = |copy: &ProviderAccountCopyMetadata| {
            copy.source_kernel_id == source.kernel_id
                && copy.source_machine_id == source.machine_id
                && copy.source_account_id == materialization.profile.profile_id
                && copy.target_kernel_id == target.kernel_id
                && copy.target_machine_id == target.machine_id
        };
        // Seed the high-water mark from existing copies/tombstones on upgrade.
        let recorded = document
            .profiles
            .iter()
            .filter(|profile| {
                profile.public.owner_user_id == owner && profile.public.provider == provider
            })
            .flat_map(|profile| {
                profile
                    .public
                    .materializations
                    .iter()
                    .filter_map(|status| status.copy.as_ref())
            })
            .chain(
                document
                    .retired_account_copies
                    .iter()
                    .filter(|entry| {
                        entry.owner_user_id == owner && entry.observation.provider == provider
                    })
                    .filter_map(|entry| entry.observation.status.copy.as_ref()),
            )
            .filter(|copy| matches_source(copy))
            .map(|copy| copy.copied_at_ms)
            .max()
            .unwrap_or(0);
        let scope = serde_json::to_vec(&(
            "slice-account-copy",
            owner,
            provider,
            &source.machine_id,
            &source.kernel_id,
            &materialization.profile.profile_id,
            &target.machine_id,
            &target.kernel_id,
        ))
        .map_err(|error| registry_error("admit account copy", error.to_string()))?;
        let directory = self.path.with_extension("copy-import-generations");
        if path_entry_exists(&directory)? {
            validate_managed_directory(&directory, "admit account copy")?;
        }
        fs::create_dir_all(&directory).map_err(registry_io("admit account copy"))?;
        set_private_dir_permissions(&directory)?;
        let path = directory.join(format!("{:x}", Sha256::digest(scope)));
        let consumed = read_bounded_regular_file_no_follow(&path, 32, "copy import generation")?
            .map(|bytes| {
                std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|text| text.trim().parse::<u64>().ok())
                    .ok_or_else(|| {
                        registry_error(
                            "admit account copy",
                            "invalid copy import generation journal",
                        )
                    })
            })
            .transpose()?
            .unwrap_or(0);
        if materialization.generated_at_ms <= recorded.max(consumed) {
            return Err(registry_error("admit account copy", "stale account copy import; request a new owner import or log in on the receiving machine"));
        }
        atomic_write_private(
            &path,
            format!("{}\n", materialization.generated_at_ms).as_bytes(),
        )
    }

    pub(crate) fn record_confirmed_account_copy(
        &self,
        owner: &str,
        expected: &ProviderAccountCopyExpectation,
        target_kind: ProviderAccountMaterializationTargetKind,
        target_machine: &str,
        target_kernel: &str,
        target_account: &str,
        mut status: ProviderAccountMaterializationStatus,
    ) -> Result<(), DaemonError> {
        let profile = self.get(owner, &expected.provider, &expected.source_account_id)?;
        let previous = profile
            .materializations
            .iter()
            .find(|entry| entry.target_kind == target_kind && entry.target_ref == target_kernel)
            .and_then(|entry| entry.copy.as_ref());
        let valid =
            if let Some((identity, copy)) = self.copy_identity.as_ref().zip(status.copy.as_ref()) {
                expected.source.machine_id == identity.machine_id
                    && expected.source.kernel_id == identity.kernel_id
                    && copy.source_machine_id == identity.machine_id
                    && copy.source_kernel_id == identity.kernel_id
                    && copy.source_account_id == expected.source_account_id
                    && copy.target_machine_id == target_machine
                    && copy.target_kernel_id == target_kernel
                    && copy.target_account_id == target_account
                    && copy.auth_state != ProviderAccountCopyAuthState::Removed
                    && status.target_ref == target_kernel
                    && status.target_kind == target_kind
                    && (expected.matches(copy)
                        || previous.is_some_and(|issued| same_copy_generation(issued, copy))
                        || self.issued_copy_matches(
                            owner,
                            expected,
                            target_kind,
                            target_machine,
                            target_kernel,
                            copy,
                        )?)
            } else {
                false
            };
        if !valid {
            return Err(registry_error("record account copy", "copy receipt does not match the home-issued account, generation and receiving placement"));
        }
        let copy = status.copy.as_mut().unwrap();
        let confirmed_generation = copy.copied_at_ms;
        copy.warning_seen = previous.is_some_and(|copy| copy.warning_seen);
        status.observed_at_ms = crate::session::unix_epoch_ms();
        status.state = if copy.auth_state == ProviderAccountCopyAuthState::Authenticated {
            ProviderAccountMaterializationState::Materialized
        } else {
            ProviderAccountMaterializationState::Stale
        };
        status.last_error = None;
        self.update_materialization_status(
            owner,
            &expected.provider,
            &expected.source_account_id,
            status,
        )?;
        self.finish_issued_account_copy(
            owner,
            expected,
            target_kind,
            target_machine,
            target_kernel,
            confirmed_generation,
        )
    }

    /// Home-confirmed provenance selects the receiving profile; the worker owns live auth.
    pub(crate) fn confirmed_remote_copy<'a>(
        &self,
        profile: &'a ProviderAccountProfile,
        kind: ProviderAccountMaterializationTargetKind,
        machine: &str,
        kernel: &str,
    ) -> Option<&'a ProviderAccountCopyMetadata> {
        let identity = self.copy_identity.as_ref()?;
        profile
            .materializations
            .iter()
            .filter(|status| {
                status.target_kind == kind
                    && status.target_ref == kernel
                    && matches!(
                        status.state,
                        ProviderAccountMaterializationState::Materialized
                            | ProviderAccountMaterializationState::Stale
                    )
            })
            .filter_map(|status| status.copy.as_ref())
            .find(|copy| {
                copy.source_machine_id == identity.machine_id
                    && copy.source_kernel_id == identity.kernel_id
                    && copy.source_account_id == profile.profile_id
                    && copy.target_machine_id == machine
                    && copy.target_kernel_id == kernel
                    && !copy.target_account_id.trim().is_empty()
                    && !copy.renewable_services.is_empty()
                    && copy.auth_state != ProviderAccountCopyAuthState::Removed
            })
    }

    pub(crate) fn apply_remote_account_copy_observation(
        &self,
        owner: &str,
        provider: &str,
        source_account: &str,
        target_kind: ProviderAccountMaterializationTargetKind,
        target_machine: &str,
        target_kernel: &str,
        observation: &ProviderAccountCopyObservation,
    ) -> Result<(), DaemonError> {
        let Some(copy) = &observation.status.copy else {
            return Ok(());
        };
        if observation.status.target_kind != target_kind
            || observation.status.target_ref != target_kernel
            || copy.target_kernel_id != target_kernel
            || copy.target_machine_id != target_machine
            || !self.copy_identity.as_ref().is_some_and(|identity| {
                copy.source_kernel_id == identity.kernel_id
                    && copy.source_machine_id == identity.machine_id
            })
        {
            return Ok(());
        }
        let provider = normalize_provider(provider)?;
        if observation.provider != provider {
            return Ok(());
        }
        let mut document = self.write_document()?;
        let Ok(profile) =
            resolve_stored_profile_mut(&mut document, owner, provider, source_account)
        else {
            return Ok(());
        };
        if copy.source_account_id != profile.public.profile_id {
            return Ok(());
        }
        let Some(issued) =
            profile.public.materializations.iter_mut().find(|entry| {
                entry.target_kind == target_kind && entry.target_ref == target_kernel
            })
        else {
            return Ok(());
        };
        let Some(issued_copy) = &mut issued.copy else {
            return Ok(());
        };
        if !same_copy_generation(issued_copy, copy) || issued_copy.auth_state == copy.auth_state {
            return Ok(());
        }
        // Only observation fields can change. Placement/provenance, warning policy
        // and generation remain the home-issued values.
        issued_copy.auth_state = copy.auth_state.clone();
        issued.observed_at_ms = crate::session::unix_epoch_ms();
        issued.state = if copy.auth_state == ProviderAccountCopyAuthState::Authenticated {
            ProviderAccountMaterializationState::Materialized
        } else {
            ProviderAccountMaterializationState::Stale
        };
        self.persist_locked(&document)
    }

    pub(crate) fn received_copy_observations(
        &self,
        owner: &str,
        source_kernel: &str,
    ) -> Result<Vec<ProviderAccountCopyObservation>, DaemonError> {
        let document = self.read_document()?;
        let mut observations = Vec::new();
        // Tombstones precede current copies; a later deliberate import supersedes removal.
        observations.extend(
            document
                .retired_account_copies
                .iter()
                .filter(|entry| entry.owner_user_id == owner)
                .map(|entry| entry.observation.clone()),
        );
        for profile in document
            .profiles
            .iter()
            .filter(|profile| profile.public.owner_user_id == owner)
        {
            observations.extend(profile.public.materializations.iter().map(|status| {
                ProviderAccountCopyObservation {
                    provider: profile.public.provider.clone(),
                    status: status.clone(),
                }
            }));
        }
        observations.retain(|entry| {
            entry.status.copy.as_ref().is_some_and(|copy| {
                copy.source_kernel_id == source_kernel
                    && self
                        .copy_identity
                        .as_ref()
                        .is_some_and(|identity| identity.kernel_id == copy.target_kernel_id)
            })
        });
        Ok(observations)
    }

    /// The profile received a renewable login copy that this kernel still holds.
    fn holds_incoming_copy(&self, profile: &ProviderAccountProfile) -> bool {
        self.copy_identity.as_ref().is_some_and(|identity| {
            profile
                .materializations
                .iter()
                .filter_map(|status| status.copy.as_ref())
                .any(|copy| {
                    copy.target_kernel_id == identity.kernel_id
                        && copy.target_account_id == profile.profile_id
                        && copy.auth_state != ProviderAccountCopyAuthState::Removed
                        && !copy.renewable_services.is_empty()
                })
        })
    }

    /// Probes the official credential path without reading credential bytes. Claude
    /// keeps the content check: portability filtering and Keychain fallback need it.
    pub(crate) fn credential_artifact_missing(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> bool {
        let Ok(provider) = normalize_provider(provider) else {
            return false;
        };
        let Ok(document) = self.read_document() else {
            return false;
        };
        let Ok(stored) = resolve_stored_profile(&document, owner, provider, profile_id) else {
            return false;
        };
        match &stored.locator {
            ProviderAccountLocator::Codex { codex_home } => !codex_home.join("auth.json").is_file(),
            ProviderAccountLocator::Opencode { xdg_data_home, .. } => {
                !xdg_data_home.join("opencode/auth.json").is_file()
            }
            locator => materialization_files(locator).is_ok_and(|files| files.is_empty()),
        }
    }

    pub(crate) fn copied_login_artifact_missing(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let profile = self.get(owner, provider, profile_id)?;
        Ok(self.holds_incoming_copy(&profile)
            && self.credential_artifact_missing(owner, provider, profile_id))
    }

    pub(crate) fn copied_login_needs_login(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        // Only a registered account profile can be a copied login; dev and default runs are not.
        if normalize_provider(provider).is_err() {
            return Ok(false);
        }
        let Some(profile) = self.find(owner, provider, profile_id)? else {
            return Ok(false);
        };
        Ok(self.holds_incoming_copy(&profile)
            && (matches!(
                profile.auth_state,
                ProviderAccountAuthState::NotConfigured | ProviderAccountAuthState::Expired
            ) || self.credential_artifact_missing(owner, provider, profile_id)))
    }

    /// Renewal-failure recovery logs out only a received copy; a local login keeps its state.
    pub(crate) fn mark_copied_login_logged_out(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<(), DaemonError> {
        let profile = self.get(owner, provider, profile_id)?;
        if self.holds_incoming_copy(&profile) {
            self.mark_logged_out(owner, provider, profile_id)?;
        }
        Ok(())
    }

    pub(crate) fn has_renewable_login(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        // A provider can remove an invalidated OAuth file before reporting the failure.
        // Retain the provenance of a copied renewable login for that recovery case.
        if let Ok(materialization) =
            self.export_materialization(owner_user_id, provider, profile_id)
        {
            for file in &materialization.files {
                let Ok(bytes) =
                    base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
                else {
                    continue;
                };
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                if renewable_login(provider, &value) {
                    return Ok(true);
                }
                if nonempty(value.get("OPENAI_API_KEY"))
                    || (provider == "opencode"
                        && value.as_object().is_some_and(|entries| {
                            entries.values().any(|auth| {
                                auth.get("type").and_then(serde_json::Value::as_str) == Some("api")
                            })
                        }))
                {
                    return Ok(false);
                }
            }
        }
        let profile = self.get(owner_user_id, provider, profile_id)?;
        if profile
            .materializations
            .iter()
            .filter_map(|status| status.copy.as_ref())
            .any(|copy| {
                copy.target_account_id == profile.profile_id
                    && self
                        .copy_identity
                        .as_ref()
                        .is_some_and(|id| id.kernel_id == copy.target_kernel_id)
                    && !copy.renewable_services.is_empty()
            })
        {
            return Ok(true);
        }
        Ok(self
            .read_document()?
            .credential_copy_notices
            .iter()
            .any(|notice| {
                notice.owner_user_id == owner_user_id
                    && notice.provider == profile.provider
                    && notice.profile_id == profile.profile_id
            }))
    }

    /// One metadata path for both sides of a committed transfer. Never stores secrets.
    pub(crate) fn record_account_copy(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
        local_profile_id: &str,
        target_kind: ProviderAccountMaterializationTargetKind,
        target_machine_id: &str,
        target_kernel_id: &str,
        target_account_id: &str,
    ) -> Result<(), DaemonError> {
        if materialization.files.is_empty() {
            return Ok(());
        }
        let source = materialization.copy_source.as_ref().ok_or_else(|| {
            registry_error(
                "record account copy",
                "provider transfer has no source machine identity",
            )
        })?;
        if [
            source.machine_id.as_str(),
            source.kernel_id.as_str(),
            target_machine_id,
            target_kernel_id,
            target_account_id,
        ]
        .iter()
        .any(|id| id.trim().is_empty())
        {
            return Err(registry_error(
                "record account copy",
                "provider transfer machine/account identity is empty",
            ));
        }
        let provider = normalize_provider(&materialization.profile.provider)?;
        let services = renewable_services(materialization);
        let profile = self.get(owner_user_id, provider, local_profile_id)?;
        let mut status = ProviderAccountMaterializationStatus {
            target_kind,
            target_ref: target_kernel_id.into(),
            state: ProviderAccountMaterializationState::Materialized,
            observed_at_ms: crate::session::unix_epoch_ms(),
            last_error: None,
            copy: Some(ProviderAccountCopyMetadata {
                source_machine_id: source.machine_id.clone(),
                source_kernel_id: source.kernel_id.clone(),
                source_account_id: materialization.profile.profile_id.clone(),
                target_machine_id: target_machine_id.into(),
                target_kernel_id: target_kernel_id.into(),
                target_account_id: target_account_id.into(),
                renewable_services: services,
                auth_state: ProviderAccountCopyAuthState::Unknown,
                copied_at_ms: materialization.generated_at_ms,
                warning_seen: false,
            }),
        };
        // Replayed imports do not reset warning delivery or a receiver's newer login/logout.
        if let Some(previous) = profile
            .materializations
            .iter()
            .find(|entry| entry.target_ref == target_kernel_id)
            .and_then(|entry| entry.copy.as_ref())
        {
            if previous.source_machine_id == source.machine_id
                && previous.source_account_id == materialization.profile.profile_id
                && previous.target_account_id == target_account_id
                && previous.copied_at_ms == materialization.generated_at_ms
            {
                return Ok(());
            }
        }
        if self
            .copy_identity
            .as_ref()
            .is_some_and(|id| id.kernel_id == target_kernel_id)
        {
            status.copy.as_mut().unwrap().auth_state = copy_auth_state(profile.auth_state);
        }
        self.update_materialization_status(owner_user_id, provider, local_profile_id, status)?;
        Ok(())
    }

    pub(crate) fn record_received_account_copy(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
        target_profile_id: &str,
        target_kind: ProviderAccountMaterializationTargetKind,
    ) -> Result<(), DaemonError> {
        let target = self.copy_identity.as_ref().ok_or_else(|| {
            registry_error(
                "record account copy",
                "receiving kernel has no machine identity",
            )
        })?;
        self.record_account_copy(
            owner_user_id,
            materialization,
            target_profile_id,
            target_kind,
            &target.machine_id,
            &target.kernel_id,
            target_profile_id,
        )
    }

    pub(crate) fn observe_target_copy(
        &self,
        owner_user_id: &str,
        provider: &str,
        target_kernel_id: &str,
        target_account_id: &str,
        state: ProviderAccountCopyAuthState,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let mut changed = false;
        for profile in &mut document.profiles {
            if profile.public.owner_user_id != owner_user_id || profile.public.provider != provider
            {
                continue;
            }
            for status in &mut profile.public.materializations {
                if let Some(copy) = &mut status.copy {
                    // Provider activity reports the same state repeatedly; record transitions only.
                    if copy.target_kernel_id == target_kernel_id
                        && copy.target_account_id == target_account_id
                        && copy.auth_state != state
                    {
                        changed = true;
                        copy.auth_state = state.clone();
                        status.observed_at_ms = crate::session::unix_epoch_ms();
                        status.state = if state == ProviderAccountCopyAuthState::Authenticated {
                            ProviderAccountMaterializationState::Materialized
                        } else {
                            ProviderAccountMaterializationState::Stale
                        };
                    }
                }
            }
        }
        if changed {
            self.persist_locked(&document)?;
        }
        Ok(())
    }

    pub(crate) fn take_credential_copy_notice(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        machine: &str,
    ) -> Result<Option<String>, DaemonError> {
        let profile = self.get(owner_user_id, provider, profile_id)?;
        let mut document = self.write_document()?;
        let original = document.clone();
        let profile_index = resolved_profile_index(
            &document,
            owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )?;
        if let Some(copy) = document.profiles[profile_index]
            .public
            .materializations
            .iter_mut()
            .filter_map(|status| status.copy.as_mut())
            .find(|copy| {
                copy.target_machine_id == machine
                    && copy.target_account_id == profile.profile_id
                    && !copy.renewable_services.is_empty()
                    && !copy.warning_seen
            })
        {
            let message = format!("{} login on {machine} was copied from {}. This copy can log you out. Consider logging in on this machine using the provider's official login.", profile.provider, copy.source_machine_id);
            copy.warning_seen = true;
            if let Err(error) = self.persist_locked(&document) {
                *document = original;
                return Err(error);
            }
            return Ok(Some(message));
        }
        let Some(notice) = document.credential_copy_notices.iter_mut().find(|notice| {
            notice.owner_user_id == owner_user_id
                && notice.provider == profile.provider
                && notice.profile_id == profile.profile_id
                && !notice.notified_machines.iter().any(|id| id == machine)
        }) else {
            return Ok(None);
        };
        let provider = match profile.provider.as_str() {
            "codex" => "Codex",
            "claude" => "Claude",
            "opencode" => "OpenCode",
            other => other,
        };
        let message = format!("{provider} credentials on {machine} were copied from {}. The provider may invalidate one copy when another refreshes; you may need to log in on this machine later.", notice.source_machine);
        notice.notified_machines.push(machine.into());
        if let Err(error) = self.persist_locked(&document) {
            *document = original;
            return Err(error);
        }
        Ok(Some(message))
    }
}

pub(super) fn copy_auth_state(state: ProviderAccountAuthState) -> ProviderAccountCopyAuthState {
    match state {
        ProviderAccountAuthState::Authenticated => ProviderAccountCopyAuthState::Authenticated,
        ProviderAccountAuthState::Expired | ProviderAccountAuthState::NotConfigured => {
            ProviderAccountCopyAuthState::NeedsLogin
        }
        ProviderAccountAuthState::Unknown | ProviderAccountAuthState::Error => {
            ProviderAccountCopyAuthState::Unknown
        }
    }
}

pub(super) fn observe_received_copies(
    profile: &mut ProviderAccountProfile,
    state: ProviderAccountAuthState,
    identity: Option<&ProviderAccountCopySource>,
) {
    for status in &mut profile.materializations {
        if let Some(copy) = &mut status.copy {
            if copy.target_account_id == profile.profile_id
                && identity.is_some_and(|id| id.kernel_id == copy.target_kernel_id)
            {
                copy.auth_state = copy_auth_state(state);
                status.observed_at_ms = crate::session::unix_epoch_ms();
                status.state = if state == ProviderAccountAuthState::Authenticated {
                    ProviderAccountMaterializationState::Materialized
                } else {
                    ProviderAccountMaterializationState::Stale
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrev_f3_copy_observations_cannot_retarget_or_replace_issued_inventory() {
        let root = crate::test_support::TestWorktree::new("copy-observation-authority");
        let registry = ProviderAccountProfileRegistry::open(root.path().join("registry.json"))
            .unwrap()
            .with_machine_identity("home-machine", "home-kernel");
        let source = registry.create_managed("owner", "codex", "Source").unwrap();
        let other = registry.create_managed("owner", "codex", "Other").unwrap();
        let issued = ProviderAccountMaterializationStatus {
            target_kind: ProviderAccountMaterializationTargetKind::Worker,
            target_ref: "worker-a".into(),
            state: ProviderAccountMaterializationState::Materialized,
            observed_at_ms: 1,
            last_error: None,
            copy: Some(ProviderAccountCopyMetadata {
                source_machine_id: "home-machine".into(),
                source_kernel_id: "home-kernel".into(),
                source_account_id: source.profile_id.clone(),
                target_machine_id: "machine-a".into(),
                target_kernel_id: "worker-a".into(),
                target_account_id: "receiver-account".into(),
                renewable_services: vec!["codex".into()],
                auth_state: ProviderAccountCopyAuthState::Authenticated,
                copied_at_ms: 1,
                warning_seen: false,
            }),
        };
        registry
            .update_materialization_status("owner", "codex", &source.profile_id, issued.clone())
            .unwrap();
        for attack in 0..6 {
            let mut status = issued.clone();
            status.copy.as_mut().unwrap().auth_state = ProviderAccountCopyAuthState::NeedsLogin;
            match attack {
                0 => status.target_ref = "worker-b".into(),
                1 => status.target_kind = ProviderAccountMaterializationTargetKind::Slice,
                2 => status.copy.as_mut().unwrap().source_account_id = other.profile_id.clone(),
                3 => status.copy.as_mut().unwrap().copied_at_ms = 2,
                4 => status.copy.as_mut().unwrap().renewable_services = vec!["attacker".into()],
                _ => status.copy.as_mut().unwrap().target_account_id = "other-receiver".into(),
            }
            let expected = ProviderAccountCopyExpectation {
                provider: "codex".into(),
                source: ProviderAccountCopySource {
                    machine_id: "home-machine".into(),
                    kernel_id: "home-kernel".into(),
                },
                source_account_id: source.profile_id.clone(),
                renewable_services: vec!["codex".into()],
                generated_at_ms: 1,
            };
            assert!(
                registry
                    .record_confirmed_account_copy(
                        "owner",
                        &expected,
                        ProviderAccountMaterializationTargetKind::Worker,
                        "machine-a",
                        "worker-a",
                        "receiver-account",
                        status.clone()
                    )
                    .is_err(),
                "forged receipt {attack} was accepted"
            );
            registry
                .apply_remote_account_copy_observation(
                    "owner",
                    "codex",
                    &source.profile_id,
                    ProviderAccountMaterializationTargetKind::Worker,
                    "machine-a",
                    "worker-a",
                    &ProviderAccountCopyObservation {
                        provider: "codex".into(),
                        status,
                    },
                )
                .unwrap();
            assert_eq!(
                registry
                    .get("owner", "codex", &source.profile_id)
                    .unwrap()
                    .materializations,
                vec![issued.clone()],
                "attack {attack} changed the issued copy"
            );
            assert!(registry
                .get("owner", "codex", &other.profile_id)
                .unwrap()
                .materializations
                .is_empty());
        }
        let mut observed = issued.clone();
        observed.copy.as_mut().unwrap().auth_state = ProviderAccountCopyAuthState::NeedsLogin;
        registry
            .apply_remote_account_copy_observation(
                "owner",
                "codex",
                &source.profile_id,
                ProviderAccountMaterializationTargetKind::Worker,
                "machine-a",
                "worker-a",
                &ProviderAccountCopyObservation {
                    provider: "codex".into(),
                    status: observed,
                },
            )
            .unwrap();
        assert_eq!(
            registry
                .get("owner", "codex", &source.profile_id)
                .unwrap()
                .materializations[0]
                .copy
                .as_ref()
                .unwrap()
                .auth_state,
            ProviderAccountCopyAuthState::NeedsLogin
        );
    }

    #[test]
    fn mp08_mp10_mp11_only_renewable_logins_get_copy_notices() {
        assert!(renewable_login(
            "codex",
            &serde_json::json!({"tokens":{"refresh_token":"synthetic"}})
        ));
        assert!(!renewable_login(
            "codex",
            &serde_json::json!({"OPENAI_API_KEY":"synthetic","tokens":{"refresh_token":"synthetic"}})
        ));
        assert!(renewable_login(
            "claude",
            &serde_json::json!({"claudeAiOauth":{"refreshToken":"synthetic"}})
        ));
        assert!(!renewable_login(
            "claude",
            &serde_json::json!({"setupToken":"synthetic"})
        ));
        assert!(renewable_login(
            "opencode",
            &serde_json::json!({"openai":{"type":"oauth","refresh":"synthetic"}})
        ));
        assert!(!renewable_login(
            "opencode",
            &serde_json::json!({"openai":{"type":"api","key":"synthetic"}})
        ));
        assert!(renewable_login(
            "opencode",
            &serde_json::json!({"anthropic":{"type":"oauth","refresh":"synthetic"}})
        ));
        assert!(renewable_login(
            "opencode",
            &serde_json::json!({"openai":{"type":"api","key":"synthetic"},"other":{"type":"oauth","refresh":"synthetic"}})
        ));
        assert!(!renewable_login(
            "opencode",
            &serde_json::json!({"other":{"type":"oauth","refresh":" "}})
        ));
    }

    #[test]
    fn mp08_mp10_mp11_unregistered_or_unsupported_profile_is_not_a_copied_login() {
        let root =
            std::env::temp_dir().join(format!("chariox-copy-notice-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let registry = ProviderAccountProfileRegistry::open(&root.join("registry.json"))
            .unwrap()
            .with_machine_identity("worker-machine", "worker-kernel");
        assert!(!registry
            .copied_login_needs_login("owner", "dev-stub", "default")
            .unwrap());
        assert!(!registry
            .copied_login_needs_login("owner", "codex", "default")
            .unwrap());
        drop(registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp08_mp10_mp11_renewal_failure_marks_only_received_copies_logged_out() {
        let root =
            std::env::temp_dir().join(format!("chariox-copy-renewal-{}", rand::random::<u64>()));
        let registry = ProviderAccountProfileRegistry::open(&root.join("registry.json"))
            .unwrap()
            .with_machine_identity("worker-machine", "worker-kernel");
        let local = registry.create_managed("owner", "codex", "Local").unwrap();
        let env = registry
            .resolve_environment("owner", "codex", &local.profile_id)
            .unwrap();
        fs::write(
            Path::new(&env["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"refresh_token":"synthetic-local"}}"#,
        )
        .unwrap();
        let authenticate = |profile_id: &str| {
            registry
                .update_observation(
                    "owner",
                    "codex",
                    profile_id,
                    ProviderAccountAuthState::Authenticated,
                    Some(format!("synthetic identity {profile_id}")),
                    None,
                    None,
                    None,
                )
                .unwrap();
        };
        authenticate(&local.profile_id);
        assert!(registry
            .has_renewable_login("owner", "codex", &local.profile_id)
            .unwrap());
        // A locally linked login keeps its last observed state until the user logs in again.
        registry
            .mark_copied_login_logged_out("owner", "codex", &local.profile_id)
            .unwrap();
        let local = registry.get("owner", "codex", &local.profile_id).unwrap();
        assert_eq!(local.auth_state, ProviderAccountAuthState::Authenticated);
        assert_eq!(
            local.identity_summary,
            Some(format!("synthetic identity {}", local.profile_id))
        );

        // A local OpenCode login keeps all last-observed services and metadata.
        let opencode = registry
            .create_managed("owner", "opencode", "Local services")
            .unwrap();
        registry
            .update_observation(
                "owner",
                "opencode",
                &opencode.profile_id,
                ProviderAccountAuthState::Authenticated,
                Some("local identity".into()),
                Some("local plan".into()),
                None,
                None,
            )
            .unwrap();
        registry
            .update_services(
                "owner",
                "opencode",
                &opencode.profile_id,
                ["openai", "anthropic"]
                    .into_iter()
                    .map(|service| ProviderAccountService {
                        service_id: service.into(),
                        label: service.into(),
                        auth_state: ProviderAccountAuthState::Authenticated,
                        credential_type: ProviderAccountServiceCredentialType::Oauth,
                        billing_kind: None,
                    })
                    .collect(),
            )
            .unwrap();
        let before = serde_json::to_value(
            registry
                .get("owner", "opencode", &opencode.profile_id)
                .unwrap(),
        )
        .unwrap();
        registry
            .mark_copied_login_logged_out("owner", "opencode", &opencode.profile_id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(
                registry
                    .get("owner", "opencode", &opencode.profile_id)
                    .unwrap()
            )
            .unwrap(),
            before
        );

        let materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource {
                machine_id: "source-machine".into(),
                kernel_id: "source-kernel".into(),
            }),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "codex".into(),
                profile_id: "copied".into(),
                label: "Copy".into(),
                origin: ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"tokens":{"refresh_token":"synthetic"}}"#),
            }],
            generated_at_ms: 1,
        };
        let copied = registry
            .materialize_replica("owner", &materialization)
            .unwrap();
        registry
            .record_received_account_copy(
                "owner",
                &materialization,
                &copied.profile_id,
                ProviderAccountMaterializationTargetKind::Worker,
            )
            .unwrap();
        authenticate(&copied.profile_id);
        // A different kernel does not hold this incoming copy.
        registry
            .clone()
            .with_machine_identity("other-machine", "other-kernel")
            .mark_copied_login_logged_out("owner", "codex", &copied.profile_id)
            .unwrap();
        assert_eq!(
            registry
                .get("owner", "codex", &copied.profile_id)
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Authenticated
        );
        registry
            .mark_copied_login_logged_out("owner", "codex", &copied.profile_id)
            .unwrap();
        assert_eq!(
            registry
                .get("owner", "codex", &copied.profile_id)
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::NotConfigured
        );
        drop(registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mp08_mp10_mp11_unchanged_copy_observation_does_not_rewrite_registry() {
        use std::os::unix::fs::MetadataExt;
        let root =
            std::env::temp_dir().join(format!("chariox-copy-observe-{}", rand::random::<u64>()));
        let path = root.join("registry.json");
        let registry = ProviderAccountProfileRegistry::open(&path).unwrap();
        let profile = registry.create_managed("owner", "codex", "Source").unwrap();
        let status = ProviderAccountMaterializationStatus {
            copy: Some(ProviderAccountCopyMetadata {
                source_machine_id: "home-machine".into(),
                source_kernel_id: "home-kernel".into(),
                source_account_id: profile.profile_id.clone(),
                target_machine_id: "worker-machine".into(),
                target_kernel_id: "worker-kernel".into(),
                target_account_id: "worker-account".into(),
                renewable_services: vec!["codex".into()],
                auth_state: ProviderAccountCopyAuthState::NeedsLogin,
                copied_at_ms: 1,
                warning_seen: false,
            }),
            target_kind: ProviderAccountMaterializationTargetKind::Worker,
            target_ref: "worker-kernel".into(),
            state: ProviderAccountMaterializationState::Stale,
            observed_at_ms: 2,
            last_error: None,
        };
        let inode = || fs::metadata(&path).unwrap().ino();
        registry
            .update_materialization_status("owner", "codex", &profile.profile_id, status.clone())
            .unwrap();
        let written = inode();
        // Leased projections repeat the same observations on every event.
        registry
            .update_materialization_status("owner", "codex", &profile.profile_id, status)
            .unwrap();
        registry
            .observe_target_copy(
                "owner",
                "codex",
                "worker-kernel",
                "worker-account",
                ProviderAccountCopyAuthState::NeedsLogin,
            )
            .unwrap();
        assert_eq!(inode(), written);
        registry
            .observe_target_copy(
                "owner",
                "codex",
                "worker-kernel",
                "worker-account",
                ProviderAccountCopyAuthState::Authenticated,
            )
            .unwrap();
        assert_ne!(inode(), written);
        let copy = registry
            .get("owner", "codex", &profile.profile_id)
            .unwrap()
            .materializations[0]
            .copy
            .clone()
            .unwrap();
        assert_eq!(copy.auth_state, ProviderAccountCopyAuthState::Authenticated);
        drop(registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp08_mp10_mp11_reimport_supersedes_removal_tombstones() {
        let root =
            std::env::temp_dir().join(format!("chariox-copy-tombstone-{}", rand::random::<u64>()));
        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json"))
            .unwrap()
            .with_machine_identity("worker-machine", "worker-kernel");
        let import = |label: &str| {
            let profile = registry.create_managed("owner", "codex", label).unwrap();
            let materialization = ProviderAccountMaterialization {
                copy_source: Some(ProviderAccountCopySource {
                    machine_id: "source-machine".into(),
                    kernel_id: "source-kernel".into(),
                }),
                profile: ProviderAccountReplicaMetadata {
                    owner_user_id: "owner".into(),
                    provider: "codex".into(),
                    profile_id: "source-account".into(),
                    label: "Source".into(),
                    origin: profile.origin,
                    is_default: false,
                },
                files: vec![ProviderAccountMaterializationFile {
                    relative_path: "auth.json".into(),
                    contents_base64: base64::engine::general_purpose::STANDARD
                        .encode(br#"{"tokens":{"refresh_token":"synthetic"}}"#),
                }],
                generated_at_ms: 1,
            };
            registry
                .record_received_account_copy(
                    "owner",
                    &materialization,
                    &profile.profile_id,
                    ProviderAccountMaterializationTargetKind::Worker,
                )
                .unwrap();
            profile.profile_id
        };
        let removed_states = || {
            registry
                .received_copy_observations("owner", "source-kernel")
                .unwrap()
                .into_iter()
                .map(|entry| entry.status.copy.unwrap().auth_state)
                .collect::<Vec<_>>()
        };
        let first = import("First");
        registry
            .remove_registration("owner", "codex", &first)
            .unwrap();
        let second = import("Second");
        registry
            .remove_registration("owner", "codex", &second)
            .unwrap();
        assert_eq!(
            removed_states(),
            vec![ProviderAccountCopyAuthState::Removed]
        );
        import("Third");
        assert!(removed_states()
            .iter()
            .all(|state| *state != ProviderAccountCopyAuthState::Removed));
        drop(registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp08_mp10_mp11_notice_is_once_per_account_machine_across_restart() {
        let root =
            std::env::temp_dir().join(format!("chariox-copy-notice-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("registry.json");
        let registry = ProviderAccountProfileRegistry::open(&path)
            .unwrap()
            .with_machine_identity("worker-machine", "worker-kernel");
        let profile = registry
            .create_managed("owner", "codex", "Synthetic")
            .unwrap();
        let materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource {
                machine_id: "source-machine".into(),
                kernel_id: "source-kernel".into(),
            }),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "codex".into(),
                profile_id: profile.profile_id.clone(),
                label: "Synthetic".into(),
                origin: profile.origin,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"tokens":{"refresh_token":"synthetic"}}"#),
            }],
            generated_at_ms: 1,
        };
        registry
            .record_received_account_copy(
                "owner",
                &materialization,
                &profile.profile_id,
                ProviderAccountMaterializationTargetKind::Worker,
            )
            .unwrap();
        let message = registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "worker-machine")
            .unwrap()
            .unwrap();
        assert!(message.contains("This copy can log you out"));
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "worker-machine")
            .unwrap()
            .is_none());
        drop(registry);
        let registry = ProviderAccountProfileRegistry::open(&path)
            .unwrap()
            .with_machine_identity("worker-machine", "worker-kernel");
        registry
            .record_received_account_copy(
                "owner",
                &materialization,
                &profile.profile_id,
                ProviderAccountMaterializationTargetKind::Worker,
            )
            .unwrap();
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "worker-machine")
            .unwrap()
            .is_none());
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &profile.profile_id, "another-machine")
            .unwrap()
            .is_none());
        assert!(!String::from_utf8(fs::read(&path).unwrap())
            .unwrap()
            .contains("refresh_token"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn remapped_default_copy_survives_credential_removal_and_scopes_login_observations() {
        let root =
            std::env::temp_dir().join(format!("chariox-copy-remap-{}", rand::random::<u64>()));
        let registry = ProviderAccountProfileRegistry::open(root.join("registry.json"))
            .unwrap()
            .with_machine_identity("target-machine", "target-kernel");
        let target = registry
            .create_managed("owner", "codex", "Target default")
            .unwrap();
        registry
            .set_default("owner", "codex", &target.profile_id)
            .unwrap();
        let managed = registry
            .resolve_environment("owner", "codex", &target.profile_id)
            .unwrap();
        // Model an untouched native default, not an existing managed credential publication.
        fs::remove_dir_all(Path::new(&managed["CODEX_HOME"]).parent().unwrap()).unwrap();
        {
            let mut document = registry.write_document().unwrap();
            let stored = document
                .profiles
                .iter_mut()
                .find(|p| p.public.profile_id == target.profile_id)
                .unwrap();
            stored.public.origin = ProviderAccountProfileOrigin::Default;
            stored.locator = ProviderAccountLocator::Codex {
                codex_home: root.join("native/codex"),
            };
            registry.persist_locked(&document).unwrap();
        }
        let materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource {
                machine_id: "source-machine".into(),
                kernel_id: "source-kernel".into(),
            }),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "codex".into(),
                profile_id: "source-default".into(),
                label: "Source".into(),
                origin: ProviderAccountProfileOrigin::Default,
                is_default: true,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"tokens":{"refresh_token":"synthetic"}}"#),
            }],
            generated_at_ms: 1,
        };
        let receipt = registry
            .materialize_managed_context_replica(
                "owner",
                "context",
                &"a".repeat(64),
                &materialization,
            )
            .unwrap();
        assert_eq!(receipt.profile_id, target.profile_id);
        registry
            .record_received_account_copy(
                "owner",
                &materialization,
                &receipt.profile_id,
                ProviderAccountMaterializationTargetKind::Worker,
            )
            .unwrap();
        let env = registry
            .resolve_environment("owner", "codex", &receipt.profile_id)
            .unwrap();
        fs::remove_file(Path::new(&env["CODEX_HOME"]).join("auth.json")).unwrap();
        registry
            .mark_logged_out("owner", "codex", &receipt.profile_id)
            .unwrap();
        assert!(registry
            .has_renewable_login("owner", "codex", &receipt.profile_id)
            .unwrap());
        assert!(registry
            .copied_login_needs_login("owner", "codex", &receipt.profile_id)
            .unwrap());
        let copied = registry.get("owner", "codex", &receipt.profile_id).unwrap();
        let metadata = copied.materializations[0].copy.as_ref().unwrap();
        assert_eq!(metadata.source_account_id, "source-default");
        assert_eq!(metadata.target_account_id, target.profile_id);
        assert_eq!(
            metadata.auth_state,
            ProviderAccountCopyAuthState::NeedsLogin
        );
        assert!(registry
            .take_credential_copy_notice("owner", "codex", &receipt.profile_id, "target-machine")
            .unwrap()
            .is_some());
        // A successful official login publishes a new receiving artifact; changing
        // cached app-server observation alone must not hide the missing-file logout.
        fs::write(
            Path::new(&env["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"refresh_token":"synthetic-new-login"}}"#,
        )
        .unwrap();
        registry
            .update_observation(
                "owner",
                "codex",
                &receipt.profile_id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(!registry
            .copied_login_needs_login("owner", "codex", &receipt.profile_id)
            .unwrap());
        drop(registry);
        fs::remove_dir_all(root).unwrap();
    }
}

pub(super) fn retire_received_copies(
    document: &mut RegistryDocument,
    profile: &ProviderAccountProfile,
    identity: Option<&ProviderAccountCopySource>,
) {
    for mut status in profile.materializations.clone() {
        if let Some(copy) = &mut status.copy {
            if identity.is_some_and(|identity| identity.kernel_id == copy.target_kernel_id) {
                prune_superseded_tombstones(
                    document,
                    &profile.owner_user_id,
                    &profile.provider,
                    copy,
                );
                copy.auth_state = ProviderAccountCopyAuthState::Removed;
                status.state = ProviderAccountMaterializationState::Stale;
                status.observed_at_ms = crate::session::unix_epoch_ms();
                document
                    .retired_account_copies
                    .push(RetiredProviderAccountCopy {
                        owner_user_id: profile.owner_user_id.clone(),
                        observation: ProviderAccountCopyObservation {
                            provider: profile.provider.clone(),
                            status,
                        },
                    });
            }
        }
    }
}

/// One tombstone per copy slot: a newer removal or a current copy supersedes it.
/// The home keys copies by source account and receiving kernel.
pub(super) fn prune_superseded_tombstones(
    document: &mut RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    copy: &ProviderAccountCopyMetadata,
) -> bool {
    let before = document.retired_account_copies.len();
    document.retired_account_copies.retain(|entry| {
        !(entry.owner_user_id == owner_user_id
            && entry.observation.provider == provider
            && entry
                .observation
                .status
                .copy
                .as_ref()
                .is_some_and(|retired| {
                    retired.source_kernel_id == copy.source_kernel_id
                        && retired.source_account_id == copy.source_account_id
                        && retired.target_kernel_id == copy.target_kernel_id
                }))
    });
    document.retired_account_copies.len() != before
}
