//! MP-08/MP-10/MP-11: execution account namespace regressions.
use super::*;

// MP-08/MP-10/MP-11: a cold leased launch must admit its first prompt and
// advance the startup queue using the same account as launch preparation.
#[tokio::test]
async fn leased_account_authority_preserves_prompt_admission_and_queue() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("leased-account-admission");
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    let mut config = crate::config::DaemonConfig::load_from_env();
    config.accept_remote_leases = true;
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let registry = app.provider_account_profile_registry();
    let profile = registry
        .create_managed("cloud-owner", "codex", "selected")
        .unwrap();
    registry
        .update_observation(
            "cloud-owner",
            "codex",
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let leased = {
        let mut leases = crate::app::RemoteLeaseRuntime::new(&mut app);
        let lease = leases
            .create_execution_lease("home", "session", "agent", false, "cloud-owner")
            .unwrap();
        leases
            .create_leased_agent_from_base_directory(
                root.path(),
                &lease.id,
                "codex",
                &profile.profile_id,
                Some("model".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let request = runtime
        .prepare_provider_launch_request_with_vault(
            crate::provider::LaunchProviderRequest::new(
                &leased.backing_session_id,
                "codex",
                "codex",
                &profile.profile_id,
                "model",
            )
            .with_agent_id(&leased.backing_agent_id)
            .with_structured_endpoint("ws://127.0.0.1:9"),
            "MP-08/MP-10/MP-11 admission regression",
        )
        .await
        .unwrap();
    // This supplementary test supplies no server and starts no provider process.
    let run = runtime.owned.start_provider_launch(request).unwrap().run;
    runtime
        .owned
        .provider_run_projection
        .mark_leased_provider_run(run.id());
    let admitted = crate::app::KernelAgentService::new(&mut *app.lock().await)
        .submit_prompt_holding_dispatch(
            &leased.backing_session_id,
            &leased.backing_attachment_id,
            Some(&leased.backing_agent_id),
            "MP-08/MP-10/MP-11 actual cold-start admission guard",
            "",
            vec![],
        );
    let _ = admitted.expect("leased prompt admission must keep the account");
    let agent = runtime
        .owned
        .agent_store
        .get_agent(&leased.backing_agent_id)
        .unwrap();
    assert!(
        runtime.owned.provider_account_allows_queued_prompt_advance(
            &leased.backing_session_id,
            &agent,
            "MP-08/MP-10/MP-11 startup queue"
        ),
        "leased startup queue must keep the execution account"
    );
    let mut wrong_owner = agent.clone();
    wrong_owner.set_owner_user_id("collaborator");
    assert!(
        !runtime.owned.provider_account_allows_queued_prompt_advance(
            &leased.backing_session_id,
            &wrong_owner,
            "MP-11 mismatched lease identity"
        )
    );
    let mut ordinary_profiles = std::collections::BTreeMap::new();
    for owner in ["local", "collaborator"] {
        let profile = registry.create_managed(owner, "codex", "selected").unwrap();
        registry
            .update_observation(
                owner,
                "codex",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        ordinary_profiles.insert(owner, profile.profile_id);
    }
    let mut unavailable = agent.clone();
    unavailable.set_account_profile(Some(ordinary_profiles["local"].clone()));
    assert!(
        !runtime.owned.provider_account_allows_queued_prompt_advance(
            &leased.backing_session_id,
            &unavailable,
            "MP-11 no home account fallback"
        )
    );
    for (owner, expected) in [("cloud-owner", "local"), ("collaborator", "collaborator")] {
        let (session, ordinary) = crate::app::KernelSessionService::new(&mut *app.lock().await)
            .create_session(root.session_request().with_owner_user_id(owner))
            .unwrap();
        let ordinary = runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                ordinary.id(),
                "codex",
                Some("model".into()),
                None,
                Some(ordinary_profiles[expected].clone()),
                Default::default(),
            )
            .unwrap();
        assert!(runtime.owned.provider_account_allows_queued_prompt_advance(
            session.id(),
            &ordinary,
            "MP-08/MP-10/MP-11 ordinary admission control"
        ));
        if owner == "collaborator" {
            let mut isolated = ordinary.clone();
            isolated.set_account_profile(Some(ordinary_profiles["local"].clone()));
            assert!(
                !runtime.owned.provider_account_allows_queued_prompt_advance(
                    session.id(),
                    &isolated,
                    "MP-11 collaborator account isolation"
                )
            );
        }
    }
}

// MP-08/MP-10/MP-11: same-owner workers must use the validated lease
// namespace even when the home account alias exists on this machine.
#[tokio::test]
async fn leased_account_authority_survives_launch_reload_and_worker_recovery() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("leased-account-authority");
    std::env::set_var("CHARIOX_HOME", root.path().join("state"));
    let mut config = crate::config::DaemonConfig::load_from_env();
    config.accept_remote_leases = true;
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "cloud-owner".into(),
        ..Default::default()
    });
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::KernelInit;
    let vault = std::path::PathBuf::from(&config.user_config.credential_vault.path);
    crate::secret::unlock_chariox_encrypted_vault(
        &vault,
        "synthetic-lease-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    for provider in ["codex", "claude"] {
        for _recovery_round in 0..2 {
            let mut app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
            let registry = app.provider_account_profile_registry();
            // Same alias in all three namespaces makes accidental fallback observable.
            let mut profiles = std::collections::BTreeMap::new();
            for owner in ["local", "cloud-owner", "collaborator"] {
                let profile = registry
                    .list(owner, Some(provider))
                    .unwrap()
                    .into_iter()
                    .find(|profile| profile.label == "selected")
                    .unwrap_or_else(|| {
                        registry
                            .create_managed(owner, provider, "selected")
                            .unwrap()
                    });
                if provider == "claude" {
                    crate::provider::store_provider_account_credential(
                        &config,
                        owner,
                        provider,
                        &profile.profile_id,
                        &format!("synthetic-{owner}"),
                        true,
                    )
                    .unwrap();
                }
                profiles.insert(owner, profile.profile_id);
            }
            let leased = {
                let mut leases = crate::app::RemoteLeaseRuntime::new(&mut app);
                let lease = leases
                    .create_execution_lease(
                        "home-kernel",
                        "home-session",
                        "home-agent",
                        false,
                        "cloud-owner",
                    )
                    .unwrap();
                leases
                    .create_leased_agent_from_base_directory(
                        root.path(),
                        &lease.id,
                        provider,
                        &profiles["cloud-owner"],
                        Some("model".into()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap()
            };
            let request = crate::provider::LaunchProviderRequest::new(
                &leased.backing_session_id,
                provider,
                provider,
                &profiles["cloud-owner"],
                "model",
            )
            .with_agent_id(&leased.backing_agent_id);
            let app = Arc::new(Mutex::new(app));
            let runtime = owned_runtime_state(&app).await;
            let mut wire = serde_json::to_value(&request).unwrap();
            wire["provider_account_owner_user_id"] = serde_json::json!("local");
            let injected: crate::provider::LaunchProviderRequest =
                serde_json::from_value(wire).unwrap();
            assert!(
                injected.provider_account_owner_user_id.is_none(),
                "client requests must not select the execution namespace"
            );
            assert!(registry
                .get("cloud-owner", provider, &profiles["cloud-owner"])
                .is_ok());
            assert!(registry
                .get("local", provider, &profiles["cloud-owner"])
                .is_err());
            let prepared = runtime
                .prepare_provider_launch_request_with_vault(
                    request.clone(),
                    "MP-08/MP-10/MP-11 leased account regression",
                )
                .await
                .expect("leased launch must keep its account namespace");
            assert_eq!(
                prepared.provider_account_env,
                registry
                    .resolve_environment("cloud-owner", provider, &profiles["cloud-owner"])
                    .unwrap()
            );
            if provider == "claude" {
                assert!(prepared
                    .provider_credential_env
                    .iter()
                    .any(|(_, value)| value == "synthetic-cloud-owner"));
            }
            let run = crate::provider::RuntimeProviderRun::new(
                "leased-account-run",
                &prepared,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                    process_label: "namespace regression".into(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: vec![],
                    pty_env: Default::default(),
                    pty_env_remove: vec![],
                    working_directory: prepared.working_directory.clone(),
                    structured_endpoint: None,
                },
            );
            let reload = super::super::super::provider_reload::policy_reload_launch_request(
                &run,
                &leased.backing_agent_id,
                Default::default(),
            );
            let reloaded = runtime
                .prepare_provider_launch_request_with_vault(
                    reload,
                    "MP-08/MP-10/MP-11 leased reload regression",
                )
                .await
                .unwrap();
            assert_eq!(reloaded.provider_account_env, prepared.provider_account_env);
            if provider == "claude" {
                let credentials = runtime
                    .resolve_provider_account_credentials_for_run_with_vault(
                        &run,
                        "MP-08/MP-10/MP-11 leased recovery regression",
                    )
                    .await
                    .unwrap();
                assert!(credentials
                    .iter()
                    .any(|(_, value)| value == "synthetic-cloud-owner"));
            }
            let home_only = registry
                .list("local", Some(provider))
                .unwrap()
                .into_iter()
                .find(|profile| profile.label == "home-only")
                .unwrap_or_else(|| {
                    registry
                        .create_managed("local", provider, "home-only")
                        .unwrap()
                });
            let unavailable = runtime
                .prepare_provider_launch_request_with_vault(
                    crate::provider::LaunchProviderRequest::new(
                        &leased.backing_session_id,
                        provider,
                        provider,
                        &home_only.profile_id,
                        "model",
                    )
                    .with_agent_id(&leased.backing_agent_id),
                    "MP-11 namespace fallback rejection",
                )
                .await;
            assert!(
                unavailable.is_err(),
                "lease must not fall back to the home namespace"
            );
            // Ordinary home Cloud aliases and collaborator isolation are controls.
            for (owner, expected) in [("cloud-owner", "local"), ("collaborator", "collaborator")] {
                let (session, agent) =
                    crate::app::KernelSessionService::new(&mut *app.lock().await)
                        .create_session(root.session_request().with_owner_user_id(owner))
                        .unwrap();
                let ordinary = runtime
                    .prepare_provider_launch_request_with_vault(
                        crate::provider::LaunchProviderRequest::new(
                            session.id(),
                            provider,
                            provider,
                            &profiles[expected],
                            "model",
                        )
                        .with_agent_id(agent.id()),
                        "MP-08/MP-10/MP-11 ordinary control",
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    ordinary.provider_account_env,
                    registry
                        .resolve_environment(expected, provider, &profiles[expected])
                        .unwrap()
                );
            }
            drop(runtime);
            drop(app);
            // Real restart discards ephemeral leases; product recovery re-creates
            // their backing binding and reuses the retained account registry.
            assert!(registry
                .get("cloud-owner", provider, &profiles["cloud-owner"])
                .is_ok());
        }
    }
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
}
