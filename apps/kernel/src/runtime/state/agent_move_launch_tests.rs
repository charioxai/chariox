//! MP-08 / MP-10 / MP-11: launch preparation must use the durable receipt.
use super::provider_launch_owned_state::owned_runtime_state;
use super::*;

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
}
