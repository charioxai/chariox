use super::*;
use crate::project_environment::*;
// MP-08 / MP-10 / MP-11: account and Project secrets share a single
// Always-policy operation, but subsequent operations must unlock again.
#[tokio::test]
async fn cold_prompt_account_and_project_share_one_vault_unlock() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root =
        std::env::temp_dir().join(format!("prompt-vault-operation-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&root).unwrap();
    let old_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &root);
    let old_binary = std::env::var_os("CHARIOX_CLAUDE_BIN");
    let binary = root.join("claude");
    std::fs::write(&binary, "#!/bin/sh\nexit 85\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &binary);
    let path = root.join("vault.json");
    let mut config =
        crate::DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = path.display().to_string();
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::Always;
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, _) = app
        .create_session(crate::session::CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let profile = app
        .provider_account_profile_registry()
        .create_managed("local", "claude", "operation account")
        .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &path,
        "synthetic-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    crate::provider::store_verified_provider_account_credential(
        &config,
        "local",
        "claude",
        &profile.profile_id,
        "synthetic-account",
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
    crate::secret::project_environment_vault(&config)
        .unwrap()
        .set_secret("project-operation", "api", "synthetic-project")
        .unwrap();
    let evidence = ProjectEnvironmentEvidence::default();
    ProjectEnvironmentStore::new(&config.private_runtime_state_root())
        .save(&StoredProjectEnvironment {
            source: None,
            manifest: ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: session.project_id().into(),
                evidence_digest: evidence.digest(),
                entries: vec![ProjectEnvironmentEntry {
                    name: "PROJECT_API_KEY".into(),
                    workspace_id: session.workspace_id().into(),
                    kind: ProjectEnvironmentEntryKind::Variable,
                    classification: ProjectEnvironmentClassification::Secret,
                    excluded: false,
                    uses: vec![ProjectEnvironmentUse {
                        path: "app.ts".into(),
                        line: 1,
                    }],
                    locator: ProjectEnvironmentLocator::Vault {
                        service: "project-operation".into(),
                        key: "api".into(),
                    },
                    status: ProjectEnvironmentEntryStatus::Found,
                }],
                private_files: vec![],
                toolchain_hints: vec![],
                package_hints: vec![],
                service_hints: vec![],
            },
            evidence,
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        })
        .unwrap();
    crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "claude-p")
                .with_model("claude-sonnet")
                .with_account_profile(&profile.profile_id),
        )
        .unwrap();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let runtime = super::super::provider_output_runtime_tests::owned_runtime_state(&app).await;
    let mut counts = Vec::new();
    let mut delivered = Vec::new();
    for _ in 0..2 {
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-p",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let operation = runtime.with_project_prompt_environment(session.id(), agent.id(), |app| {
            app.start_provider_launch(request)
        });
        tokio::pin!(operation);
        let mut count = 0;
        let started = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                tokio::select! {
                    result = &mut operation => break result.unwrap(),
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
                let session = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap();
                if let Some(interaction) = session.active_interaction_for_agent(agent.id()) {
                    assert_eq!(interaction.title(), Some("Unlock Chariox Vault"));
                    count += 1;
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
                }
            }
        })
        .await
        .unwrap();
        counts.push(count);
        let probe = crate::provider::ProviderCredentialDeliveryProbe::install(
            started.run.id(),
            &[
                ("CLAUDE_CODE_OAUTH_TOKEN", "synthetic-account"),
                ("PROJECT_API_KEY", "synthetic-project"),
            ],
        );
        runtime
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app)
                    .spawn_for_launch_with_credentials(
                        &started.run,
                        &started.provider_credential_env,
                    )
            })
            .await
            .unwrap();
        crate::provider::ProviderProcessService::initialize_runtime_binding_with_credentials(
            &started.run,
            &started.provider_credential_env,
        )
        .unwrap();
        delivered
            .push(probe.observed_exactly("pty_spawn") && probe.observed_exactly("runtime_binding"));
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&path)
                .unwrap()
                .unlocked,
            "Always must relock when the prompt operation completes"
        );
        runtime
            .owned
            .provider_store
            .terminate_run_provider_only(session.id(), started.run.id())
            .unwrap();
    }
    app.lock().await.shutdown_cleanup().unwrap();
    crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
    crate::secret::clear_vault_secret_process_cache().unwrap();
    if let Some(value) = old_home {
        std::env::set_var("CHARIOX_HOME", value);
    } else {
        std::env::remove_var("CHARIOX_HOME");
    }
    if let Some(value) = old_binary {
        std::env::set_var("CHARIOX_CLAUDE_BIN", value);
    } else {
        std::env::remove_var("CHARIOX_CLAUDE_BIN");
    }
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        counts,
        vec![1, 1],
        "exactly one passphrase per operation, including the next prompt"
    );
    assert_eq!(
        delivered,
        vec![true, true],
        "both secret sources must reach provider launch"
    );
}
