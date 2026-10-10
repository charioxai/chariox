use super::super::provider_output_runtime_tests::owned_runtime_state;
use super::*;
use crate::DaemonConfig;
use crate::{agent::CreateAgentRequest, app::KernelSessionService, session::CreateSessionRequest};
use tokio::sync::Mutex;
// MP-08 / MP-10 / MP-11: the Vault account is intentionally unresolved
// at workflow admission; provenance must follow its eventual resolution.
#[tokio::test]
async fn workflow_deferred_vault_resolution_records_rejected_revision() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!("workflow-revision-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&root).unwrap();
    let old_home = std::env::var_os("CHARIOX_HOME");
    let old_binary = std::env::var_os("CHARIOX_CLAUDE_BIN");
    std::env::set_var("CHARIOX_HOME", &root);
    let binary = root.join("claude");
    std::fs::write(&binary, "#!/bin/sh\nexit 85\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let vault_path = root.join("vault.json");
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault_path.display().to_string();
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::Always;
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, _) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let profile = app
        .provider_account_profile_registry()
        .create_managed("local", "claude", "deferred revision")
        .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &vault_path,
        "synthetic-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    crate::provider::store_verified_provider_account_credential(
        &config,
        "local",
        "claude",
        &profile.profile_id,
        "synthetic-workflow-token",
        false,
    )
    .unwrap();
    app.provider_account_profile_registry()
        .update_observation(
            "local",
            "claude",
            &profile.profile_id,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let selected = crate::provider::provider_account_credential_verification(
        "local",
        "claude",
        &profile.profile_id,
    )
    .unwrap()
    .revision;
    crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    let agent = KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "claude-p")
                .with_model("claude-sonnet")
                .with_account_profile(&profile.profile_id),
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let (id, _) = runtime
        .owned
        .workflow_ensure_provider_run(session.id(), agent.id(), false, None)
        .unwrap();
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(&id)
            .unwrap()
            .account_credential_revision(),
        None
    );
    let probe = crate::provider::ProviderCredentialDeliveryProbe::install(
        &id,
        &[("CLAUDE_CODE_OAUTH_TOKEN", "synthetic-workflow-token")],
    );
    runtime.spawn_detached_workflow_provider_launch(id.clone());
    let interaction = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(i) = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap()
                .active_interaction_for_agent(agent.id())
            {
                break i.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    runtime
        .resolve_terminal_runtime_interaction(
            session.id(),
            interaction.id(),
            "passphrase",
            Some("synthetic-passphrase"),
            Some("local"),
        )
        .await
        .unwrap();
    let run = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let run = runtime.owned.provider_store.get_run(&id).unwrap();
            if run.state() != crate::provider::ProviderRunState::Starting {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let revision = run.account_credential_revision();
    let started = crate::app::StartedProviderLaunch {
        run,
        previous_active_run_id: None,
        provider_credential_env: Default::default(),
    };
    assert!(runtime
        .try_provider_launch_auth_recovery(
            &started,
            "API Error: 401 Invalid authentication credentials"
        )
        .await
        .unwrap());
    let observed = runtime
        .owned
        .provider_account_profiles
        .get("local", "claude", &profile.profile_id)
        .unwrap()
        .auth_state;
    // Let only this isolated synthetic account's failed CLI unwind.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            for interaction in snapshot.active_interactions() {
                runtime
                    .resolve_terminal_runtime_interaction(
                        session.id(),
                        interaction.id(),
                        "cancel",
                        None,
                        Some("local"),
                    )
                    .await
                    .unwrap();
            }
            if runtime
                .owned
                .provider_auth_recovery_runs
                .lock()
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let delivered =
        probe.observed_exactly("pty_spawn") && probe.observed_exactly("runtime_binding");
    app.lock().await.shutdown_cleanup().unwrap();
    crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    for (name, value) in [
        ("CHARIOX_HOME", old_home),
        ("CHARIOX_CLAUDE_BIN", old_binary),
    ] {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
    assert!(delivered);
    assert_eq!(
        revision, selected,
        "deferred launch must bind the exact resolved registration"
    );
    assert_eq!(
        observed,
        crate::account_profile::ProviderAccountAuthState::Expired,
        "unchanged rejected registration must require repair"
    );
}
