//! MP-08 / MP-11, A01/S03: supplementary real-writer/app-lock race regression.
use super::*;
use crate::{
    app::{DaemonApp, KernelSessionService},
    config::DaemonConfig,
    runtime::router::CommandRouter,
    session::CreateSessionRequest,
    workflow_code::*,
};

struct TestRoot(std::path::PathBuf);
impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-am1-app-wait-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn room_admission_artifact_creator_rechecked_after_app_wait() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(artifact_creator_race());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn artifact_creator_race() {
    let root = TestRoot::new();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.user_config.state.path = Some(root.path().join("state/state.db").display().to_string());
    config.user_config_path = root.path().join("config.toml");
    config = config.with_session_history_root(root.path().join("history"));
    config.user_config.artifacts.operational.root =
        Some(root.path().join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.path().join("artifact-index.db").display().to_string());
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, actor) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.path().to_string_lossy(),
            root.path().to_string_lossy(),
        ))
        .unwrap();
    let peer = KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"),
        )
        .unwrap();
    let registry = WorkflowCodeArtifactRegistry::new(vec![config
        .workflow_code_artifact_root()
        .join("rooms")
        .join(session.id())]);
    let definition: WorkflowCodeDefinition =
        serde_json::from_value(serde_json::json!({"workflow":{}})).unwrap();
    let validation = WorkflowCodeValidationReport {
        ok: true,
        diagnostics: vec![],
    };
    registry
        .save(
            "review-source",
            WorkflowCodeLanguage::JavaScript,
            "owner source",
            definition.clone(),
            validation.clone(),
            WorkflowCodeArtifactActor::new(actor.owner_user_id(), Some(actor.id().to_string())),
            WorkflowCodeArtifactHistoryAction::Created,
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let mut runtime = CommandRouter::with_interactive_capacity(app.clone(), 1).runtime_state();
    let probe = Arc::new(tokio::sync::Notify::new());
    runtime.observe_app_lock_wait_for_test(probe.clone());
    let held = app.lock().await;
    let request = LocalDaemonRequest::DeleteWorkflowCodeArtifact(
        crate::local::DeleteWorkflowCodeArtifactRequest {
            session_id: session.id().to_string(),
            name: "review-source".to_string(),
        },
    );
    let caller = actor.owner_user_id().to_string();
    let actor_id = actor.id().to_string();
    let pending = tokio::spawn(async move {
        runtime
            .execute_workflow_request(request, caller, Some(actor_id))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), probe.notified())
        .await
        .unwrap();
    // Owner removes the old object; a peer creates a new object at that name
    // while the earlier, admitted operation waits for the app lock.
    registry.delete("review-source").unwrap();
    registry
        .save(
            "review-source",
            WorkflowCodeLanguage::JavaScript,
            "peer replacement",
            definition,
            validation,
            WorkflowCodeArtifactActor::new(peer.owner_user_id(), Some(peer.id().to_string())),
            WorkflowCodeArtifactHistoryAction::Created,
        )
        .unwrap();
    drop(held);
    let (result, _) = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_err(),
        "A01/S03: stale creator admission must not delete a peer replacement"
    );
    let survivor = registry.get("review-source").unwrap().unwrap();
    assert_eq!(
        survivor
            .metadata
            .provenance
            .created_by
            .metaagent_id
            .as_deref(),
        Some(peer.id())
    );
    assert_eq!(survivor.source, "peer replacement");
}
