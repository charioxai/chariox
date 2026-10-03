//! MP-08/MP-10/MP-11: queued ordinary turns use fresh Project inputs on the native thread.
use super::*;
use crate::project_environment::*;

#[derive(Clone, Copy)]
enum Case {
    Refresh,
    SynchronousCompletion,
    ReplacementFailure,
    VaultCancellation,
}

#[tokio::test]
async fn mp08_mp10_mp11_queued_project_refresh_preserves_native_conversation() {
    run_case(Case::Refresh).await;
}

#[tokio::test]
async fn mp08_mp10_mp11_queued_project_replacement_failure_preserves_queue() {
    run_case(Case::ReplacementFailure).await;
}

#[tokio::test]
async fn mp08_mp10_mp11_queued_project_vault_cancellation_preserves_queue() {
    run_case(Case::VaultCancellation).await;
}

#[tokio::test]
async fn mp08_mp10_mp11_queued_project_synchronous_completion_defers_activation() {
    run_case(Case::SynchronousCompletion).await;
}

async fn run_case(case: Case) {
    use std::os::unix::fs::PermissionsExt;
    let _env = crate::env_lock::lock();
    let workspace = crate::test_support::TestWorktree::new("queued-project-native-refresh");
    let mut config = crate::config::DaemonConfig::for_tests();
    let fixture = config.private_runtime_state_root().join("codex-fixture.py");
    std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
    std::fs::write(
        &fixture,
        include_str!("project_queued_environment_fixture.py"),
    )
    .unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &fixture);
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let vault_path = config
        .private_runtime_state_root()
        .join("project-vault.json");
    if matches!(case, Case::VaultCancellation) {
        config.user_config.credential_vault.path = vault_path.to_string_lossy().into_owned();
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "synthetic-fixture-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
    } else {
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::ProcessMemory;
    }
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex")
                .with_model("fixture-model"),
        )
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "project-queue-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    save_manifest(&app, &session);
    std::fs::write(workspace.path().join(".env"), "APP_LABEL=before\n").unwrap();
    app.launch_provider(
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "fixture-model",
        )
        .with_agent_id(agent.id()),
    )
    .unwrap();
    let first = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "remember first native turn",
            Vec::new(),
        )
        .unwrap();
    assert!(matches!(
        first,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let second = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "continue that native conversation",
            Vec::new(),
        )
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Queued { prompt: queued } = second else {
        panic!("second ordinary turn must queue during the first");
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            runtime.owned.reap_structured_prompt_jobs();
            if runtime
                .owned
                .provider_store
                .get_run_for_agent(session.id(), agent.id())
                .unwrap()
                .provider_session_id()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let first_run = runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .unwrap();
    std::fs::write(workspace.path().join(".env"), "APP_LABEL=after\n").unwrap();
    if matches!(case, Case::ReplacementFailure) {
        std::fs::remove_file(&fixture).unwrap();
    }
    if matches!(case, Case::VaultCancellation) {
        crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
    }
    if matches!(case, Case::SynchronousCompletion) {
        let completion = runtime
            .owned
            .complete_local_prompt_with_queued_advance(
                session.id(),
                agent.id(),
                Some(first_run.id()),
                &queued,
            )
            .unwrap()
            .unwrap();
        assert!(completion.completion.started_next.is_none());
        let state = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        assert!(state.active_prompt_for_agent(agent.id()).is_none());
        assert_eq!(
            state.queued_prompts_for_agent(agent.id()).unwrap()[0].id(),
            queued.id()
        );
        let mut dispatches = WorkflowPromptDispatches::default();
        dispatches
            .project_queue_promotions
            .push((session.id().to_string(), agent.id().to_string()));
        runtime.spawn_workflow_prompt_dispatches(dispatches);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap()
                .active_prompt_for_agent(agent.id())
                .is_none()
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    let settle =
        runtime.settle_owned_provider_prompt(session.id(), first_run.id(), true, false, true);
    let settlement = if matches!(case, Case::SynchronousCompletion) {
        crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: false,
            cancelled_prompt: false,
            started_next_prompt: true,
        }
    } else if matches!(case, Case::VaultCancellation) {
        let cancel = async {
            loop {
                let state = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap();
                if let Some(interaction) = state
                    .active_interactions()
                    .iter()
                    .find(|i| i.id().starts_with("vault-unlock-"))
                {
                    assert!(
                        state.active_prompt_for_agent(agent.id()).is_none(),
                        "the first turn settles before Vault unlock"
                    );
                    assert_eq!(
                        state.queued_prompts_for_agent(agent.id()).unwrap()[0].id(),
                        queued.id()
                    );
                    runtime
                        .resolve_runtime_interaction(session.id(), interaction.id(), "cancel", None)
                        .await
                        .unwrap();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(settle, cancel).0
        })
        .await
        .unwrap()
        .unwrap()
    } else {
        settle.await.unwrap()
    };
    if matches!(case, Case::ReplacementFailure | Case::VaultCancellation) {
        app.lock()
            .await
            .end_agent_provider_run(session.id(), agent.id())
            .unwrap();
        let state = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        assert!(!settlement.started_next_prompt);
        assert!(state.active_prompt_for_agent(agent.id()).is_none());
        let backlog = state.queued_prompts_for_agent(agent.id()).unwrap();
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].id(), queued.id());
        assert_eq!(backlog[0].prompt(), queued.prompt());
        let native: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(
            native["threads"]["native-thread-1"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "no stale queued turn may reach the provider"
        );
        assert!(runtime
            .owned
            .operational_history_store
            .load_prompt_settlement_event(
                session.id(),
                agent.id(),
                match first {
                    crate::session::PromptSubmissionOutcome::Started { ref prompt } => prompt.id(),
                    _ => unreachable!(),
                }
            )
            .unwrap()
            .is_some());
        return;
    }
    assert!(settlement.started_next_prompt);
    let refreshed = runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .unwrap();
    if refreshed.id() == first_run.id() {
        app.lock()
            .await
            .end_agent_provider_run(session.id(), agent.id())
            .unwrap();
    }
    assert_ne!(
        refreshed.id(),
        first_run.id(),
        "changed Project values must replace the idle process before queued dispatch"
    );
    assert_ne!(
        refreshed.project_environment_revision(),
        first_run.project_environment_revision()
    );
    // Drive only the existing kernel/provider mailbox, without launching a model.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            runtime.owned.reap_structured_prompt_jobs();
            let state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(fixture.with_extension("json")).unwrap())
                    .unwrap();
            if state["threads"]["native-thread-1"]
                .as_array()
                .is_some_and(|turns| turns.len() == 2)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.lock()
        .await
        .end_agent_provider_run(session.id(), agent.id())
        .unwrap();
    let native: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.with_extension("json")).unwrap()).unwrap();
    assert_eq!(
        native["threads"].as_object().unwrap().len(),
        1,
        "no blank conversation may be created"
    );
    let turns = native["threads"]["native-thread-1"].as_array().unwrap();
    assert_eq!(turns[0]["project_label"], "before");
    assert_eq!(turns[1]["project_label"], "after");
    assert_eq!(
        turns[1]["previous"][0], turns[0],
        "the second turn must retain the first native turn"
    );
    assert_eq!(native["resumes"], serde_json::json!(["native-thread-1"]));
}

fn save_manifest(app: &DaemonApp, session: &crate::session::RuntimeSession) {
    ProjectEnvironmentStore::new(&app.config().private_runtime_state_root())
        .save(&StoredProjectEnvironment {
            source: None,
            manifest: ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: session.project_id().into(),
                evidence_digest: ProjectEnvironmentEvidence::default().digest(),
                entries: vec![ProjectEnvironmentEntry {
                    name: "APP_LABEL".into(),
                    workspace_id: session.workspace_id().into(),
                    kind: ProjectEnvironmentEntryKind::Variable,
                    classification: ProjectEnvironmentClassification::NonSecret,
                    excluded: false,
                    uses: vec![ProjectEnvironmentUse {
                        path: "app.ts".into(),
                        line: 1,
                    }],
                    locator: ProjectEnvironmentLocator::EnvFile {
                        path: ".env".into(),
                        key: "APP_LABEL".into(),
                    },
                    status: ProjectEnvironmentEntryStatus::Found,
                }],
                private_files: vec![],
                toolchain_hints: vec![],
                package_hints: vec![],
                service_hints: vec![],
            },
            evidence: ProjectEnvironmentEvidence::default(),
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        })
        .unwrap();
}
