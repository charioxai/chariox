use std::path::PathBuf;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunState;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_account_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner_user_id = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    let invalidates_catalog = !matches!(
        request,
        LocalDaemonRequest::ListProviderAccountProfiles(_)
            | LocalDaemonRequest::GetProviderAccountProfile(_)
    );
    let registry = runtime_state.provider_account_profile_registry().clone();
    let runtime_for_checks = runtime_state.clone();
    let response = tokio::task::spawn_blocking(move || match request {
        LocalDaemonRequest::ListProviderAccountProfiles(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfilesListed {
                profiles: registry.list(&owner_user_id, request.provider.as_deref())?,
            })
        }
        LocalDaemonRequest::GetProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.get(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::CreateProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.create_managed(
                    &owner_user_id,
                    &request.provider,
                    &request.label,
                )?,
            })
        }
        LocalDaemonRequest::LinkProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.link_existing(
                    &owner_user_id,
                    &request.provider,
                    &request.label,
                    &PathBuf::from(request.path),
                )?,
            })
        }
        LocalDaemonRequest::ImportNativeProviderAccountProfile(request) => {
            let native_owner = runtime_for_checks
                .provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
            if owner_user_id != native_owner {
                return Err(DaemonError::LocalTransport {
                    operation: "import native account profile",
                    message:
                        "only the provider-account authority owner may import a host-native account"
                            .to_string(),
                });
            }
            let home = std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "import native account profile",
                    message: "the kernel host HOME must be an absolute path".to_string(),
                })?;
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.import_native_default(
                    &owner_user_id,
                    &request.provider,
                    &home,
                )?,
            })
        }
        LocalDaemonRequest::RenameProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.rename(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    &request.label,
                )?,
            })
        }
        LocalDaemonRequest::SetDefaultProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile: registry.set_default(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::RefreshProviderAccountProfile(request) => {
            Ok(LocalDaemonResponse::ProviderAccountProfile {
                profile:
                    crate::local::provider_requests::refresh_provider_account_profile_response(
                        &registry,
                        &owner_user_id,
                        &request.provider,
                        &request.account_profile,
                    )?,
            })
        }
        LocalDaemonRequest::RemoveProviderAccountProfile(request) => {
            ensure_profile_idle(
                &runtime_for_checks,
                &registry,
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            invalidate_profile_endpoint(
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            );
            Ok(LocalDaemonResponse::ProviderAccountProfileRemoved {
                profile: registry.remove_registration(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                )?,
            })
        }
        LocalDaemonRequest::DeleteProviderAccountProfileData(request) => {
            ensure_profile_idle(
                &runtime_for_checks,
                &registry,
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            invalidate_profile_endpoint(
                &owner_user_id,
                &request.provider,
                &request.account_profile,
            );
            Ok(LocalDaemonResponse::ProviderAccountProfileDataDeleted {
                profile: registry.delete_managed_profile_data(
                    &owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    &request.confirmation_profile_id,
                )?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider account request",
            message: "unsupported provider account request".to_string(),
        }),
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "provider account request",
        message: error.to_string(),
    })??;
    if invalidates_catalog {
        runtime_state
            .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
            .await;
        runtime_state.record_waiting_room_change();
    }
    Ok(response)
}

fn invalidate_profile_endpoint(owner_user_id: &str, provider: &str, account_profile: &str) {
    match crate::provider::canonical_provider_family(provider) {
        Some("codex") => {
            crate::provider::invalidate_codex_account_endpoint(owner_user_id, account_profile)
        }
        Some("opencode") => {
            crate::provider::invalidate_opencode_account_endpoint(owner_user_id, account_profile)
        }
        _ => {}
    }
}

pub(crate) fn ensure_profile_idle(
    runtime_state: &KernelRuntimeState,
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<(), DaemonError> {
    let profile = registry.get(owner_user_id, provider, profile_id)?;
    let bound_agents = bound_agent_labels(
        &runtime_state.list_session_snapshots(),
        &profile.provider,
        |agent_owner_user_id| {
            runtime_state.provider_account_authority_owner_user_id(agent_owner_user_id)
                == owner_user_id
        },
        |account_profile| {
            account_profile_reference_matches(
                registry,
                owner_user_id,
                &profile.provider,
                account_profile,
                &profile.profile_id,
            )
        },
    );
    if !bound_agents.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` is assigned to agent(s) {}; choose another account for those agents before removing or deleting it",
                profile.label,
                bound_agents.join(", "),
            ),
        });
    }
    let active = runtime_state
        .provider_runs_for_external_session_attachment()
        .into_iter()
        .any(|run| {
            let run_owner_user_id =
                runtime_state.provider_account_authority_owner_user_id(run.owner_user_id());
            run_owner_user_id == owner_user_id
                && crate::provider::canonical_provider_family(run.provider())
                    == crate::provider::canonical_provider_family(provider)
                && account_profile_reference_matches(
                    registry,
                    &run_owner_user_id,
                    &profile.provider,
                    run.account_profile(),
                    &profile.profile_id,
                )
                && run.state() != ProviderRunState::Ended
        });
    if active {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` has an active provider run; end the run before removing or deleting it",
                profile.label
            ),
        });
    }
    if runtime_state
        .provider_login_process_store()
        .has_running_for_profile(owner_user_id, &profile.provider, &profile.profile_id)
    {
        return Err(DaemonError::LocalTransport {
            operation: "mutate provider account profile",
            message: format!(
                "account profile `{}` has an active provider authentication workflow; cancel it before removing or deleting the profile",
                profile.label
            ),
        });
    }
    Ok(())
}

fn bound_agent_labels(
    sessions: &[crate::session::RuntimeSession],
    provider: &str,
    owner_matches: impl Fn(&str) -> bool,
    account_profile_matches: impl Fn(&str) -> bool,
) -> Vec<String> {
    let provider_family = crate::provider::canonical_provider_family(provider);
    let mut labels = sessions
        .iter()
        .flat_map(|session| session.agents())
        .filter(|agent| {
            owner_matches(agent.owner_user_id())
                && agent_provider_account_references(agent).any(|(provider, account_profile)| {
                    crate::provider::canonical_provider_family(provider) == provider_family
                        && account_profile_matches(account_profile)
                })
        })
        .map(|agent| agent.alias().unwrap_or(agent.id()).to_string())
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels
}

fn agent_provider_account_references(
    agent: &crate::agent::AgentInstance,
) -> impl Iterator<Item = (&str, &str)> {
    std::iter::once((agent.provider(), agent.provider_account_profile())).chain(
        agent.substitutes().iter().map(|profile| {
            (
                profile.provider.as_str(),
                profile.account_profile.as_deref().unwrap_or("default"),
            )
        }),
    )
}

fn account_profile_reference_matches(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
    target_profile_id: &str,
) -> bool {
    registry
        .get(owner_user_id, provider, account_profile)
        .is_ok_and(|profile| profile.profile_id == target_profile_id)
}

#[cfg(test)]
mod tests {
    use super::{account_profile_reference_matches, bound_agent_labels};
    use crate::agent::{AgentInstance, AgentSubstituteProfile, GridPosition};
    use crate::session::RuntimeSession;

    #[cfg(unix)]
    #[test]
    fn mp08_mp10_mp11_slice_reimport_preserves_authenticated_receiving_login() {
        if crate::test_support::isolate_environment_test() {
            return;
        }
        use crate::account_profile::*;
        use base64::Engine;
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_support::TestWorktree::new("credcopies-preserve");
        let executable = root.path().join("opencode");
        std::fs::write(&executable, "#!/bin/sh\nprintf 'fixture-opencode\\n'\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("CHARIOX_OPENCODE_BIN", executable);
        let registry = ProviderAccountProfileRegistry::open(root.path().join("accounts.json"))
            .unwrap()
            .with_machine_identity("slice-machine", "slice-kernel");
        let profile = registry
            .create_managed("owner", "opencode", "Synthetic receiver")
            .unwrap();
        let environment = registry
            .resolve_environment("owner", "opencode", &profile.profile_id)
            .unwrap();
        let auth_path =
            std::path::Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        let receiving_login = br#"{"openai":{"type":"api","key":"synthetic-receiver"}}"#;
        std::fs::write(&auth_path, receiving_login).unwrap();
        let mut materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource {
                machine_id: "source-machine".into(),
                kernel_id: "source-kernel".into(),
            }),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "opencode".into(),
                profile_id: profile.profile_id.clone(),
                label: "Synthetic source".into(),
                origin: ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "data/opencode/auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"openai":{"type":"api","key":"synthetic-source"}}"#),
            }],
            generated_at_ms: 1,
        };
        registry
            .record_received_account_copy(
                "owner",
                &materialization,
                &profile.profile_id,
                ProviderAccountMaterializationTargetKind::Slice,
            )
            .unwrap();
        materialization.generated_at_ms = 2;
        let received = super::import_managed_account_copy_with_registry(
            &registry,
            "owner",
            materialization.clone(),
        )
        .unwrap();
        assert_eq!(received.auth_state, ProviderAccountAuthState::Authenticated);
        assert_eq!(std::fs::read(&auth_path).unwrap(), receiving_login);
        assert_eq!(
            received.materializations[0]
                .copy
                .as_ref()
                .unwrap()
                .copied_at_ms,
            1
        );
        // A missing authoritative local login is not a replaceable replica.
        std::fs::remove_file(&auth_path).unwrap();
        let error =
            super::import_managed_account_copy_with_registry(&registry, "owner", materialization)
                .expect_err("an owner import must still preserve authoritative local profiles");
        assert!(error
            .to_string()
            .contains("authoritative local account profile"));
        assert!(!auth_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn mp08_mp10_mp11_slice_reimport_after_remove_replaces_missing_copy() {
        if crate::test_support::isolate_environment_test() {
            return;
        }
        use crate::account_profile::*;
        use base64::Engine;
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_support::TestWorktree::new("credcopies-reimport");
        let executable = root.path().join("opencode");
        std::fs::write(&executable, "#!/bin/sh\nprintf 'fixture-opencode\\n'\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("CHARIOX_OPENCODE_BIN", executable);
        let registry = ProviderAccountProfileRegistry::open(root.path().join("accounts.json"))
            .unwrap()
            .with_machine_identity("slice-machine", "slice-kernel");
        let copied_login =
            br#"{"openai":{"type":"oauth","access":"synthetic-access","refresh":"synthetic-source"}}"#;
        let mut materialization = ProviderAccountMaterialization {
            copy_source: Some(ProviderAccountCopySource {
                machine_id: "source-machine".into(),
                kernel_id: "source-kernel".into(),
            }),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "opencode".into(),
                profile_id: "copied-account".into(),
                label: "Synthetic copy".into(),
                origin: ProviderAccountProfileOrigin::CharioxCreated,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "data/opencode/auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(copied_login),
            }],
            generated_at_ms: 1,
        };
        let profile = super::import_managed_account_copy_with_registry(
            &registry,
            "owner",
            materialization.clone(),
        )
        .unwrap();
        assert_eq!(profile.auth_state, ProviderAccountAuthState::Authenticated);
        let environment = registry
            .resolve_environment("owner", "opencode", &profile.profile_id)
            .unwrap();
        let auth_path =
            std::path::Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
        let copied_at = |profile: &ProviderAccountProfile| {
            profile.materializations[0]
                .copy
                .as_ref()
                .unwrap()
                .copied_at_ms
        };
        // A receiving login that is present but not valid is the receiver's own to fix.
        std::fs::write(&auth_path, b"{}").unwrap();
        materialization.generated_at_ms = 2;
        let error = super::import_managed_account_copy_with_registry(
            &registry,
            "owner",
            materialization.clone(),
        )
        .expect_err("an invalid receiving login must not be overwritten");
        assert!(error.to_string().contains("/slice auth login"));
        assert_eq!(std::fs::read(&auth_path).unwrap(), b"{}");
        // `/slice auth remove` deletes only the slice credential; the owner's next
        // import replaces the copy with a new tracked generation.
        std::fs::remove_file(&auth_path).unwrap();
        materialization.generated_at_ms = 3;
        let received =
            super::import_managed_account_copy_with_registry(&registry, "owner", materialization)
                .expect("an owner import must replace a removed receiving copy");
        assert_eq!(received.auth_state, ProviderAccountAuthState::Authenticated);
        assert_eq!(std::fs::read(&auth_path).unwrap(), copied_login);
        assert_eq!(copied_at(&received), 3);
    }

    #[test]
    fn account_binding_detection_covers_provider_families_and_agent_owners() {
        let mut session = RuntimeSession::new(
            "session-a",
            Some("accounts".to_string()),
            "workspace-a",
            "worktree-a",
            "machine-a",
            "kernel-a",
        );
        let mut bound = AgentInstance::new(
            "agent-bound",
            "agent-bound",
            session.id(),
            Some("reviewer".to_string()),
            "claude-headless",
            Some("opus".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        bound.set_owner_user_id("owner-a");
        bound.set_account_profile(Some("secondary".to_string()));
        let mut other_owner = bound.clone();
        other_owner.set_alias(Some("other-owner".to_string()));
        other_owner.set_owner_user_id("owner-b");
        let mut other_account = bound.clone();
        other_account.set_alias(Some("other-account".to_string()));
        other_account.set_account_profile(Some("default".to_string()));
        session.set_agents(vec![bound, other_owner, other_account]);

        assert_eq!(
            bound_agent_labels(
                &[session],
                "claude",
                |owner| owner == "owner-a",
                |account_profile| account_profile == "secondary",
            ),
            vec!["reviewer".to_string()],
        );
    }

    #[test]
    fn account_binding_detection_covers_saved_starter_and_inactive_substitutes() {
        let mut session = RuntimeSession::new(
            "session-a",
            Some("accounts".to_string()),
            "workspace-a",
            "worktree-a",
            "machine-a",
            "kernel-a",
        );
        let mut agent = AgentInstance::new(
            "agent-bound",
            "agent-bound",
            session.id(),
            Some("reviewer".to_string()),
            "codex",
            Some("gpt-5.6".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_owner_user_id("owner-a");
        agent.set_account_profile(Some("starter-account".to_string()));
        agent.add_substitute(
            AgentSubstituteProfile::new("opencode", "deepseek-v4-pro", Some("high".into()))
                .with_account_profile(Some("substitute-account".to_string())),
        );
        session.set_agents(vec![agent]);

        assert_eq!(
            bound_agent_labels(
                &[session.clone()],
                "codex",
                |owner| owner == "owner-a",
                |account_profile| account_profile == "starter-account",
            ),
            vec!["reviewer".to_string()],
            "the configured profile is an account binding",
        );
        assert_eq!(
            bound_agent_labels(
                &[session],
                "opencode",
                |owner| owner == "owner-a",
                |account_profile| account_profile == "substitute-account",
            ),
            vec!["reviewer".to_string()],
            "every configured substitute remains an account binding",
        );
    }

    #[test]
    fn account_binding_detection_resolves_default_alias_to_stable_id() {
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-account-binding-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("accounts.json"),
        )
        .unwrap();
        let profile = registry
            .migrate_effective_defaults("owner-a", &home)
            .unwrap()
            .into_iter()
            .find(|profile| profile.provider == "codex")
            .unwrap();

        assert!(account_profile_reference_matches(
            &registry,
            "owner-a",
            "codex",
            "default",
            &profile.profile_id,
        ));
        assert!(account_profile_reference_matches(
            &registry,
            "owner-a",
            "codex",
            &profile.profile_id,
            &profile.profile_id,
        ));
        assert!(!account_profile_reference_matches(
            &registry,
            "owner-a",
            "codex",
            "missing",
            &profile.profile_id,
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}

pub(crate) async fn import_managed_account_copy(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    materialization: crate::account_profile::ProviderAccountMaterialization,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
    let owner_user_id = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    let registry = runtime_state.provider_account_profile_registry().clone();
    let response = tokio::task::spawn_blocking(move || {
        import_managed_account_copy_with_registry(&registry, &owner_user_id, materialization)
    })
    .await
    .map_err(|_| DaemonError::LocalTransport {
        operation: "import provider account copy",
        message: "account import task failed".into(),
    })??;
    runtime_state
        .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
        .await;
    runtime_state.record_waiting_room_change();
    Ok(response)
}

fn import_managed_account_copy_with_registry(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    mut materialization: crate::account_profile::ProviderAccountMaterialization,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
    // One receiving authority serializes admission and installation. Recording a
    // request only after installation would leave a replay window on failure.
    static IMPORT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _import = IMPORT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::account_profile::validate_managed_context_materialization_shape(
        &materialization.profile.provider,
        &materialization,
    )?;
    if materialization.copy_source.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "import provider account copy",
            message: "managed account copy requires source machine identity".into(),
        });
    }
    registry.admit_slice_copy_generation(owner_user_id, &materialization)?;
    materialization.profile.owner_user_id = owner_user_id.to_owned();
    materialization.profile.origin =
        crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated;
    // Reconnects and repeated imports preserve a receiving machine's own login.
    if let Some(profile) = registry
        .list(owner_user_id, Some(&materialization.profile.provider))?
        .into_iter()
        .find(|profile| profile.profile_id == materialization.profile.profile_id)
    {
        // An explicit owner import may replace a removed replica. The registry still
        // refuses authoritative local profiles and managed-context accounts.
        if !registry.credential_artifact_missing(
            owner_user_id,
            &profile.provider,
            &profile.profile_id,
        ) {
            let status = crate::local::provider_requests::observe_provider_auth_status(
                registry,
                owner_user_id,
                &profile.provider,
                &profile.profile_id,
            )?;
            if status.auth_state != "authenticated" {
                return Err(DaemonError::LocalTransport {
                    operation: "import provider account copy",
                    message: format!(
                        "Receiving profile is logged out; run /slice auth login <slice-ref> {} {} on this receiving slice, then retry the import",
                        profile.provider, profile.profile_id,
                    ),
                });
            }
            return registry.get(owner_user_id, &profile.provider, &profile.profile_id);
        }
    }
    let profile = registry.materialize_replica(owner_user_id, &materialization)?;
    registry.record_received_account_copy(
        owner_user_id,
        &materialization,
        &profile.profile_id,
        crate::account_profile::ProviderAccountMaterializationTargetKind::Slice,
    )?;
    if crate::local::provider_requests::observe_provider_auth_status(
        registry,
        owner_user_id,
        &profile.provider,
        &profile.profile_id,
    )
    .is_err()
    {
        registry.update_observation(
            owner_user_id,
            &profile.provider,
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Error,
            None,
            None,
            None,
            None,
        )?;
    }
    registry.get(owner_user_id, &profile.provider, &profile.profile_id)
}
