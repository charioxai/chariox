//! MP-08 / MP-10 / MP-11: forwarded response release keeps the strict prompt witness.
use super::*;
use crate::agent::RemoteAgentBinding;
use crate::execution_lease::RemoteWorkflowTurnContext;
use crate::runtime::router::CommandRouter;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL;

struct Fixture {
    runtime: KernelRuntimeState,
    context: RemoteWorkflowTurnContext,
    _worktree: crate::test_support::TestWorktree,
}

impl Fixture {
    fn new() -> Self {
        Self::with_publication(false)
    }

    fn with_publication(publication: bool) -> Self {
        let worktree = crate::test_support::TestWorktree::new("forwarded-workflow-release");
        let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), None)
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .unwrap();
        app.sessions_mut()
            .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
            .unwrap();
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint_with_publication_invocation(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("return output".into()),
                publication.then(|| crate::session::WorkflowPublicationInvocationEnvelope {
                    publication_id: "release-publication".into(),
                    hook_id: None,
                    invocation_id: "release-invocation".into(),
                    transport: "human_http".into(),
                    endpoint_id: endpoint.id().into(),
                    queue_ref: None,
                    input: serde_json::json!({"prompt": "return output"}),
                    artifacts: vec![],
                    mode: Some("sync".into()),
                    caller: serde_json::json!({"type": "anonymous"}),
                }),
            )
            .unwrap();
        let node_run_id = run.node_runs()[0].id().to_string();
        let delivery_token = format!("workflow-ack:{node_run_id}");
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                run.id(),
                &node_run_id,
                delivery_token.clone(),
                "return output".into(),
                None,
                None,
            )
            .unwrap();
        app.sessions_mut()
            .start_workflow_node_run(session.id(), run.id(), &node_run_id)
            .unwrap();
        app.agents()
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".into(),
                    worker_machine_id: "worker-machine-1".into(),
                    execution_lease_id: "release-lease".into(),
                    leased_agent_id: "release-leased-agent".into(),
                    active_worker_provider_run_id: Some("worker-run-1".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let router = CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let runtime = router.runtime_state();
        runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().into(),
                prompt: crate::session::PromptQueueItem::new(
                    "release-prompt",
                    crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
                    agent.id(),
                    "return output",
                    crate::session::PromptStatus::Queued,
                )
                .with_workflow_context(run.id(), &node_run_id),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap();
        Self {
            runtime,
            context: RemoteWorkflowTurnContext {
                home_kernel_id: session.host_daemon_id().into(),
                home_session_id: session.id().into(),
                home_agent_id: agent.id().into(),
                workflow_run_id: run.id().into(),
                workflow_node_run_id: node_run_id,
                delivery_token,
            },
            _worktree: worktree,
        }
    }

    fn request(&self) -> RelayPeerRequest {
        RelayPeerRequest::ForwardWorkflowRuntimeTool {
            context: self.context.clone(),
            tool_name: VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.into(),
            arguments: serde_json::json!({"workflow_output_json": serde_json::json!({"answer": "released"}).to_string()}),
        }
    }

    async fn admitted(&self) -> KernelRuntimeState {
        self.runtime
            .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
                "worker-kernel-1",
            ))
            .prepare_forwarded_peer_request(&self.request())
            .await
            .unwrap()
    }

    async fn submit_with_release(
        &self,
        admitted: &KernelRuntimeState,
    ) -> crate::transport::runtime_tools::RuntimeToolResult {
        admitted.dispatch_forwarded_workflow_runtime_tool_call(
            self.context.clone(), VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.into(),
            serde_json::json!({"workflow_output_json": serde_json::json!({"answer": "released"}).to_string()}),
            |result| {
                assert!(self.active_prompt().is_some(), "home prompt settled before response release");
                admitted.with_forwarded_binding_operation(|| Ok(result))
            },
        ).await.expect("response must be released before settling the original prompt")
    }

    fn active_prompt(&self) -> Option<crate::session::PromptQueueItem> {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.context.home_session_id)
            .unwrap();
        self.runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &self.context.home_agent_id)
    }
}

#[tokio::test]
async fn mp11_forwarded_workflow_response_keeps_prompt_open_for_release() {
    let fixture = Fixture::new();
    let admitted = fixture.admitted().await;
    let result = fixture.submit_with_release(&admitted).await;
    assert!(result.ok && result.payload["submitted"] == true);
    assert!(
        fixture.active_prompt().is_none(),
        "home prompt must settle after release"
    );
}

#[tokio::test]
async fn mp11_forwarded_workflow_response_refuses_unrelated_settlement() {
    let fixture = Fixture::new();
    let admitted = fixture.admitted().await;
    fixture
        .runtime
        .owned
        .complete_remote_prompt_owner(
            &fixture.context.home_session_id,
            &fixture.context.home_agent_id,
            "unrelated-completion",
            None,
        )
        .unwrap();
    let mut released = false;
    let error = admitted
        .with_forwarded_binding_operation(|| {
            released = true;
            Ok(())
        })
        .unwrap_err();
    assert!(!released);
    assert!(error.to_string().contains("changed"), "{error}");
}

#[tokio::test]
async fn mp11_forwarded_workflow_peer_releases_response_then_settles_prompt() {
    let fixture = Fixture::new();
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        fixture.runtime.app.clone(),
        1,
    ));
    let state = Arc::new(tokio::sync::RwLock::new(
        crate::transport::relay_client::RelayClientState::default(),
    ));
    let (outgoing, _priority, _events) =
        crate::transport::relay_client::RelayOutgoingSender::channel(16);
    let worker = crate::DaemonConfig::for_tests();
    let worker_key = worker.relay_public_key.clone();
    let identity = chariox_relay::protocol::RelayCallerIdentity {
        realm_id: "realm-1".into(),
        subject: "worker-kernel-1".into(),
        subject_kind: chariox_relay::auth::RelaySubjectKind::Machine,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some("user-1".into()),
        public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
            &worker_key,
        )),
    };
    let response = crate::transport::relay_client::send_authenticated_peer_request_for_test(
        &router,
        &state,
        &outgoing,
        "worker-kernel-1",
        identity,
        &worker.relay_private_key,
        &fixture
            .runtime
            .owned
            .config_projection
            .snapshot()
            .relay_public_key,
        fixture.request(),
    )
    .await
    .expect("worker must receive the successful encrypted submit response");
    assert!(
        matches!(response, RelayPeerResponse::WorkflowRuntimeToolHandled { result } if result.ok && result.payload["submitted"] == true)
    );
    assert!(
        fixture.active_prompt().is_none(),
        "prompt must settle after release"
    );
    assert!(fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.context.home_agent_id)
        .unwrap()
        .remote_execution()
        .unwrap()
        .active_worker_provider_run_id
        .is_none());
}

#[tokio::test]
async fn mp11_forwarded_workflow_publication_keeps_prompt_open_for_release() {
    let fixture = Fixture::with_publication(true);
    let admitted = fixture.admitted().await;
    let result = fixture.submit_with_release(&admitted).await;
    assert!(result.ok && result.payload["submitted"] == true);
    assert!(
        fixture.active_prompt().is_none(),
        "publication prompt must settle after release"
    );
}

#[tokio::test]
async fn mp11_forwarded_workflow_failed_release_does_not_settle_prompt() {
    let fixture = Fixture::new();
    let admitted = fixture.admitted().await;
    let error = admitted
        .dispatch_forwarded_workflow_runtime_tool_call(
            fixture.context.clone(),
            VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.into(),
            serde_json::json!({"workflow_output_json": "{}"}),
            |_| -> Result<(), DaemonError> {
                Err(DaemonError::LocalTransport {
                    operation: "test response release",
                    message: "release refused".into(),
                })
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("release refused"));
    assert!(fixture.active_prompt().is_some());
    assert_eq!(
        fixture
            .runtime
            .owned
            .agent_store
            .get_agent(&fixture.context.home_agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .active_worker_provider_run_id
            .as_deref(),
        Some("worker-run-1")
    );
}

#[tokio::test]
async fn mp11_forwarded_workflow_submission_refuses_unrelated_prompt_settlement() {
    let fixture = Fixture::new();
    let admitted = fixture.admitted().await;
    let mut released = false;
    let error = admitted
        .dispatch_forwarded_workflow_runtime_tool_call(
            fixture.context.clone(),
            VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.into(),
            serde_json::json!({"workflow_output_json": "{}"}),
            |result| {
                let session = fixture
                    .runtime
                    .owned
                    .session_store
                    .get_session(&fixture.context.home_session_id)?;
                fixture
                    .runtime
                    .owned
                    .prompt_state_owner
                    .complete_active_prompt_only(&session, &fixture.context.home_agent_id)
                    .unwrap();
                admitted.with_forwarded_binding_operation(|| {
                    released = true;
                    Ok(result)
                })
            },
        )
        .await
        .unwrap_err();
    assert!(!released);
    assert!(error.to_string().contains("prompt changed"), "{error}");
}

#[tokio::test]
async fn mp11_forwarded_workflow_receipt_preserves_prompt_replaced_after_release() {
    let fixture = Fixture::new();
    let admitted = fixture.admitted().await;
    let mut released = false;
    let error = admitted
        .dispatch_forwarded_workflow_runtime_tool_call(
            fixture.context.clone(),
            VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.into(),
            serde_json::json!({"workflow_output_json": "{}"}),
            |result| {
                let result = admitted.with_forwarded_binding_operation(|| {
                    released = true;
                    Ok(result)
                })?;
                let session = fixture
                    .runtime
                    .owned
                    .session_store
                    .get_session(&fixture.context.home_session_id)?;
                fixture
                    .runtime
                    .owned
                    .prompt_state_owner
                    .complete_active_prompt_only(&session, &fixture.context.home_agent_id)
                    .unwrap();
                fixture.runtime.owned.submit_remote_prepared_prompt(
                    &crate::app::KernelPreparedPromptSubmission {
                        session_id: fixture.context.home_session_id.clone(),
                        prompt: crate::session::PromptQueueItem::new(
                            "replacement",
                            crate::scheduler::runtime::workflow_prompt_source_attachment_id(
                                &fixture.context.workflow_run_id,
                            ),
                            &fixture.context.home_agent_id,
                            "replacement must survive",
                            crate::session::PromptStatus::Queued,
                        ),
                        force_queue: false,
                        refresh_projection: true,
                    },
                )?;
                Ok(result)
            },
        )
        .await
        .unwrap_err();
    assert!(released);
    assert!(matches!(error, DaemonError::NoActivePrompt { .. }));
    assert_eq!(
        fixture.active_prompt().unwrap().prompt(),
        "replacement must survive"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .agent_store
            .get_agent(&fixture.context.home_agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .active_worker_provider_run_id
            .as_deref(),
        Some("worker-run-1")
    );
}
