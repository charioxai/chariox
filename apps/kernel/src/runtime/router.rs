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
    mod session_history_projection;
    mod session_read_projection;
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
    async fn session_runtime_publishes_attach_and_focus_projection_without_router_snapshot() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let second_agent = spawn_test_agent(&mut app, &session_id, "reviewer", "claude-code");
        assert_ne!(first_agent.id(), second_agent.id());

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let attach_request = attach_request(&session_id, "cli-session-projection");
        let attach_command = KernelCommand::from_local_request(
            "cmd-session-projection-attach",
            None,
            None,
            &attach_request,
        );
        let attachment_id = match router
            .dispatch(attach_command, attach_request)
            .await
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
            _ => panic!("unexpected attach response"),
        };

        let focus_request = focus_request(&session_id, second_agent.id());
        let focus_command = KernelCommand::from_local_request(
            "cmd-session-projection-focus",
            None,
            None,
            &focus_request,
        );
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should succeed");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-session-projection-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "session state should come from the SessionRuntime-published projection without taking the app lock"
        );
        drop(app_guard);

        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState {
                session,
                agent_activity,
            } => {
                assert!(session.has_attachment(&attachment_id));
                assert_eq!(session.focused_agent_id(), Some(second_agent.id()));
                assert!(agent_activity.contains_key(second_agent.id()));
            }
            _ => panic!("unexpected session state response"),
        }
    }

    #[tokio::test]
    async fn agent_lifecycle_refresh_uses_published_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("projected-agent".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        });
        let spawn_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-spawn",
            None,
            None,
            &spawn_request,
        );
        let spawned_agent_id = match router
            .dispatch(spawn_command, spawn_request)
            .await
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            _ => panic!("unexpected spawn response"),
        };
        assert!(
            router.session_runtime.has_lane(&session_id).await,
            "agent lifecycle should run through the session runtime lane"
        );

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-spawn-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
        let state_response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("spawn-projected state should not wait for the app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);
        match state_response {
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == spawned_agent_id));
            }
            _ => panic!("unexpected state response"),
        }

        let destroy_request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: session_id.clone(),
            agent_id: spawned_agent_id.clone(),
        });
        let destroy_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-destroy",
            None,
            None,
            &destroy_request,
        );
        router
            .dispatch(destroy_command, destroy_request)
            .await
            .expect("destroy should succeed");
        assert!(
            router.session_runtime.has_lane(&session_id).await,
            "destroying an agent should not bypass the session runtime lane"
        );

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-destroy-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
        let state_response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("destroy-projected state should not wait for the app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);
        match state_response {
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(!session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == spawned_agent_id));
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn end_session_uses_session_lane_and_removes_lane_registration() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let attach_request = attach_request(&session_id, "cli-1");
        let attach_command =
            KernelCommand::from_local_request("cmd-attach", None, None, &attach_request);
        router
            .dispatch(attach_command, attach_request)
            .await
            .expect("attach should create a session lane");
        assert!(router.session_runtime.has_lane(&session_id).await);

        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let end_command = KernelCommand::from_local_request("cmd-end", None, None, &end_request);
        let response = router
            .dispatch(end_command, end_request)
            .await
            .expect("end session should run through the session lane");

        assert!(matches!(
            response,
            crate::local::LocalDaemonResponse::SessionEnded { .. }
        ));
        assert!(
            !router.session_runtime.has_lane(&session_id).await,
            "ending a session should remove its mailbox registration"
        );
    }

    #[tokio::test]
    async fn delete_session_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let create_request = LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-delete-projection", "worktree")
                .with_alias("doomed"),
        );
        let create_command = KernelCommand::from_local_request(
            "cmd-delete-projection-create",
            None,
            None,
            &create_request,
        );
        let session_id = match router
            .dispatch(create_command, create_request)
            .await
            .expect("create should warm session projection")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
            _ => panic!("unexpected create response"),
        };

        let app_guard = app.lock().await;
        let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "doomed".to_string(),
            workspace_id: Some("workspace-delete-projection".to_string()),
        });
        let delete_command =
            KernelCommand::from_local_request("cmd-delete-projection", None, None, &delete_request);
        let delete_router = router.clone();
        let delete_task =
            tokio::spawn(
                async move { delete_router.dispatch(delete_command, delete_request).await },
            );

        let delete_response = timeout(Duration::from_millis(100), delete_task)
            .await
            .expect("owned delete should not wait for the app lock")
            .expect("delete task should join")
            .expect("delete should succeed");
        drop(app_guard);
        assert!(matches!(
            delete_response,
            LocalDaemonResponse::SessionDeleted { .. }
        ));
        assert!(
            !router.session_runtime.has_lane(&session_id).await,
            "deleting a session should remove its mailbox registration"
        );
    }

    #[tokio::test]
    async fn missing_delete_session_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "missing-session".to_string(),
            workspace_id: None,
        });
        let delete_command =
            KernelCommand::from_local_request("cmd-delete-missing", None, None, &delete_request);
        let delete_router = router.clone();
        let delete_task =
            tokio::spawn(
                async move { delete_router.dispatch(delete_command, delete_request).await },
            );

        let error = timeout(Duration::from_millis(100), delete_task)
            .await
            .expect("missing delete should not wait for the app lock")
            .expect("delete task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_detach_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-warm-detach", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let detach_request = LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
            attachment_id: "missing-attachment".to_string(),
        });
        let detach_command =
            KernelCommand::from_local_request("cmd-detach-missing", None, None, &detach_request);
        let detach_router = router.clone();
        let detach_task =
            tokio::spawn(
                async move { detach_router.dispatch(detach_command, detach_request).await },
            );

        let error = timeout(Duration::from_millis(100), detach_task)
            .await
            .expect("missing detach should not wait for the app lock")
            .expect("detach task should join")
            .expect_err("missing attachment should fail");
        drop(app_guard);

        match error {
            DaemonError::AttachmentNotFound { attachment_id } => {
                assert_eq!(attachment_id, "missing-attachment");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_attach_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-attach-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let attach_request = attach_request("missing-session", "cli-missing-session");
        let attach_command =
            KernelCommand::from_local_request("cmd-attach-missing", None, None, &attach_request);
        let attach_router = router.clone();
        let attach_task =
            tokio::spawn(
                async move { attach_router.dispatch(attach_command, attach_request).await },
            );

        let error = timeout(Duration::from_millis(100), attach_task)
            .await
            .expect("missing attach should not wait for the app lock")
            .expect("attach task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_alias_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-alias-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let alias_request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: "missing-session".to_string(),
            alias: "review".to_string(),
        });
        let alias_command =
            KernelCommand::from_local_request("cmd-alias-missing", None, None, &alias_request);
        let alias_router = router.clone();
        let alias_task =
            tokio::spawn(async move { alias_router.dispatch(alias_command, alias_request).await });

        let error = timeout(Duration::from_millis(100), alias_task)
            .await
            .expect("missing alias should not wait for the app lock")
            .expect("alias task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_end_session_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-end-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: "missing-session".to_string(),
        });
        let end_command =
            KernelCommand::from_local_request("cmd-end-missing", None, None, &end_request);
        let end_router = router.clone();
        let end_task =
            tokio::spawn(async move { end_router.dispatch(end_command, end_request).await });

        let error = timeout(Duration::from_millis(100), end_task)
            .await
            .expect("missing end should not wait for the app lock")
            .expect("end task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn invalid_focus_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-focus-invalid-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let focus_request = focus_request(&session_id, "missing-agent");
        let focus_command =
            KernelCommand::from_local_request("cmd-focus-invalid", None, None, &focus_request);
        let focus_router = router.clone();
        let focus_task =
            tokio::spawn(async move { focus_router.dispatch(focus_command, focus_request).await });

        let error = timeout(Duration::from_millis(100), focus_task)
            .await
            .expect("invalid focus should not wait for the app lock")
            .expect("focus task should join")
            .expect_err("missing agent should fail");
        drop(app_guard);

        match error {
            DaemonError::AgentNotInSession {
                session_id: error_session_id,
                agent_id,
            } => {
                assert_eq!(error_session_id, session_id);
                assert_eq!(agent_id, "missing-agent");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_cycle_focus_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-cycle-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let cycle_request = LocalDaemonRequest::CycleAgentFocus(CycleAgentFocusRequest {
            session_id: "missing-session".to_string(),
        });
        let cycle_command =
            KernelCommand::from_local_request("cmd-cycle-missing", None, None, &cycle_request);
        let cycle_router = router.clone();
        let cycle_task =
            tokio::spawn(async move { cycle_router.dispatch(cycle_command, cycle_request).await });

        let error = timeout(Duration::from_millis(100), cycle_task)
            .await
            .expect("missing cycle focus should not wait for the app lock")
            .expect("cycle task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn daemon_health_projection_reports_session_and_agent_mailboxes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
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

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command =
            KernelCommand::from_local_request("cmd-focus", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should create a session lane");

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from health projection test".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");

        let workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("health-workflow".to_string()),
        });
        let workflow_command =
            KernelCommand::from_local_request("cmd-workflow", None, None, &workflow_request);
        router
            .dispatch(workflow_command, workflow_request)
            .await
            .expect("workflow command should create a workflow lane");

        let shell_request = LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            command: "/bin/true".to_string(),
            args: Vec::new(),
            working_directory: None,
            timeout_ms: Some(1_000),
        });
        let shell_command =
            KernelCommand::from_local_request("cmd-capability", None, None, &shell_request);
        router
            .dispatch(shell_command, shell_request)
            .await
            .expect_err(
                "capability command should report executor failure for missing test worktree",
            );

        let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let health_command =
            KernelCommand::from_local_request("cmd-health", None, None, &health_request);
        let health_response = router
            .dispatch(health_command, health_request)
            .await
            .expect("health projection should be returned");
        let projection = match health_response {
            LocalDaemonResponse::DaemonHealth { projection } => projection,
            _ => panic!("unexpected health response"),
        };
        assert!(projection
            .session_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
        assert!(projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id && lane.queue_limit == 128));
        assert!(projection
            .workflow_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
        assert_eq!(projection.session_projection.projected_sessions, 1);
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert_eq!(projection.session_projection.queued_prompts, 0);
        assert_eq!(projection.agent_runtime_projection.projected_agents, 1);
        assert_eq!(projection.agent_runtime_projection.active_prompts, 1);
        assert_eq!(projection.agent_runtime_projection.queued_prompts, 0);
        assert_eq!(projection.capability_executor.max_concurrent_jobs, 64);
        assert_eq!(projection.capability_executor.available_permits, 64);
        assert_eq!(projection.capability_executor.submitted_jobs, 1);
        assert_eq!(projection.capability_executor.completed_jobs, 0);
        assert_eq!(projection.capability_executor.failed_jobs, 1);
        assert_eq!(projection.capability_executor.rejected_jobs, 0);
        assert!(!projection.provider_catalog.cached);
    }

    #[tokio::test]
    async fn daemon_health_reads_terminal_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let health_command =
            KernelCommand::from_local_request("cmd-health-no-lock", None, None, &health_request);
        let health_router = router.clone();
        let health_task =
            tokio::spawn(
                async move { health_router.dispatch(health_command, health_request).await },
            );

        let response = timeout(Duration::from_millis(100), health_task)
            .await
            .expect("daemon health should not wait for the app lock")
            .expect("health task should join")
            .expect("health should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::DaemonHealth { projection } => {
                assert_eq!(projection.terminal_stream.pending_output_records, 0);
            }
            _ => panic!("unexpected health response"),
        }
    }

    #[tokio::test]
    async fn relay_status_uses_config_projection_without_app_lock() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:9".to_string());
        config.relay_token = Some("secret".to_string());
        config.host_machine_id = "machine-projected".to_string();
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let relay_request = LocalDaemonRequest::RelayStatus(RelayStatusRequest);
        let relay_command = KernelCommand::from_local_request(
            "cmd-relay-status-projection",
            None,
            None,
            &relay_request,
        );
        let relay_router = router.clone();
        let relay_task =
            tokio::spawn(async move { relay_router.dispatch(relay_command, relay_request).await });

        let response = timeout(Duration::from_millis(100), relay_task)
            .await
            .expect("relay status should not wait for the app lock")
            .expect("relay task should join")
            .expect("relay status should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::RelayStatus { status } => {
                assert!(status.configured);
                assert_eq!(status.relay_url.as_deref(), Some("ws://127.0.0.1:9"));
                assert!(status.relay_token_configured);
                assert_eq!(status.machine_id, "machine-projected");
            }
            _ => panic!("unexpected relay response"),
        }
    }

    #[tokio::test]
    async fn provider_command_catalogs_do_not_wait_for_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let catalog_request =
            LocalDaemonRequest::GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest);
        let catalog_command = KernelCommand::from_local_request(
            "cmd-provider-command-catalog-projection",
            None,
            None,
            &catalog_request,
        );
        let catalog_router = router.clone();
        let catalog_task = tokio::spawn(async move {
            catalog_router
                .dispatch(catalog_command, catalog_request)
                .await
        });

        let response = timeout(Duration::from_millis(100), catalog_task)
            .await
            .expect("provider command catalogs should not wait for the app lock")
            .expect("catalog task should join")
            .expect("provider command catalogs should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::ProviderCommandCatalogs { catalogs } => {
                assert!(!catalogs.is_empty());
            }
            _ => panic!("unexpected provider command catalog response"),
        }
    }

    #[tokio::test]
    async fn provider_auth_status_does_not_use_generic_app_fallback() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let auth_request =
            LocalDaemonRequest::GetProviderAuthStatus(GetProviderAuthStatusRequest {
                provider: "unsupported-provider".to_string(),
            });
        let auth_command = KernelCommand::from_local_request(
            "cmd-provider-auth-no-fallback",
            None,
            None,
            &auth_request,
        );
        let auth_router = router.clone();
        let auth_task =
            tokio::spawn(async move { auth_router.dispatch(auth_command, auth_request).await });

        let error = timeout(Duration::from_millis(100), auth_task)
            .await
            .expect("provider auth status should not wait for the app lock")
            .expect("auth task should join")
            .expect_err("unsupported provider should be rejected");
        drop(app_guard);

        match error {
            DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "get_provider_auth_status");
                assert!(message.contains("unsupported-provider"));
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn agent_and_workflow_lanes_are_removed_when_session_ends() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-agent-lane-cleanup",
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

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "create agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-agent-lane-create", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");
        assert!(router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));
        let workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("cleanup-workflow".to_string()),
        });
        let workflow_command = KernelCommand::from_local_request(
            "cmd-workflow-lane-create",
            None,
            None,
            &workflow_request,
        );
        router
            .dispatch(workflow_command, workflow_request)
            .await
            .expect("workflow command should create a workflow lane");
        assert!(router.workflow_runtime.has_lane(&session_id).await);

        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let end_command =
            KernelCommand::from_local_request("cmd-agent-lane-end", None, None, &end_request);
        router
            .dispatch(end_command, end_request)
            .await
            .expect("ending session should clean up agent lane");

        assert!(!router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));
        assert!(!router.workflow_runtime.has_lane(&session_id).await);
    }

    #[tokio::test]
    async fn agent_lane_is_removed_when_agent_is_destroyed() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-agent-destroy-lane-cleanup",
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

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "create agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-agent-destroy-lane-create",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");
        assert!(router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));

        let destroy_request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id,
            agent_id: agent_id.clone(),
        });
        let destroy_command = KernelCommand::from_local_request(
            "cmd-agent-destroy-lane-cleanup",
            None,
            None,
            &destroy_request,
        );
        router
            .dispatch(destroy_command, destroy_request)
            .await
            .expect("destroying agent should clean up agent lane");

        assert!(!router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));
    }

    #[tokio::test]
    async fn prompt_submit_uses_agent_lane_without_generic_interactive_lane() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
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
        let app_guard = app.lock().await;

        let first_request = focus_request(&session_id, &agent_id);
        let first_command =
            KernelCommand::from_local_request("cmd-focus-1", None, None, &first_request);
        let first_router = router.clone();
        let first_task =
            tokio::spawn(async move { first_router.dispatch(first_command, first_request).await });

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let prompt_response = timeout(Duration::from_millis(100), prompt_task)
            .await
            .expect("owned prompt submit should not wait for the app lock")
            .expect("prompt task should join")
            .expect("prompt should submit");
        drop(app_guard);
        let _ = first_task.await.expect("first focus should join");
        match prompt_response {
            crate::local::LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), agent_id);
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn prompt_submit_uses_session_focus_projection_without_app_lock_for_routing() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let focused_agent = spawn_test_agent(&mut app, &session_id, "focused", "claude-code");
        launch_test_provider(
            &mut app,
            &session_id,
            focused_agent.id(),
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let focus_request = focus_request(&session_id, focused_agent.id());
        let focus_command =
            KernelCommand::from_local_request("cmd-focus-projection", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should populate the projection");

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello through projected focus".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt-projection", None, None, &prompt_request);
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let prompt_response = timeout(Duration::from_millis(100), prompt_task)
            .await
            .expect("owned prompt submit should not wait for the app lock")
            .expect("prompt task should join")
            .expect("prompt should submit");
        drop(app_guard);
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), focused_agent.id());
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn prompt_submit_uses_warmed_session_projection_without_app_lock_for_focus_fallback() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-session-projection-focus",
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
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-focus-fallback-warm",
            None,
            None,
            &state_request,
        );
        router
            .dispatch(state_command, state_request)
            .await
            .expect("state read should warm the session projection");

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello through warmed session projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-session-projection-focus",
            None,
            None,
            &prompt_request,
        );
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let prompt_response = timeout(Duration::from_millis(100), prompt_task)
            .await
            .expect("owned prompt submit should not wait for the app lock")
            .expect("prompt task should join")
            .expect("prompt should submit");
        drop(app_guard);
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), agent_id);
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn agent_spawn_refreshes_focus_projection_for_followup_prompt_routing() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("spawned".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        });
        let spawn_command =
            KernelCommand::from_local_request("cmd-spawn-projection", None, None, &spawn_request);
        let spawned_agent = match router
            .dispatch(spawn_command, spawn_request)
            .await
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };

        {
            let mut app = app.lock().await;
            launch_test_provider(
                &mut app,
                &session_id,
                spawned_agent.id(),
                "dev-stub",
                "claude-code",
                "sonnet",
            );
        }

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello after spawn".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-after-spawn",
            None,
            None,
            &prompt_request,
        );
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let mut spawned_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == spawned_agent.id())
            {
                spawned_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            spawned_agent_lane_created,
            "spawn should refresh focused-agent projection before followup prompt routing"
        );

        drop(app_guard);
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), spawned_agent.id());
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_uses_projection_after_prompt_submit_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
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
            prompt: "warm session projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt-state", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm the session projection");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command =
            KernelCommand::from_local_request("cmd-state-projection", None, None, &state_request);
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "warm GetSessionState should be served from the session projection without app lock access"
        );

        drop(app_guard);
        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(session.active_prompt_for_agent(&agent_id).is_some());
                assert_eq!(session.agents().len(), 1);
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_keeps_activity_after_runtime_interaction_projection_refresh() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let provider_run = launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        router.active_turns.start(crate::app::ActiveTurnState::new(
            session_id.clone(),
            agent_id.clone(),
            "prompt-1".to_string(),
            provider_run.id().to_string(),
        ));
        let interaction = RuntimeInteraction::new(
            "interaction-1",
            &agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Info,
            Some("Approve file changes?".to_string()),
            "Approve file changes?",
            vec![RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow",
                Some(RuntimeInteractionChoiceStyle::Primary),
            )],
            None,
            None,
            None,
        );
        let _resolution = router
            .runtime_state
            .create_runtime_interaction(&session_id, interaction)
            .await
            .expect("interaction should register");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command =
            KernelCommand::from_local_request("cmd-state-interaction", None, None, &state_request);
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "warm GetSessionState should be served from the session projection without app lock access"
        );

        drop(app_guard);
        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState {
                session,
                agent_activity,
            } => {
                assert_eq!(session.focused_agent_id(), Some(agent_id.as_str()));
                assert_eq!(session.agents().len(), 1);
                assert_eq!(session.active_interactions().len(), 1);
                let activity = agent_activity
                    .get(&agent_id)
                    .expect("agent activity should include focused agent");
                assert!(
                    activity.busy,
                    "active turn must keep focused agent working during permission popup"
                );
                assert!(
                    activity.active_turn.is_some(),
                    "active turn projection must survive interaction projection refresh"
                );
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn update_session_config_uses_session_runtime_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-config-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let update_request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
            requires_idle: false,
        });
        let update_command =
            KernelCommand::from_local_request("cmd-session-config", None, None, &update_request);
        let update_response = router
            .dispatch(update_command, update_request)
            .await
            .expect("session config update should succeed");
        match update_response {
            LocalDaemonResponse::SessionConfigUpdated { config, session } => {
                assert_eq!(config.version(), 1);
                assert_eq!(session.config_state().version(), 1);
                assert_eq!(
                    session.config_state().values().get("theme"),
                    Some(&"compact".to_string())
                );
            }
            _ => panic!("unexpected config response"),
        }

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-session-config-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "session config update should publish a session projection for lock-free state reads"
        );

        drop(app_guard);
        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session, .. } => {
                assert_eq!(session.config_state().version(), 1);
                assert_eq!(
                    session.config_state().values().get("theme"),
                    Some(&"compact".to_string())
                );
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn alias_session_uses_session_runtime_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let alias_request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session_id.clone(),
            alias: "review entry".to_string(),
        });
        let alias_command =
            KernelCommand::from_local_request("cmd-session-alias", None, None, &alias_request);
        let alias_response = router
            .dispatch(alias_command, alias_request)
            .await
            .expect("session alias should succeed");
        match alias_response {
            LocalDaemonResponse::SessionAliased { session } => {
                assert_eq!(session.alias(), Some("review_entry"));
            }
            _ => panic!("unexpected alias response"),
        }

        let app_guard = app.lock().await;
        let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "review_entry".to_string(),
            workspace_id: Some("workspace".to_string()),
        });
        let resolve_command = KernelCommand::from_local_request(
            "cmd-session-alias-resolve",
            None,
            None,
            &resolve_request,
        );
        let resolve_router = router.clone();
        let resolve_task = tokio::spawn(async move {
            resolve_router
                .dispatch(resolve_command, resolve_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            resolve_task.is_finished(),
            "session alias should publish a projection that resolves without app lock access"
        );

        drop(app_guard);
        let resolve_response = resolve_task
            .await
            .expect("resolve task should join")
            .expect("resolve should succeed");
        match resolve_response {
            LocalDaemonResponse::SessionResolved { session } => {
                assert_eq!(session.id(), session_id);
                assert_eq!(session.alias(), Some("review_entry"));
            }
            _ => panic!("unexpected resolve response"),
        }
    }

    #[tokio::test]
    async fn poll_runtime_notices_routes_through_session_runtime() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let source = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-notice-source",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let recipient = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-notice-recipient",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("recipient attachment should attach");
        app.record_notice(
            &session_id,
            None,
            vec![recipient.id().to_string()],
            format!(
                "Attachment `{}` updated configuration for session `{}`.",
                source.id(),
                session_id
            ),
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-runtime-notices-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm session projection");

        let app_guard = app.lock().await;
        let poll_request = LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
            session_id: session_id.clone(),
            attachment_id: recipient.id().to_string(),
        });
        let poll_command =
            KernelCommand::from_local_request("cmd-runtime-notices", None, None, &poll_request);
        let poll_router = router.clone();
        let poll_task =
            tokio::spawn(async move { poll_router.dispatch(poll_command, poll_request).await });
        let poll_response = timeout(Duration::from_millis(100), poll_task)
            .await
            .expect("notice poll should not wait for the app lock")
            .expect("poll task should join")
            .expect("notice poll should succeed");
        drop(app_guard);

        assert!(
            router.session_runtime.has_lane(&session_id).await,
            "notice polling should be admitted through the per-session runtime lane"
        );
        match poll_response {
            LocalDaemonResponse::RuntimeNotices { notices } => {
                assert_eq!(notices.len(), 1);
                assert_eq!(notices[0].session_id, session_id);
            }
            _ => panic!("unexpected notice response"),
        }
    }

    #[tokio::test]
    async fn resize_without_active_run_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-resize-no-active-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm session projection");

        let app_guard = app.lock().await;
        let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 40,
        });
        let resize_command = KernelCommand::from_local_request(
            "cmd-resize-no-active-projection",
            None,
            None,
            &resize_request,
        );
        let resize_router = router.clone();
        let resize_task =
            tokio::spawn(
                async move { resize_router.dispatch(resize_command, resize_request).await },
            );

        let error = timeout(Duration::from_millis(100), resize_task)
            .await
            .expect("resize absence should not wait for the app lock")
            .expect("resize task should join")
            .expect_err("resize without active provider run should fail");
        drop(app_guard);

        match error {
            DaemonError::NoActiveProviderRun {
                session_id: error_session_id,
            } => assert_eq!(error_session_id, session_id),
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn get_session_state_projection_tracks_prompt_completion_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-complete-projection",
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
            prompt: "complete projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-complete-state",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent runtime projection should track prompt state after submit");
        assert!(prompt_projection.active_prompt.is_some());
        assert_eq!(prompt_projection.queued_prompt_count, 0);

        let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session_id.clone(),
        });
        let complete_command = KernelCommand::from_local_request(
            "cmd-complete-state-projection",
            None,
            None,
            &complete_request,
        );
        router
            .dispatch(complete_command, complete_request)
            .await
            .expect("prompt completion should publish session projection through agent runtime");
        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent runtime projection should retain prompt state after complete");
        assert!(prompt_projection.active_prompt.is_none());
        assert_eq!(prompt_projection.queued_prompt_count, 0);

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-state-complete-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "completed prompt state should be served from projection without app lock access"
        );
        drop(app_guard);

        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(session.active_prompt_for_agent(&agent_id).is_none());
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn session_snapshot_refresh_tracks_agent_runtime_projection() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-prompt-shadow-refresh",
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
            prompt: "shadow refresh".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-shadow-submit", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm agent runtime projection");
        assert!(router
            .agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

        {
            let app = app.lock().await;
            app.sessions_mut()
                .complete_active_prompt_only(&session_id, &agent_id)
                .expect("compatibility state should be externally settled");
        }
        assert!(
            router
                .agent_runtime_projection
                .get(&agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some(),
            "prompt projection should stay stale until a session snapshot is observed"
        );

        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let pump_command =
            KernelCommand::from_local_request("cmd-shadow-refresh", None, None, &pump_request);
        router
            .dispatch(pump_command, pump_request)
            .await
            .expect("snapshot-producing pump should refresh projections");

        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent prompt projection should remain registered");
        assert!(prompt_projection.active_prompt.is_none());
        assert_eq!(prompt_projection.queued_prompt_count, 0);
    }

    #[tokio::test]
    async fn prompt_complete_uses_agent_runtime_projection_when_session_projection_is_stale() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let default_agent_id = default_agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-complete-owner-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
        let spawned_agent_id = spawned_agent.id().to_string();
        launch_test_provider(
            &mut app,
            &session_id,
            &spawned_agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );
        focus_test_agent(&mut app, &session_id, &default_agent_id);
        let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("idle session snapshot should be available");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(spawned_agent_id.clone()),
            prompt: "complete owner projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-complete-owner",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        router.session_projection.update(idle_session_snapshot);

        let app_guard = app.lock().await;
        let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session_id.clone(),
        });
        let complete_command = KernelCommand::from_local_request(
            "cmd-complete-owner-projection",
            None,
            None,
            &complete_request,
        );
        let complete_router = router.clone();
        let complete_task = tokio::spawn(async move {
            complete_router
                .dispatch(complete_command, complete_request)
                .await
        });

        let mut spawned_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == spawned_agent_id)
            {
                spawned_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            spawned_agent_lane_created,
            "prompt complete should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
        );
        assert!(
            !complete_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let complete_response = complete_task
            .await
            .expect("complete task should join")
            .expect("prompt should complete");
        match complete_response {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected complete response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_projection_tracks_prompt_cancellation_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-cancel-projection",
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
            prompt: "cancel projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-cancel-state",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        assert!(router
            .agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

        let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let cancel_command = KernelCommand::from_local_request(
            "cmd-cancel-state-projection",
            None,
            None,
            &cancel_request,
        );
        router
            .dispatch(cancel_command, cancel_request)
            .await
            .expect("prompt cancellation should publish session projection");
        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent runtime projection should retain prompt state after cancel");
        assert_eq!(
            prompt_projection
                .active_prompt
                .as_ref()
                .map(|prompt| prompt.status()),
            Some(PromptStatus::Cancelling)
        );

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-state-cancel-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "cancelled prompt state should be served from projection without app lock access"
        );
        drop(app_guard);

        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session, .. } => {
                let active_prompt = session
                    .active_prompt_for_agent(&agent_id)
                    .expect("prompt should still be settling");
                assert_eq!(active_prompt.status(), PromptStatus::Cancelling);
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn prompt_cancel_uses_agent_runtime_projection_when_session_projection_is_stale() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let default_agent_id = default_agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-cancel-owner-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
        let spawned_agent_id = spawned_agent.id().to_string();
        launch_test_provider(
            &mut app,
            &session_id,
            &spawned_agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );
        focus_test_agent(&mut app, &session_id, &default_agent_id);
        let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("idle session snapshot should be available");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(spawned_agent_id.clone()),
            prompt: "cancel owner projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-cancel-owner",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        router.session_projection.update(idle_session_snapshot);

        let app_guard = app.lock().await;
        let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let cancel_command = KernelCommand::from_local_request(
            "cmd-cancel-owner-projection",
            None,
            None,
            &cancel_request,
        );
        let cancel_router = router.clone();
        let cancel_task =
            tokio::spawn(
                async move { cancel_router.dispatch(cancel_command, cancel_request).await },
            );

        let mut spawned_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == spawned_agent_id)
            {
                spawned_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            spawned_agent_lane_created,
            "prompt cancel should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
        );
        assert!(
            !cancel_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let cancel_response = cancel_task
            .await
            .expect("cancel task should join")
            .expect("prompt should cancel");
        match cancel_response {
            LocalDaemonResponse::PromptCancelled { cancellation } => {
                assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
            }
            _ => panic!("unexpected cancel response"),
        }
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
