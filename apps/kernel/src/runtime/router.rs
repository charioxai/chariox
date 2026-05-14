use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;

use crate::app::{DaemonApp, PromptActivityStore};
use crate::history::OperationalHistoryStore;
use crate::history::SessionHistoryStore;
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::capability_executor::{CapabilityExecutorHealthStore, CapabilityRuntimeStore};
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionHistoryProjectionStore,
    SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_output_executor::TerminalOutputExecutor;
use crate::runtime::workflow_actor::WorkflowRuntime;
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::terminal::TerminalStreamHealthStore;
use crate::transport::relay_client::RelayClientState;

mod caller_identity_bridge;
mod cloud_relay_bridge;
mod composition;
mod dispatch;
mod pre_lane_dispatch;
mod priority_dispatch;
mod refresh_dispatch;
mod relay_peer_bridge;
mod runtime_tool_bridge;
mod status_projection_bridge;

pub(crate) const INTERACTIVE_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Clone)]
pub(crate) struct CommandRouter {
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: KernelRuntimeState,
    agent_runtime: AgentRuntime,
    session_runtime: SessionRuntime,
    workflow_runtime: WorkflowRuntime,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    history_projection: SessionHistoryProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    active_turns: crate::app::ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    capability_health: CapabilityExecutorHealthStore,
    capability_runtime: CapabilityRuntimeStore,
    transport_health: TransportHealthStore,
    terminal_health: TerminalStreamHealthStore,
    terminal_output_executor: TerminalOutputExecutor,
    workspace_coordinator: WorkspaceCoordinator,
    provider_launch_pending: ProviderLaunchPendingTracker,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::agent::CreateAgentRequest;
    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasSessionRequest,
        AttachToSessionRequest, CancelActivePromptRequest, CompletePromptRequest,
        CreateWorkflowEndpointRequest, CreateWorkflowRequest, CycleAgentFocusRequest,
        DeleteKernelRequest, DeleteSessionRequest, DestroyAgentRequest, DetachFromSessionRequest,
        EndSessionRequest, FocusAgentRequest, GetDaemonHealthRequest, GetProviderAuthStatusRequest,
        GetProviderCatalogRequest, GetProviderCommandCatalogsRequest, GetProviderRunRequest,
        GetSessionHistoryRequest, GetSessionStateRequest, InvokeWorkflowEndpointRequest,
        LaunchProviderRunRequest, ListAgentsRequest, ListProviderProcessesRequest,
        ListSessionsRequest, ListWorkflowRunsRequest, ListWorkflowWatchdogsRequest,
        ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest,
        PumpTerminalOutputRequest, QueryHistoryRequest, RelayStatusRequest,
        RemoveWorkflowEdgeRequest, ResizeTerminalRequest, ResolveSessionRequest,
        ResolveWorkflowRequest, RunShellCapabilityRequest, SpawnAgentRequest, SubmitPromptRequest,
        TeardownProviderProcessesRequest, UpdateSessionConfigRequest,
    };
    use crate::provider::{
        LaunchProviderRequest, OpenCodeProviderCatalog, OpenCodeProviderInfo, RuntimeProviderRun,
    };
    use crate::runtime::command::{
        KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource,
    };
    use crate::runtime::router::CommandRouter;
    use crate::session::{
        CreateSessionRequest, PromptStatus, PromptSubmissionOutcome, RuntimeInteraction,
        RuntimeInteractionChoice, RuntimeInteractionChoiceStyle, RuntimeInteractionKind,
        RuntimeInteractionLevel, SessionStatus, DEFAULT_LOCAL_USER_ID,
    };
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    fn spawn_test_agent(
        app: &mut DaemonApp,
        session_id: &str,
        alias: &str,
        provider: &str,
    ) -> crate::agent::AgentInstance {
        crate::app::KernelSessionService::new(app)
            .spawn_agent(CreateAgentRequest::new(session_id, provider).with_alias(alias))
            .expect("agent should spawn")
    }

    fn launch_test_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        adapter_key: &str,
        provider: &str,
        model: &str,
    ) -> RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(session_id, adapter_key, provider, "default", model)
                    .with_agent_id(agent_id),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    fn remote_command_for_request(
        request: &LocalDaemonRequest,
        user_id: Option<&str>,
    ) -> KernelCommand {
        KernelCommand::from_local_request_with_caller(
            "remote-command",
            KernelCommandSource::RelayClient,
            KernelCaller {
                caller_id: "client-remote".to_string(),
                caller_kind: KernelCallerKind::RemoteClient,
                user_id: user_id.map(str::to_string),
                client_id: Some("client-remote".to_string()),
                machine_id: None,
                realm_id: Some("realm-1".to_string()),
                public_key_thumbprint: Some("thumbprint-remote".to_string()),
            },
            None,
            None,
            request,
        )
    }

    fn focus_test_agent(app: &mut DaemonApp, session_id: &str, agent_id: &str) {
        crate::app::KernelSessionService::new(app)
            .focus_agent(session_id, agent_id)
            .expect("focus should succeed");
    }

    mod provider_projection;
    mod remote_authorization;
    mod session_actor_projection;
    mod session_history_projection;
    mod session_lifecycle_projection;
    mod session_read_projection;
    mod status_projection;
    mod terminal_output_projection;

    #[tokio::test]
    async fn pending_provider_launch_cleanup_does_not_wait_for_app_lock_when_projection_is_cold() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        router
            .provider_launch_pending
            .insert_for_tests("cold-session")
            .await;

        let app_guard = app.lock().await;
        let cleanup_router = router.clone();
        let cleanup_task = tokio::spawn(async move {
            cleanup_router
                .provider_launch_pending
                .clear_if_settled(
                    &cleanup_router.app,
                    "cold-session",
                    &cleanup_router.session_projection,
                    &cleanup_router.provider_run_projection,
                )
                .await;
        });

        timeout(Duration::from_millis(100), cleanup_task)
            .await
            .expect("cold pending launch cleanup should not wait for the app lock")
            .expect("cleanup task should join");
        drop(app_guard);

        assert!(
            router
                .provider_launch_pending
                .contains_for_tests("cold-session")
                .await,
            "cold cleanup should leave the guard for a later projection-backed refresh"
        );
    }

    #[tokio::test]
    async fn routes_interactive_commands_through_bounded_lane() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "cli-1".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        });
        let command = KernelCommand::from_local_request("cmd-1", None, None, &request);

        let response = router
            .dispatch(command, request)
            .await
            .expect("command should run");

        assert!(matches!(
            response,
            crate::local::LocalDaemonResponse::SessionAttached { .. }
        ));
    }

    #[tokio::test]
    async fn runtime_agent_skill_grant_survives_kernel_restart() {
        let config = DaemonConfig::for_tests();
        let (session_id, agent_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            let granted = router
                .runtime_state
                .grant_agent_skill(agent.id(), "review".to_string(), DEFAULT_LOCAL_USER_ID)
                .await
                .expect("skill grant should persist");
            assert!(granted.skill_grants().contains(&"review".to_string()));
            (session.id().to_string(), agent.id().to_string())
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_agent = app
            .agents
            .get_agent(&agent_id)
            .expect("agent should restore");
        assert_eq!(restored_agent.session_id(), session_id);
        assert!(restored_agent
            .skill_grants()
            .contains(&"review".to_string()));
    }

    #[tokio::test]
    async fn runtime_agent_capability_grants_accept_agent_id_or_public_ref() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let agent_id = agent.id().to_string();
        let agent_ref = agent.agent_ref().to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);

        for (agent_ref, skill_name) in [(agent_id, "by-id"), (agent_ref, "by-ref")] {
            let agent = router
                .runtime_state
                .grant_agent_skill(&agent_ref, skill_name.to_string(), DEFAULT_LOCAL_USER_ID)
                .await
                .expect("grant should succeed");
            assert_eq!(agent.session_id(), session.id());
            assert!(agent.skill_grants().contains(&skill_name.to_string()));
        }
    }

    #[tokio::test]
    async fn workflow_definition_survives_kernel_restart() {
        let config = DaemonConfig::for_tests();
        let (session_id, agent_id, workflow_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            let (created, _) = router
                .runtime_state
                .execute_workflow_request(
                    LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                        session_id: session.id().to_string(),
                        alias: Some("review".to_string()),
                    }),
                    DEFAULT_LOCAL_USER_ID.to_string(),
                )
                .await;
            let workflow_id = match created.expect("workflow should create") {
                LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            };
            let (added, _) = router
                .runtime_state
                .execute_workflow_request(
                    LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow_id.clone(),
                        agent_id: agent.id().to_string(),
                        expected_workflow_revision: None,
                    }),
                    DEFAULT_LOCAL_USER_ID.to_string(),
                )
                .await;
            added.expect("workflow node should add");
            (
                session.id().to_string(),
                agent.id().to_string(),
                workflow_id,
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        let workflow = restored_session
            .workflows()
            .iter()
            .find(|workflow| workflow.id() == workflow_id)
            .expect("workflow should restore");
        assert_eq!(workflow.alias(), Some("review"));
        assert_eq!(workflow.nodes().len(), 1);
        assert_eq!(workflow.nodes()[0].agent_id(), agent_id);
    }

    #[tokio::test]
    async fn runtime_end_and_delete_session_survive_kernel_restart() {
        let end_config = DaemonConfig::for_tests();
        let ended_session_id = {
            let mut app = DaemonApp::bootstrap(end_config.clone()).expect("daemon should boot");
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            router
                .runtime_state
                .end_session(session.id())
                .await
                .expect("session should end");
            session.id().to_string()
        };
        let app = DaemonApp::bootstrap(end_config).expect("daemon should reboot");
        let restored = app
            .sessions()
            .get_session(&ended_session_id)
            .expect("ended session should restore");
        assert_eq!(restored.status(), SessionStatus::Ended);
        assert!(app.agents.get_session_agents(&ended_session_id).is_empty());

        let delete_config = DaemonConfig::for_tests();
        let deleted_session_id = {
            let mut app = DaemonApp::bootstrap(delete_config.clone()).expect("daemon should boot");
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            router
                .runtime_state
                .delete_session_ref(session.id(), None)
                .await
                .expect("session should delete");
            session.id().to_string()
        };
        let app = DaemonApp::bootstrap(delete_config).expect("daemon should reboot");
        assert!(app.sessions().get_session(&deleted_session_id).is_err());
        assert!(app
            .agents
            .get_session_agents(&deleted_session_id)
            .is_empty());
    }

    #[tokio::test]
    async fn rejects_session_commands_when_bounded_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_and_session_capacity(Arc::clone(&app), 1, 1);
        let app_guard = app.lock().await;

        let first_request = attach_request(&session_id, "cli-1");
        let first_result_rx = router
            .session_runtime
            .enqueue_for_test(&session_id, "cmd-1", "session.attach", first_request)
            .await
            .expect("first command should enter the session lane");

        let mut first_command_is_running = false;
        for _ in 0..50 {
            if router
                .session_runtime
                .lane_capacity(&session_id)
                .await
                .is_some_and(|capacity| capacity == 1)
            {
                first_command_is_running = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            first_command_is_running,
            "first session command should be running before filling the queue"
        );

        let queued_request = attach_request(&session_id, "queued-cli");
        let queued_result_rx = router
            .session_runtime
            .enqueue_for_test(&session_id, "cmd-queued", "session.attach", queued_request)
            .await
            .expect("queued command should fill the session lane");

        let mut session_lane_is_full = false;
        for _ in 0..50 {
            if router
                .session_runtime
                .lane_capacity(&session_id)
                .await
                .is_some_and(|capacity| capacity == 0)
            {
                session_lane_is_full = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            session_lane_is_full,
            "session command queue should be full before overflow dispatch"
        );

        let third_request = attach_request(&session_id, "cli-overflow");
        let third_command =
            KernelCommand::from_local_request("cmd-overflow", None, None, &third_request);
        let error = router
            .dispatch(third_command, third_request)
            .await
            .expect_err("overflow session command should be rejected while lane is full");
        assert!(error
            .to_string()
            .contains("session command lane overloaded"));

        drop(app_guard);
        let _ = first_result_rx.await.expect("first result should resolve");
        let _ = queued_result_rx
            .await
            .expect("queued result should resolve");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_submit_does_not_wait_behind_slow_history_load() {
        let mut config = DaemonConfig::for_tests();
        config.session_history_read_delay_ms = 120;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-slow-history",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.history_store()
            .append(
                &session,
                &crate::history::SessionHistoryEntry::user_prompt(
                    &session_id,
                    attachment.id(),
                    &agent_id,
                    "slow history entry",
                ),
            )
            .expect("legacy-only history should append");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-history-prompt-state",
            None,
            None,
            &state_request,
        );
        router
            .dispatch(state_command, state_request)
            .await
            .expect("state read should warm session projection");

        let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            round_count: Some(10),
            max_chars: None,
            before_entry_index: None,
            before_entry_char_offset: None,
        });
        let history_command = KernelCommand::from_local_request(
            "cmd-history-slow-background",
            None,
            None,
            &history_request,
        );
        let history_router = router.clone();
        let history_task = tokio::spawn(async move {
            history_router
                .dispatch(history_command, history_request)
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !history_task.is_finished(),
            "test setup should keep history loading in the background"
        );

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "submit while history is slow".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-during-history",
            None,
            None,
            &prompt_request,
        );
        let prompt_response = timeout(
            Duration::from_millis(75),
            router.dispatch(prompt_command, prompt_request),
        )
        .await
        .expect("prompt submit should not wait behind slow history")
        .expect("prompt submit should succeed");
        assert!(matches!(
            prompt_response,
            LocalDaemonResponse::PromptSubmitted { .. }
        ));

        let _ = history_task
            .await
            .expect("history task should join")
            .expect("history should eventually resolve");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn focus_resize_and_cancel_do_not_wait_behind_slow_provider_catalog() {
        let mut config = DaemonConfig::for_tests();
        config.provider_catalog_read_delay_ms = 120;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-slow-catalog",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "prompt to cancel while catalog is slow".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-catalog-prompt", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should start before catalog drill");

        let catalog_request = LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest);
        let catalog_command =
            KernelCommand::from_local_request("cmd-slow-catalog", None, None, &catalog_request);
        let catalog_router = router.clone();
        let catalog_task = tokio::spawn(async move {
            catalog_router
                .dispatch(catalog_command, catalog_request)
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !catalog_task.is_finished(),
            "test setup should keep provider catalog discovery in the background"
        );

        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command = KernelCommand::from_local_request(
            "cmd-focus-during-catalog",
            None,
            None,
            &focus_request,
        );
        let focus_response = timeout(
            Duration::from_millis(75),
            router.dispatch(focus_command, focus_request),
        )
        .await
        .expect("focus should not wait behind slow catalog")
        .expect("focus should succeed");
        assert!(matches!(
            focus_response,
            LocalDaemonResponse::AgentFocused { .. }
        ));

        let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 40,
        });
        let resize_command = KernelCommand::from_local_request(
            "cmd-resize-during-catalog",
            None,
            None,
            &resize_request,
        );
        let resize_response = timeout(
            Duration::from_millis(75),
            router.dispatch(resize_command, resize_request),
        )
        .await
        .expect("resize should not wait behind slow catalog")
        .expect("resize should succeed");
        assert!(matches!(
            resize_response,
            LocalDaemonResponse::TerminalResized { .. }
        ));

        let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let cancel_command = KernelCommand::from_local_request(
            "cmd-cancel-during-catalog",
            None,
            None,
            &cancel_request,
        );
        let cancel_response = timeout(
            Duration::from_millis(75),
            router.dispatch(cancel_command, cancel_request),
        )
        .await
        .expect("cancel should not wait behind slow catalog")
        .expect("cancel should succeed");
        assert!(matches!(
            cancel_response,
            LocalDaemonResponse::PromptCancelled { .. }
        ));

        let _ = catalog_task.await.expect("catalog task should join");
    }

    #[tokio::test]
    async fn stale_workflow_revision_rejects_graph_mutation_before_state_changes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-revision",
                "worktree-workflow-revision",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let first_agent = spawn_test_agent(&mut app, &session_id, "first", "dev-stub");
        let second_agent = spawn_test_agent(&mut app, &session_id, "second", "dev-stub");
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

        let create_workflow = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("revision-flow".to_string()),
        });
        let workflow = match router
            .dispatch(
                KernelCommand::from_local_request("create-workflow", None, None, &create_workflow),
                create_workflow,
            )
            .await
            .expect("workflow should be created")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            other => panic!("unexpected workflow response: {other:?}"),
        };
        assert_eq!(workflow.revision(), 0);

        let add_first = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow.id().to_string(),
            agent_id: first_agent.id().to_string(),
            expected_workflow_revision: Some(workflow.revision()),
        });
        let workflow = match router
            .dispatch(
                KernelCommand::from_local_request("add-first", None, None, &add_first),
                add_first,
            )
            .await
            .expect("first mutation should match revision")
        {
            LocalDaemonResponse::WorkflowNodeAdded { workflow, .. } => workflow,
            other => panic!("unexpected add response: {other:?}"),
        };
        assert_eq!(workflow.revision(), 1);

        let stale_add = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow.id().to_string(),
            agent_id: second_agent.id().to_string(),
            expected_workflow_revision: Some(0),
        });
        let rejected = router
            .dispatch(
                KernelCommand::from_local_request("stale-add", None, None, &stale_add),
                stale_add,
            )
            .await
            .expect_err("stale revision should reject before mutation");
        assert!(matches!(
            rejected,
            DaemonError::WorkflowRevisionConflict {
                expected_revision: 0,
                current_revision: 1,
                ..
            }
        ));

        let resolve = LocalDaemonRequest::ResolveWorkflow(ResolveWorkflowRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow.id().to_string(),
        });
        match router
            .dispatch(
                KernelCommand::from_local_request("resolve-after-stale", None, None, &resolve),
                resolve,
            )
            .await
            .expect("workflow should resolve")
        {
            LocalDaemonResponse::WorkflowResolved { workflow } => {
                assert_eq!(workflow.revision(), 1);
                assert_eq!(workflow.nodes().len(), 1);
                assert_eq!(workflow.nodes()[0].agent_id(), first_agent.id());
            }
            other => panic!("unexpected resolve response: {other:?}"),
        }

        let fresh_add = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id,
            workflow_ref: workflow.id().to_string(),
            agent_id: second_agent.id().to_string(),
            expected_workflow_revision: Some(workflow.revision()),
        });
        match router
            .dispatch(
                KernelCommand::from_local_request("fresh-add", None, None, &fresh_add),
                fresh_add,
            )
            .await
            .expect("fresh revision should succeed")
        {
            LocalDaemonResponse::WorkflowNodeAdded { workflow, .. } => {
                assert_eq!(workflow.revision(), 2);
                assert_eq!(workflow.nodes().len(), 2);
            }
            other => panic!("unexpected fresh add response: {other:?}"),
        }
    }

    fn attach_request(session_id: &str, client_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id: client_id.to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        })
    }

    fn focus_request(session_id: &str, agent_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }

    fn assert_ownership_denied(error: DaemonError, user_id: &str, owner_user_id: &str) {
        assert!(
            matches!(
                error,
                DaemonError::OwnershipAccessDenied {
                    user_id: ref denied_user,
                    owner_user_id: ref denied_owner,
                    ..
                } if denied_user == user_id && denied_owner == owner_user_id
            ),
            "unexpected error: {error:?}"
        );
    }
}
