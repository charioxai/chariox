use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::Mutex;

use crate::agent::AgentInstance;
use crate::runtime::command::{KernelCaller, KernelCommand};
use crate::runtime::router::CommandRouter;
use crate::session::{WorkflowNodeDefinition, WorkflowRun};
use crate::{DaemonApp, DaemonConfig, DaemonError};

use super::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, CompletePromptRequest, GetWorkflowRunRequest,
    LocalDaemonRequest, LocalDaemonResponse, SpawnAgentRequest,
};

static LOCAL_ROUTER_TEST_COMMAND_ID: AtomicU64 = AtomicU64::new(1);

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
        let app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let provider_runtime_lanes = app.provider_run_operation_lanes();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity_and_provider_lanes(
            Arc::clone(&app),
            16,
            provider_runtime_lanes,
        );
        Self {
            runtime: Runtime::new().expect("test runtime should start"),
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
        self.runtime
            .block_on(self.router.dispatch(command, request))
    }

    pub(crate) fn with_app<R>(&self, f: impl FnOnce(&DaemonApp) -> R) -> R {
        let app = self.runtime.block_on(self.app.lock());
        f(&app)
    }

    pub(crate) fn with_app_mut<R>(&self, f: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.runtime.block_on(self.app.lock());
        f(&mut app)
    }

    pub(crate) fn spawn_workflow_test_agent(&self, session_id: &str, alias: &str) -> AgentInstance {
        match self
            .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.to_string(),
                alias: Some(alias.to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                machine_ref: None,
                worktree_placement: None,
            }))
            .expect("workflow test agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        }
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
                    output_schema_ref: None,
                    validation_policy: None,
                    expected_workflow_revision: None,
                },
            ))
            .expect("workflow test edge should be added")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
            _ => panic!("unexpected local response"),
        }
    }

    pub(crate) fn complete_workflow_test_prompt(&self, session_id: &str, label: &str) {
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

    pub(crate) fn wait_for_active_provider_run(&self, session_id: &str) -> String {
        self.runtime.block_on(async {
            for _ in 0..50 {
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
            panic!("provider run should become active")
        })
    }
}
