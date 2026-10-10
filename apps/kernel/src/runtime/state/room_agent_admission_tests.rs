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
fn room_admission_saved_artifact_run_preserves_peer_metadata() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let root = TestRoot::new();
                    let mut config = DaemonConfig::for_tests();
                    config.room_agent_tools = true;
                    config.user_config.artifacts.operational.root =
                        Some(root.path().join("artifacts").display().to_string());
                    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
                    let (room, actor) = KernelSessionService::new(&mut app)
                        .create_session(CreateSessionRequest::new(
                            root.path().to_string_lossy(),
                            root.path().to_string_lossy(),
                        ))
                        .unwrap();
                    let peer = KernelSessionService::new(&mut app)
                        .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "dev-stub"))
                        .unwrap();
                    crate::test_support::admit_room_test_turn(&mut app, room.id(), actor.id());
                    let child = KernelSessionService::new(&mut app)
                        .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "dev-stub")
                            .with_spawned_by_agent_id(actor.id()))
                        .unwrap();
                    let registry = WorkflowCodeArtifactRegistry::new(vec![config
                        .workflow_code_artifact_root().join("rooms").join(room.id())]);
                    for (name, creator) in [("self", Some(actor.id())), ("child", Some(child.id())),
                        ("peer", Some(peer.id())), ("unknown", None)] {
                        registry.save(name, WorkflowCodeLanguage::JavaScript, "review source",
                            serde_json::from_value(serde_json::json!({"workflow":{},
                                "nodes":[{"handle":"review", "agent":{"kind":"existing", "agent_ref":actor.id()}}],
                                "endpoints":[{"handle":"entry","entry_node":"review"}]})).unwrap(),
                            WorkflowCodeValidationReport {ok: true, diagnostics: vec![]},
                            WorkflowCodeArtifactActor::new(actor.owner_user_id(), creator.map(str::to_owned)),
                            WorkflowCodeArtifactHistoryAction::Created).unwrap();
                    }
                    let app = Arc::new(Mutex::new(app));
                    let runtime = CommandRouter::with_interactive_capacity(app.clone(), 1).runtime_state();
                    for (name, may_update_history) in [("self", true), ("child", true), ("peer", false), ("unknown", false)] {
                        let request = LocalDaemonRequest::RunWorkflowCodeArtifact(
                            crate::local::RunWorkflowCodeArtifactRequest {
                                session_id: room.id().to_owned(), name: name.to_owned(),
                                provider_rebindings: vec![], agent_rebindings: vec![],
                                endpoint: None, queue_ref: None, prompt: "review".to_owned(),
                            });
                        let scoped = runtime.with_room_request_origin(Some(actor.id()), &request);
                        let before = registry.get(name).unwrap().unwrap();
                        let mut held = app.lock().await;
                        let result = crate::runtime::state::workflow_code_request_support::workflow_code_artifact_apply_result(
                            &mut held, name, WorkflowCodeArtifactHistoryAction::Run,
                            crate::runtime::state::workflow_code_request_support::WorkflowApplyContext {
                                session_id: room.id(), provider_rebindings: &[], agent_rebindings: &[],
                                caller_user_id: actor.owner_user_id().to_owned(),
                                controlled_by_metaagent_id: Some(actor.id().to_owned()),
                                operation: "room_artifact_test", run_endpoint: Some(Some("entry")), run_queue: None,
                                authorize: &|| scoped.authorize_current_external_command(),
                            }).unwrap();
                        let workflow = held.sessions().resolve_workflow_ref(room.id(), &result.apply.workflow_id).unwrap();
                        assert_eq!(workflow.created_by_agent_id(), Some(actor.id()));
                        let after = registry.get(name).unwrap().unwrap();
                        assert_eq!(after.metadata.provenance.created_by, before.metadata.provenance.created_by);
                        if may_update_history {
                            assert_eq!(after.metadata.history.len(), before.metadata.history.len() + 1);
                        } else {
                            assert_eq!(serde_json::to_value(&after.metadata).unwrap(), serde_json::to_value(&before.metadata).unwrap(),
                                "A01 peer/unknown source runs may create caller resources but never mutate {name} metadata");
                        }
                    }
                });
        })
        .unwrap().join().unwrap();
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

// MP-08 / MP-10 / MP-11 A01-S04: existing public bindings permit execution, not mutation.
#[tokio::test]
async fn room_admission_existing_peer_workflow_is_runnable_without_child_authority() {
    let root = TestRoot::new();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, actor) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.path().to_string_lossy(),
            root.path().to_string_lossy(),
        ))
        .unwrap();
    let peer = KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "dev-stub"))
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow_controlled_by_metaagent(
            room.id(),
            Some("peer-public".into()),
            Some(peer.id().into()),
        )
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(room.id(), workflow.id(), peer.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(room.id(), workflow.id(), node.id(), Some("entry".into()))
        .unwrap();
    let state = CommandRouter::with_interactive_capacity_from_app(Arc::new(Mutex::new(app)), 1)
        .runtime_state();
    let request =
        LocalDaemonRequest::InvokeWorkflowEndpoint(crate::local::InvokeWorkflowEndpointRequest {
            session_id: room.id().into(),
            workflow_ref: workflow.id().into(),
            endpoint_ref: endpoint.id().into(),
            queue_ref: None,
            prompt: Some("review".into()),
            publication_invocation: None,
        });
    assert!(
        state
            .owned
            .ensure_workflow_request_controlled_by_metaagent(&request, actor.id())
            .is_ok(),
        "A01-S04 any room agent may run an existing public workflow"
    );
    assert!(
        crate::runtime::room_tool_admission::workflow_node(&actor, &peer).is_err(),
        "creating a new peer binding must retain its mutation fence"
    );
    assert!(state
        .owned
        .ensure_workflow_controlled_by_metaagent(
            room.id(),
            workflow.id(),
            actor.id(),
            "edit workflow"
        )
        .is_err());
    assert_eq!(
        state
            .owned
            .session_store
            .read()
            .resolve_workflow_ref(room.id(), workflow.id())
            .unwrap()
            .created_by_agent_id(),
        Some(peer.id())
    );
}

// MP-08 / MP-10 / MP-11 A10-S02: home admission follows the current lease, never a local provider-store row.
#[tokio::test]
async fn room_admission_current_leased_epoch_accepts_only_its_actor_and_run() {
    let root = TestRoot::new();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, actor) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.path().to_string_lossy(),
            root.path().to_string_lossy(),
        ))
        .unwrap();
    let peer = KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "dev-stub"))
        .unwrap();
    let state = CommandRouter::with_interactive_capacity_from_app(Arc::new(Mutex::new(app)), 1)
        .runtime_state();
    state
        .owned
        .agent_store
        .bind_remote_execution(
            actor.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker".into(),
                worker_machine_id: "machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-actor".into(),
                active_worker_provider_run_id: Some("worker-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let run = crate::provider::projected_leased_provider_run_id("leased-actor", "worker-run");
    assert!(
        state
            .authorize_room_provider_epoch(Some(actor.id()), Some(&run))
            .is_ok(),
        "current home-owned lease must authorize its provider epoch"
    );
    assert!(state
        .authorize_room_provider_epoch(Some(peer.id()), Some(&run))
        .is_err());
    let stale = crate::provider::projected_leased_provider_run_id("leased-actor", "previous-run");
    assert!(state
        .authorize_room_provider_epoch(Some(actor.id()), Some(&stale))
        .is_err());
    state
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(
            actor.id(),
            Some("replacement-run".into()),
        )
        .unwrap();
    assert!(
        state
            .authorize_room_provider_epoch(Some(actor.id()), Some(&run))
            .is_err(),
        "a replaced worker run cannot revive through projection"
    );
}
