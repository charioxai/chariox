use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::Mutex;

use crate::agent::AgentInstance;
use crate::provider::ProviderRunState;
use crate::runtime::command::{KernelCaller, KernelCommand};
use crate::runtime::router::CommandRouter;
use crate::session::{RuntimeSession, WorkflowNodeDefinition, WorkflowRun, WorkflowRunStatus};
use crate::terminal::TerminalOutputKind;
use crate::{DaemonApp, DaemonConfig, DaemonError};

use super::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, CompletePromptRequest, FocusAgentRequest,
    GetWorkflowRunRequest, LaunchProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse,
    SpawnAgentRequest,
};

static LOCAL_ROUTER_TEST_COMMAND_ID: AtomicU64 = AtomicU64::new(1);
const LOCAL_ROUTER_TEST_RUNTIME_THREAD_STACK_SIZE: usize = 32 * 1024 * 1024;

/// Router-backed in-process fixture for daemon unit tests.
///
/// All local API requests must enter through `dispatch`, which builds a
/// `KernelCommand` and exercises `CommandRouter`. Direct app access is kept only
/// for non-request state setup/inspection that has no local command boundary yet.
pub(crate) struct LocalRouterTestHarness {
    runtime: Runtime,
    app: Arc<Mutex<DaemonApp>>,
    router: CommandRouter,
}

impl LocalRouterTestHarness {
    pub(crate) fn new() -> Self {
        Self::with_config(DaemonConfig::for_tests())
    }

    pub(crate) fn with_config(config: DaemonConfig) -> Self {
        Self::with_config_and_aegs_management_http_client(
            config,
            crate::runtime::event_catalog_control::AegsManagementHttpClient::default(),
        )
    }

    pub(crate) fn with_config_and_aegs_management_http_client(
        config: DaemonConfig,
        management_client: crate::runtime::event_catalog_control::AegsManagementHttpClient,
    ) -> Self {
        let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let provider_runtime_lanes = app.provider_run_operation_lanes();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity_and_provider_lanes(
            Arc::clone(&app),
            16,
            provider_runtime_lanes,
        )
        .with_aegs_management_http_client(management_client);
        Self {
            runtime: Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(LOCAL_ROUTER_TEST_RUNTIME_THREAD_STACK_SIZE)
                .build()
                .expect("test runtime should start"),
            app,
            router,
        }
    }

    pub(crate) fn dispatch(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.dispatch_with_caller(request, KernelCaller::default())
    }

    pub(crate) fn dispatch_runtime_tool(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime.block_on(
            self.router
                .dispatch_authenticated_runtime_tool_call(auth_token, tool_name, arguments),
        )
    }

    pub(crate) fn dispatch_as_user(
        &self,
        user_id: &str,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut caller = KernelCaller::default();
        caller.user_id = Some(user_id.to_string());
        self.dispatch_with_caller(request, caller)
    }

    pub(crate) fn dispatch_with_caller(
        &self,
        request: LocalDaemonRequest,
        caller: KernelCaller,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let command_id = format!(
            "local-router-test-{}",
            LOCAL_ROUTER_TEST_COMMAND_ID.fetch_add(1, Ordering::SeqCst)
        );
        let command = KernelCommand::from_local_request_with_caller(
            &command_id,
            crate::runtime::command::KernelCommandSource::LocalCli,
            caller,
            None,
            None,
            &request,
        );
        let router = self.router.clone();
        self.runtime.block_on(async move {
            tokio::spawn(async move { router.dispatch(command, request).await })
                .await
                .expect("test dispatch task should join")
        })
    }

    pub(crate) fn with_app<R>(&self, f: impl FnOnce(&DaemonApp) -> R) -> R {
        let app = self.runtime.block_on(self.app.lock());
        f(&app)
    }

    pub(crate) fn with_app_mut<R>(&self, f: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.runtime.block_on(self.app.lock());
        f(&mut app)
    }

    pub(crate) fn runtime_state(&self) -> crate::runtime::state::KernelRuntimeState {
        self.router.runtime_state()
    }

    pub(crate) fn pump_transport_runtime(&self) {
        self.runtime.block_on(self.router.pump_transport_runtime());
    }

    pub(crate) fn reconcile_pending_event_connections(
        &self,
    ) -> crate::runtime::router::event_connection_lifecycle::EventConnectionReconciliationSummary
    {
        self.runtime
            .block_on(self.router.reconcile_pending_event_connections())
            .expect("event connection reconciliation should succeed")
    }

    pub(crate) fn transport_runtime_pump_interval_ms(
        &self,
        active_interval_ms: u64,
        idle_interval_ms: u64,
        now_ms: u64,
    ) -> u64 {
        self.router
            .transport_runtime_pump_interval_ms(active_interval_ms, idle_interval_ms, now_ms)
    }

    pub(crate) fn spawn_workflow_test_agent(&self, session_id: &str, alias: &str) -> AgentInstance {
        self.spawn_workflow_test_agent_with_worktree(session_id, alias, None)
    }

    pub(crate) fn spawn_workflow_test_agent_with_worktree(
        &self,
        session_id: &str,
        alias: &str,
        worktree_id: Option<&str>,
    ) -> AgentInstance {
        match self
            .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                account_profile: None,
                session_id: session_id.to_string(),
                alias: Some(alias.to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("workflow-test-idle".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: worktree_id.map(str::to_string),
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            }))
            .expect("workflow test agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        }
    }

    pub(crate) fn launch_workflow_test_provider(&self, session_id: &str, agent_id: &str) {
        match self
            .dispatch(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session_id.to_string(),
                    agent_id: Some(agent_id.to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "dev-stub".to_string(),
                    account_profile: "default".to_string(),
                    model: "workflow-test-idle".to_string(),
                    variant: None,
                    structured_endpoint: None,
                    provider_session_id: None,
                    native_tui: false,
                },
            ))
            .expect("workflow test provider should launch")
        {
            LocalDaemonResponse::ProviderRunLaunched { .. }
            | LocalDaemonResponse::ProviderRunLaunchAccepted { .. } => {}
            _ => panic!("unexpected local response"),
        }
        self.runtime.block_on(async {
            for _ in 0..500 {
                let running = {
                    let app = self.app.lock().await;
                    app.providers()
                        .get_run_for_agent(session_id, agent_id)
                        .is_some_and(|run| run.state() == ProviderRunState::Running)
                };
                if running {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("workflow test provider for agent `{agent_id}` should become running");
        });
    }

    pub(crate) fn add_workflow_test_node(
        &self,
        session_id: &str,
        workflow_id: &str,
        agent_id: &str,
    ) -> WorkflowNodeDefinition {
        match self
            .dispatch(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session_id.to_string(),
                    workflow_ref: workflow_id.to_string(),
                    agent_id: agent_id.to_string(),
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow test node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        }
    }

    pub(crate) fn add_workflow_test_edge(
        &self,
        session_id: &str,
        workflow_id: &str,
        from_node_id: &str,
        to_node_id: &str,
    ) {
        match self
            .dispatch(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session_id.to_string(),
                    workflow_ref: workflow_id.to_string(),
                    from_node_id: from_node_id.to_string(),
                    to_node_id: to_node_id.to_string(),
                    handoff_schema_ref: None,
                    validation_policy: None,
                    expected_workflow_revision: None,
                    source_side: None,
                    target_side: None,
                },
            ))
            .expect("workflow test edge should be added")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
            _ => panic!("unexpected local response"),
        }
    }

    pub(crate) fn fan_out_workflow_test_output(&self, session_id: &str, label: &str) {
        let provider_run_id = self.wait_for_active_provider_run(session_id);
        self.fan_out_workflow_test_output_to_provider(session_id, &provider_run_id, label);
    }

    pub(crate) fn fan_out_workflow_test_output_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        label: &str,
    ) {
        let provider_run_id = self.with_app(|app| {
            app.providers()
                .get_run_for_agent(session_id, agent_id)
                .unwrap_or_else(|| {
                    panic!("provider run should exist for workflow test agent `{agent_id}`")
                })
                .id()
                .to_string()
        });
        self.fan_out_workflow_test_output_to_provider(session_id, &provider_run_id, label);
    }

    fn fan_out_workflow_test_output_to_provider(
        &self,
        session_id: &str,
        provider_run_id: &str,
        label: &str,
    ) {
        let payload = serde_json::json!({
            "summary": format!("{label} completed"),
            "output": {
                "message": format!("{label} output"),
            },
        });
        let output = format!(
            "```json\n{}\n```\n",
            serde_json::to_string(&payload).expect("workflow test output should serialize")
        );
        self.with_app_mut(|app| {
            app.fan_out_output(
                session_id,
                provider_run_id,
                TerminalOutputKind::ProviderOutput,
                None,
                Vec::new(),
                output.as_bytes(),
            );
        });
    }

    pub(crate) fn complete_workflow_test_prompt(&self, session_id: &str, label: &str) {
        self.fan_out_workflow_test_output(session_id, label);
        match self.dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session_id.to_string(),
        })) {
            Ok(LocalDaemonResponse::PromptCompleted { .. }) => {}
            Ok(_) => panic!("unexpected local response"),
            Err(DaemonError::NoActivePrompt {
                session_id: error_session_id,
            }) if error_session_id == session_id
                && self.session_has_no_active_workflow(session_id) => {}
            Err(error) => panic!("{label} should complete: {error}"),
        }
    }

    pub(crate) fn complete_workflow_test_prompt_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        label: &str,
    ) {
        match self
            .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            }))
            .unwrap_or_else(|error| panic!("{label} agent focus should succeed: {error}"))
        {
            LocalDaemonResponse::AgentFocused { .. } => {}
            _ => panic!("unexpected local response"),
        }
        self.fan_out_workflow_test_output_for_agent(session_id, agent_id, label);
        match self
            .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session_id.to_string(),
            }))
            .unwrap_or_else(|error| panic!("{label} should complete: {error}"))
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected local response"),
        }
    }

    pub(crate) fn get_workflow_test_run(
        &self,
        session_id: &str,
        workflow_run_id: &str,
    ) -> WorkflowRun {
        match self
            .dispatch(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session_id.to_string(),
                workflow_run_ref: workflow_run_id.to_string(),
            }))
            .expect("workflow test run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        }
    }

    pub(crate) fn wait_for_workflow_test_run_where(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        reason: &str,
        predicate: impl Fn(&WorkflowRun) -> bool,
    ) -> WorkflowRun {
        self.runtime.block_on(async {
            for _ in 0..500 {
                let workflow_run = {
                    let app = self.app.lock().await;
                    app.sessions()
                        .get_session(session_id)
                        .expect("session should resolve")
                        .workflow_run(workflow_run_id)
                        .unwrap_or_else(|| {
                            panic!("workflow run `{workflow_run_id}` should resolve")
                        })
                        .clone()
                };
                if predicate(&workflow_run) {
                    return workflow_run;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("workflow run `{workflow_run_id}` did not reach expected state: {reason}");
        })
    }

    pub(crate) fn wait_for_session_where(
        &self,
        session_id: &str,
        reason: &str,
        predicate: impl Fn(&RuntimeSession) -> bool,
    ) -> RuntimeSession {
        self.runtime.block_on(async {
            for _ in 0..500 {
                let session = {
                    let app = self.app.lock().await;
                    app.sessions()
                        .get_session(session_id)
                        .expect("session should resolve")
                        .clone()
                };
                if predicate(&session) {
                    return session;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("session `{session_id}` did not reach expected state: {reason}");
        })
    }

    fn session_has_no_active_workflow(&self, session_id: &str) -> bool {
        self.with_app(|app| {
            app.sessions()
                .get_session(session_id)
                .map(|session| {
                    !session.has_active_prompt()
                        && session.workflow_runs().iter().all(|run| {
                            !matches!(
                                run.status(),
                                WorkflowRunStatus::Created
                                    | WorkflowRunStatus::Running
                                    | WorkflowRunStatus::Waiting
                                    | WorkflowRunStatus::Completing
                            )
                        })
                })
                .unwrap_or(false)
        })
    }

    pub(crate) fn wait_for_active_provider_run(&self, session_id: &str) -> String {
        self.runtime.block_on(async {
            for _ in 0..500 {
                if let Some(provider_run_id) = self
                    .app
                    .lock()
                    .await
                    .sessions()
                    .get_session(session_id)
                    .expect("session should resolve")
                    .active_provider_run_id()
                    .map(str::to_string)
                {
                    return provider_run_id;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("provider run for session `{session_id}` should become active")
        })
    }
}
