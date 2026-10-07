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

impl ProviderAccountProfileRegistry {
    pub(crate) fn record_confirmed_account_copy(
        &self,
        owner: &str,
        provider: &str,
        source_account: &str,
        target_machine: &str,
        target_kernel: &str,
        target_account: &str,
        status: ProviderAccountMaterializationStatus,
    ) -> Result<(), DaemonError> {
        let valid = self
            .copy_identity
            .as_ref()
            .zip(status.copy.as_ref())
            .is_some_and(|(identity, copy)| {
                copy.source_machine_id == identity.machine_id
                    && copy.source_kernel_id == identity.kernel_id
                    && copy.source_account_id == source_account
                    && copy.target_machine_id == target_machine
                    && copy.target_kernel_id == target_kernel
                    && copy.target_account_id == target_account
                    && status.target_ref == target_kernel
            });
        if !valid {
            return Err(DaemonError::LocalTransport {
                operation: "record account copy",
                message: "copy receipt does not match the source and receiving account identities"
                    .into(),
            });
        }
        self.update_materialization_status(owner, provider, source_account, status)?;
        Ok(())
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

    pub(crate) fn copied_login_artifact_missing(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let profile = self.get(owner, provider, profile_id)?;
        let incoming = self.copy_identity.as_ref().is_some_and(|identity| {
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
        });
        Ok(incoming
            && self
                .export_materialization(owner, provider, profile_id)
                .is_ok_and(|materialization| materialization.files.is_empty()))
    }

    pub(crate) fn copied_login_needs_login(
        &self,
        owner: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let profile = self.get(owner, provider, profile_id)?;
        Ok(
            (self.copied_login_artifact_missing(owner, provider, profile_id)?
                || matches!(
                    profile.auth_state,
                    ProviderAccountAuthState::NotConfigured | ProviderAccountAuthState::Expired
                ))
                && self.copy_identity.as_ref().is_some_and(|identity| {
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
                }),
        )
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
        let mut services = BTreeSet::new();
        for file in &materialization.files {
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&file.contents_base64)
            else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
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
                renewable_services: services.into_iter().collect(),
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
        for profile in &mut document.profiles {
            if profile.public.owner_user_id != owner_user_id || profile.public.provider != provider
            {
                continue;
            }
            for status in &mut profile.public.materializations {
                if let Some(copy) = &mut status.copy {
                    if copy.target_kernel_id == target_kernel_id
                        && copy.target_account_id == target_account_id
                    {
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
        self.persist_locked(&document)
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
