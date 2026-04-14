use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::{
    DaemonApp, KernelPreparedPromptSubmission, KernelPromptAbortDispatch, KernelPromptCancellation,
    KernelPromptDispatch, KernelPromptSubmission, KernelRemotePromptDispatch,
};
use crate::error::DaemonError;
use crate::provider::ProviderRunOperationLanes;
use crate::session::{PromptCompletion, PromptQueueItem};

#[derive(Clone)]
pub(crate) struct AgentPromptCommandService {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
}

struct AgentPromptCommandContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> AgentPromptCommandContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn submit_prepared_prompt(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        crate::app::KernelAgentService::new(self.app).submit_prepared_prompt_for_kernel(prepared)
    }

    fn cancel_agent_prompt(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        crate::app::KernelAgentService::new(self.app).cancel_agent_prompt_for_kernel(
            session_id,
            target_agent_id,
            attachment_id,
        )
    }

    fn complete_agent_prompt(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&PromptQueueItem>,
    ) -> Result<PromptCompletion, DaemonError> {
        let provider_run_id = self
            .app
            .providers()
            .get_run_for_agent(session_id, target_agent_id)
            .map(|run| run.id().to_string());
        crate::app::KernelAgentService::new(self.app).complete_active_prompt_for_kernel(
            session_id,
            target_agent_id,
            provider_run_id.as_deref(),
            next_queued_prompt,
        )
    }
}

impl AgentPromptCommandService {
    pub(crate) fn new(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        Self {
            app,
            provider_runtime_lanes,
        }
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        let mut app = self.app.lock().await;
        AgentPromptCommandContext::new(&mut app).submit_prepared_prompt(prepared)
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        let mut app = self.app.lock().await;
        AgentPromptCommandContext::new(&mut app).cancel_agent_prompt(
            session_id,
            target_agent_id,
            attachment_id,
        )
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<PromptQueueItem>,
    ) -> Result<PromptCompletion, DaemonError> {
        let mut app = self.app.lock().await;
        AgentPromptCommandContext::new(&mut app).complete_agent_prompt(
            session_id,
            target_agent_id,
            next_queued_prompt.as_ref(),
        )
    }

    pub(crate) fn spawn_prompt_dispatch(&self, dispatch: KernelPromptDispatch) {
        DaemonApp::spawn_kernel_prompt_dispatch_operation(
            Arc::clone(&self.app),
            self.provider_runtime_lanes.clone(),
            dispatch,
        );
    }

    pub(crate) fn spawn_remote_prompt_dispatch(&self, dispatch: KernelRemotePromptDispatch) {
        DaemonApp::spawn_kernel_remote_prompt_dispatch_operation(Arc::clone(&self.app), dispatch);
    }

    pub(crate) fn spawn_prompt_abort(&self, dispatch: KernelPromptAbortDispatch) {
        DaemonApp::spawn_kernel_prompt_abort_operation(
            Arc::clone(&self.app),
            self.provider_runtime_lanes.clone(),
            dispatch,
        );
    }
}
