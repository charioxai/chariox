//! MP-08 / MP-10 / MP-11: launch preparation must use the durable receipt.
use super::provider_launch_owned_state::owned_runtime_state;
use super::*;

#[tokio::test]
async fn mk03_native_probe_preserves_home_account_before_prompt_admission() {
    use crate::account_profile::ProviderAccountAuthState;
    use crate::provider::{NativeCheckpointFailure, ProviderResumeState};

    let worktree = crate::test_support::TestWorktree::new("move-return-account-admission");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, mut agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let owner = crate::account_profile::provider_account_authority_owner_user_id(
        app.config(),
        agent.owner_user_id(),
    );
    let registry = app.provider_account_profile_registry();
    let source = registry.create_managed(&owner, "codex", "source").unwrap();
    let environment = registry
        .resolve_environment(&owner, "codex", &source.profile_id)
        .unwrap();
    let source_root = std::path::PathBuf::from(&environment["CODEX_HOME"]);
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(
        source_root.join("auth.json"),
        br#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture-only"}"#,
    )
    .unwrap();
    registry
        .update_observation(
            &owner,
            "codex",
            &source.profile_id,
            ProviderAccountAuthState::Unknown,
            Some("source@example.invalid".into()),
            None,
            None,
            None,
        )
        .unwrap();
    agent.set_provider("codex");
    agent.set_account_profile(Some(source.profile_id.clone()));
    app.agents().restore_agent(agent.clone());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert!(
        !runtime.owned.provider_account_allows_queued_prompt_advance(
            session.id(),
            &agent,
            "unverified Return account",
        )
    );
    assert!(runtime
        .finish_agent_move_native_probe(
            &owner,
            &source.profile_id,
            Err(NativeCheckpointFailure::AccountMismatch),
        )
        .is_err());
    assert_eq!(
        registry
            .get(&owner, "codex", &source.profile_id)
            .unwrap()
            .auth_state,
        ProviderAccountAuthState::Unknown
    );
    runtime
        .finish_agent_move_native_probe(
            &owner,
            &source.profile_id,
            Ok(ProviderResumeState::from_codex_thread_id(
                "00000000-0000-0000-0000-000000000008",
            )),
        )
        .unwrap();
    assert!(
        runtime.owned.provider_account_allows_queued_prompt_advance(
            session.id(),
            &agent,
            "verified Return account",
        ),
        "a successful native account/session probe must admit the returned agent's next prompt"
    );
    registry
        .update_observation(
            &owner,
            "codex",
            &source.profile_id,
            ProviderAccountAuthState::Authenticated,
            Some("source@example.invalid".into()),
            None,
            None,
            None,
        )
        .expect("ordinary auth observation must keep the original home account usable");
    let original = registry.get(&owner, "codex", &source.profile_id).unwrap();
    assert_eq!(original.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(
        original.identity_summary.as_deref(),
        Some("source@example.invalid")
    );
    assert!(runtime.owned.provider_store.list_runs().is_empty());
}

#[tokio::test]
async fn mk03_return_native_preparation_keeps_remote_route_and_dispatch_fence() {
    use crate::agent_move::*;
    use crate::managed_context::development::*;

    let worktree = crate::test_support::TestWorktree::new("move-return-preparation");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, mut agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let config = app.config().clone();
    let remote = crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker".into(),
        worker_machine_id: "machine".into(),
        execution_lease_id: "lease".into(),
        leased_agent_id: "leased-agent".into(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(240),
    };
    agent.set_remote_execution(Some(remote.clone()));
    app.agents().restore_agent(agent.clone());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let identity = AgentMoveIdentity {
        operation_id: "return-native-preparation".into(),
        generation: 2,
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        owner_user_id: agent.owner_user_id().into(),
        source_kernel_id: remote.worker_kernel_id.clone(),
        target_kernel_id: config.daemon_id.clone(),
        home_key_thumbprint: "home-key".into(),
        source_key_thumbprint: "worker-key".into(),
        target_key_thumbprint: "home-key".into(),
        target_public_key: config.relay_public_key.clone(),
    };
    let capture = AgentMoveCapture {
        identity: identity.clone(),
        source_project_id: session.project_id().into(),
        archive_project_id: session.project_id().into(),
        source_provider_run_id: None,
        source_repositories: Vec::new(),
        workspace: AgentMoveArtifact {
            sha256: "0".repeat(64),
            size_bytes: 1,
        },
        native: None,
        checkpoint_binding: None,
    };
    let mut target = runtime
        .owned
        .agent_moves
        .begin_target(capture.clone(), &worktree.path().join("moves"))
        .unwrap();
    let primary = target.destination_root.join("primary");
    std::fs::create_dir_all(&primary).unwrap();
    let receipt = AgentMoveMaterialization {
        identity: identity.clone(),
        archive_sha256: capture.workspace.sha256.clone(),
        source_project_id: session.project_id().into(),
        target_project_id: session.project_id().into(),
        destination_root: target.destination_root.clone(),
        primary_repository_id: "primary".into(),
        repositories: vec![DevelopmentImportedRepository {
            workspace_kind: DevelopmentWorkspaceKind::Directory,
            repository_id: "primary".into(),
            role: DevelopmentRepositoryRole::Primary,
            target_directory: "primary".into(),
            destination_path: primary.clone(),
            head_sha: String::new(),
        }],
    };
    target.receipt = Some(receipt.clone());
    target.backing_session_id = Some(session.id().into());
    target.backing_agent_id = Some(agent.id().into());
    runtime
        .owned
        .agent_moves
        .save_target(target.clone())
        .unwrap();
    let operation = HomeMoveOperation {
        composition: Default::default(),
        identity: identity.clone(),
        revision: 0,
        original_agent: agent.clone(),
        target_ref: None,
        source_remote: Some(remote.clone()),
        phase: HomeMovePhase::Bound {
            capture,
            receipt,
            remote: None,
        },
    };
    let operation = runtime.owned.agent_moves.save_home(operation).unwrap();
    let mut request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "model",
    )
    .with_agent_id(agent.id())
    .with_owner_user_id(agent.owner_user_id())
    .with_working_directory(primary);
    assert!(runtime
        .prepare_provider_launch_request_with_vault(request.clone(), "ordinary remote launch")
        .await
        .is_err());
    assert!(app
        .lock()
        .await
        .prepare_app_provider_launch_request(request.clone(), "ordinary remote app launch",)
        .is_err());
    request.agent_move_preparation = Some(identity.operation_id.clone());
    let prepared = runtime
        .prepare_provider_launch_request_with_vault(
            request.clone(),
            "prepare returned native session",
        )
        .await
        .expect("the captured Return must prepare before clearing its remote route");
    app.lock()
        .await
        .prepare_app_provider_launch_request(prepared.clone(), "prepare fenced Return setup")
        .expect("app admission must accept the exact fenced Return");
    assert_eq!(prepared.working_directory, request.working_directory);
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .remote_execution(),
        Some(&remote),
    );
    let mut ordinary = prepared;
    ordinary.agent_move_preparation = None;
    let error = runtime.owned.start_provider_launch(ordinary).unwrap_err();
    assert!(error.to_string().contains("holds provider admission"));
    assert!(runtime.owned.provider_store.list_runs().is_empty());

    let mut wrong_operation = request.clone();
    wrong_operation.agent_move_preparation = Some("other-operation".into());
    let mut wrong_directory = request.clone();
    wrong_directory.working_directory = Some(worktree.path().join("other"));
    for invalid in [wrong_operation, wrong_directory] {
        assert!(app
            .lock()
            .await
            .prepare_app_provider_launch_request(invalid.clone(), "invalid Return app preparation",)
            .is_err());
        assert!(runtime
            .prepare_provider_launch_request_with_vault(invalid, "invalid Return preparation")
            .await
            .is_err());
    }
    let mut changed_source = remote.clone();
    changed_source.execution_lease_id = "different-lease".into();
    let mut rebound = agent.clone();
    rebound.set_remote_execution(Some(changed_source));
    runtime.owned.agent_store.restore_agent(rebound);
    assert!(runtime
        .prepare_provider_launch_request_with_vault(request.clone(), "changed Return source")
        .await
        .is_err());
    runtime.owned.agent_store.restore_agent(agent.clone());
    target.active = true;
    runtime
        .owned
        .agent_moves
        .save_target(target.clone())
        .unwrap();
    assert!(runtime
        .prepare_provider_launch_request_with_vault(request.clone(), "already active Return")
        .await
        .is_err());
    target.active = false;
    let mut uncaptured = operation.clone();
    uncaptured.phase = HomeMovePhase::Fenced;
    runtime.owned.agent_moves.save_home(uncaptured).unwrap();
    assert!(runtime
        .prepare_provider_launch_request_with_vault(request, "uncaptured Return")
        .await
        .is_err());

    let before = runtime.owned.session_snapshot(session.id()).unwrap();
    assert!(before.agents()[0].remote_execution().is_some());
    let sequence = runtime
        .owned
        .session_projection
        .session_change_sequence(session.id());
    let mut committed = runtime
        .owned
        .agent_moves
        .for_agent(session.id(), agent.id())
        .unwrap();
    let mut target = runtime
        .owned
        .agent_moves
        .target(&identity.operation_id)
        .unwrap();
    target.ready = true;
    target.active = false;
    runtime
        .owned
        .agent_moves
        .save_target(target.clone())
        .unwrap();
    let requirements = runtime.agent_move_requirements(&agent).await.unwrap();
    let readiness = runtime
        .finish_agent_move_readiness(&identity, &requirements)
        .await
        .unwrap();
    target = runtime
        .owned
        .agent_moves
        .target(&identity.operation_id)
        .unwrap();
    committed.composition.requirements = Some(requirements);
    committed.composition.readiness = Some(readiness);
    committed.phase = HomeMovePhase::Committed {
        receipt: target.receipt.clone().unwrap(),
    };
    runtime.owned.agent_moves.save_target(target).unwrap();
    runtime
        .owned
        .agent_moves
        .save_home(committed.clone())
        .unwrap();
    // The offline worker can delay source retirement, but the committed local
    // route must already be visible to existing Room subscribers.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime.resume_agent_move_operation(committed),
    )
    .await;
    assert!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .remote_execution()
            .is_none(),
        "the recovered durable mutation must have cleared the route before checking publication"
    );
    let projected = runtime.owned.session_projection.get(session.id()).unwrap();
    assert!(
        projected.agents()[0].remote_execution().is_none(),
        "recovered Committed Return must publish the local route without a fresh client read"
    );
    assert!(
        runtime
            .owned
            .session_projection
            .session_change_sequence(session.id())
            > sequence,
        "existing Room subscriptions must wake for the recovered Return"
    );
}

async fn refuses_bad_receipt(missing: bool) {
    let worktree = crate::test_support::TestWorktree::new("move-launch-receipt");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let config = app.config().clone();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let identity = crate::agent_move::AgentMoveIdentity {
        operation_id: "move-launch-receipt".into(),
        generation: 1,
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        owner_user_id: agent.owner_user_id().into(),
        source_kernel_id: config.daemon_id.clone(),
        target_kernel_id: config.daemon_id.clone(),
        home_key_thumbprint: "home-key".into(),
        source_key_thumbprint: "home-key".into(),
        target_key_thumbprint: "home-key".into(),
        target_public_key: config.relay_public_key.clone(),
    };
    let capture = crate::agent_move::AgentMoveCapture {
        identity: identity.clone(),
        source_project_id: session.project_id().into(),
        archive_project_id: session.project_id().into(),
        source_provider_run_id: None,
        source_repositories: Vec::new(),
        workspace: crate::agent_move::AgentMoveArtifact {
            sha256: "0".repeat(64),
            size_bytes: 1,
        },
        native: None,
        checkpoint_binding: None,
    };
    let mut target = runtime
        .owned
        .agent_moves
        .begin_target(capture, &worktree.path().join("moves"))
        .unwrap();
    target.backing_session_id = Some(session.id().into());
    target.backing_agent_id = Some(agent.id().into());
    target.active = true;
    target.ready = true;
    if !missing {
        target.receipt = Some(crate::agent_move::AgentMoveMaterialization {
            identity,
            archive_sha256: "0".repeat(64),
            source_project_id: session.project_id().into(),
            target_project_id: session.project_id().into(),
            destination_root: target.destination_root.clone(),
            primary_repository_id: "missing".into(),
            repositories: Vec::new(),
        });
    }
    runtime.owned.agent_moves.save_target(target).unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "model",
    )
    .with_agent_id(agent.id());
    let result = runtime
        .owned
        .prepare_provider_launch_request_without_account_credentials(
            request,
            config.runtime_mcp_url(),
        );
    assert!(
        result.is_err(),
        "launch must refuse a missing or malformed Move receipt"
    );
    let reason = result.unwrap_err().to_string();
    assert!(
        reason.contains("receipt") && (missing || reason.contains("primary workspace")),
        "{reason}"
    );
    assert!(runtime.owned.provider_store.list_runs().is_empty());
}

#[tokio::test]
async fn mk03_launch_refuses_missing_move_receipt() {
    refuses_bad_receipt(true).await;
}
#[tokio::test]
async fn mk03_launch_refuses_invalid_move_primary() {
    refuses_bad_receipt(false).await;
}

#[test]
fn mk03_chunk_admission_preserves_prefix_and_allows_retry_without_free_space() {
    let root = crate::test_support::TestWorktree::new("move-chunk-admission");
    let store = crate::agent_move::AgentMoveStore::open(root.path().join("journal")).unwrap();
    let identity = crate::agent_move::AgentMoveIdentity {
        operation_id: "chunk-admission".into(),
        generation: 1,
        home_kernel_id: "home".into(),
        home_session_id: "room".into(),
        home_agent_id: "agent".into(),
        owner_user_id: "owner".into(),
        source_kernel_id: "home".into(),
        target_kernel_id: "target".into(),
        home_key_thumbprint: "home-key".into(),
        source_key_thumbprint: "home-key".into(),
        target_key_thumbprint: "target-key".into(),
        target_public_key: "target-public".into(),
    };
    let capture = crate::agent_move::AgentMoveCapture {
        identity: identity.clone(),
        source_project_id: "project".into(),
        archive_project_id: "project".into(),
        source_provider_run_id: None,
        source_repositories: Vec::new(),
        workspace: crate::agent_move::AgentMoveArtifact {
            sha256: "0".repeat(64),
            size_bytes: 2,
        },
        native: None,
        checkpoint_binding: None,
    };
    store
        .begin_target(capture, &root.path().join("moves"))
        .unwrap();
    let kind = crate::agent_move::AgentMoveArtifactKind::Workspace;
    crate::managed_context::limits::with_free_disk(0, || {
        let result = store.write_chunk(&identity.operation_id, kind, 0, &[1]);
        assert!(
            result.is_err(),
            "staging must admit each append against remaining disk"
        );
        assert!(result.unwrap_err().to_string().contains("space"));
    });
    store
        .write_chunk(&identity.operation_id, kind, 0, &[1])
        .unwrap();
    crate::managed_context::limits::with_free_disk(0, || {
        store
            .write_chunk(&identity.operation_id, kind, 0, &[1])
            .expect("a verified retry writes no bytes");
        assert!(store
            .write_chunk(&identity.operation_id, kind, 1, &[2])
            .is_err());
    });
    let files = std::fs::read_dir(store.artifact_root(&identity.operation_id))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path()).unwrap(), [1]);
    let store = crate::agent_move::AgentMoveStore::open(root.path().join("journal")).unwrap();
    store
        .write_chunk(&identity.operation_id, kind, 0, &[1, 2])
        .expect("retry after a partial write must repair the interrupted chunk");
    assert_eq!(std::fs::read(files[0].path()).unwrap(), [1, 2]);
    std::fs::write(files[0].path(), [0, 0]).unwrap();
    store
        .write_chunk(&identity.operation_id, kind, 0, &[1, 2])
        .expect("unpublished staging with crash-corrupted data must be rewritable");
    assert_eq!(std::fs::read(files[0].path()).unwrap(), [1, 2]);
}

// MP-08 / MP-09 / MP-10 / MP-11: a crash after terminal persistence cannot strand its backlog.
#[tokio::test]
async fn mk03_restart_promotes_queue_of_already_rolled_back_move() {
    use crate::agent_move::*;
    let worktree = crate::test_support::TestWorktree::new("move-settled-queue-restart");
    let config = crate::config::DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, mut agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    agent.set_provider("dev-stub");
    agent.set_model(Some("queue-test-model".into()));
    agent.set_account_profile(None);
    app.agents().restore_agent(agent.clone());
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "settled-queue",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "queued-at-crash",
        attachment.id(),
        agent.id(),
        "queued at crash",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, true)
        .unwrap();
    app.agent_moves
        .save_home(HomeMoveOperation {
            composition: Default::default(),
            identity: AgentMoveIdentity {
                operation_id: "settled-queue".into(),
                generation: 1,
                home_kernel_id: config.daemon_id.clone(),
                home_session_id: session.id().into(),
                home_agent_id: agent.id().into(),
                owner_user_id: agent.owner_user_id().into(),
                source_kernel_id: config.daemon_id.clone(),
                target_kernel_id: "target".into(),
                home_key_thumbprint: "home-key".into(),
                source_key_thumbprint: "home-key".into(),
                target_key_thumbprint: "target-key".into(),
                target_public_key: "target-public".into(),
            },
            revision: 0,
            original_agent: agent.clone(),
            target_ref: Some("target".into()),
            source_remote: None,
            phase: HomeMovePhase::RolledBack {
                reason: "source restored before crash".into(),
            },
        })
        .unwrap();
    app.save_durable_state_snapshot().unwrap();
    drop(app);
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let runtime = owned_runtime_state(&app).await;
    assert!(
        runtime
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent.id())
            .is_some(),
        "the real restart must restore the queued prompt"
    );
    let output_runtime = runtime.clone();
    let output_pump = tokio::spawn(async move { output_runtime.run_provider_output_pump().await });
    assert_eq!(runtime.recover_agent_moves_after_restart().await, 0);
    let promoted = tokio::time::timeout(Duration::from_secs(5), async {
        while runtime
            .owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent.id())
            .is_some()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    output_pump.abort();
    let _ = output_pump.await;
    assert!(
        promoted.is_ok(),
        "terminal journal recovery must promote the restored backlog"
    );
    assert!(
        runtime
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent.id())
            .is_some(),
        "restart recovery must start its provider"
    );
}

#[tokio::test]
async fn mk03_environment_discovery_does_not_borrow_the_fenced_move_agent() {
    use crate::agent_move::*;
    use crate::managed_context::development::*;

    let worktree = crate::test_support::TestWorktree::new("move-discovery-identity");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let config = app.config().clone();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .agent_moves
        .save_home(HomeMoveOperation {
            composition: Default::default(),
            identity: AgentMoveIdentity {
                operation_id: "discovery-identity".into(),
                generation: 1,
                home_kernel_id: config.daemon_id.clone(),
                home_session_id: session.id().into(),
                home_agent_id: agent.id().into(),
                owner_user_id: agent.owner_user_id().into(),
                source_kernel_id: config.daemon_id.clone(),
                target_kernel_id: "worker".into(),
                home_key_thumbprint: "home-key".into(),
                source_key_thumbprint: "home-key".into(),
                target_key_thumbprint: "worker-key".into(),
                target_public_key: "worker-public-key".into(),
            },
            revision: 0,
            original_agent: agent.clone(),
            target_ref: None,
            source_remote: None,
            phase: HomeMovePhase::Fenced,
        })
        .unwrap();
    let project = runtime
        .owned
        .session_store
        .get_project(session.project_id())
        .unwrap();
    let repositories = vec![DevelopmentRepositorySelection {
        workspace_id: session.workspace_id().into(),
        worktree_id: None,
        worktree_path: worktree.path().to_path_buf(),
        role: DevelopmentRepositoryRole::Primary,
    }];
    let (utility_session, utility_agent, cleanup) = runtime
        .environment_utility_identity(&project, &repositories)
        .await
        .unwrap();
    assert_ne!(
        utility_agent,
        agent.id(),
        "a fenced move agent cannot run utilities"
    );
    assert_ne!(utility_session, session.id());
    assert!(
        cleanup.is_some(),
        "the replacement utility session is temporary"
    );
}
