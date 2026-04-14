use crate::app::{
    KernelPreparedPromptSubmission, KernelPromptAbortDispatch, KernelPromptCancellation,
    KernelPromptDispatch, KernelPromptSubmission, KernelRemotePromptDispatch,
};
use crate::error::DaemonError;
use crate::kernel::runtime_state::CompatibilityRuntimeState;
use crate::provider::ProviderRunOperationLanes;
use crate::session::{PromptCompletion, PromptQueueItem};

#[derive(Clone)]
pub(crate) struct AgentPromptCommandService {
    state: CompatibilityRuntimeState,
    provider_runtime_lanes: ProviderRunOperationLanes,
}

#[derive(Clone)]
struct AgentPromptDispatchContext {
    state: CompatibilityRuntimeState,
    provider_runtime_lanes: ProviderRunOperationLanes,
}

impl AgentPromptDispatchContext {
    fn new(
        state: CompatibilityRuntimeState,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        Self {
            state,
            provider_runtime_lanes,
        }
    }

    fn spawn_prompt_dispatch(&self, dispatch: KernelPromptDispatch) {
        self.state
            .spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
    }

    fn spawn_remote_prompt_dispatch(&self, dispatch: KernelRemotePromptDispatch) {
        self.state.spawn_remote_prompt_dispatch(dispatch);
    }

    fn spawn_prompt_abort(&self, dispatch: KernelPromptAbortDispatch) {
        self.state
            .spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
    }
}

impl AgentPromptCommandService {
    pub(crate) fn new(
        state: CompatibilityRuntimeState,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        Self {
            state,
            provider_runtime_lanes,
        }
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        self.state
            .with_agent_prompt_mut(|prompt| prompt.submit_prepared_prompt(prepared))
            .await
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        self.state
            .with_agent_prompt_mut(|prompt| {
                prompt.cancel_agent_prompt(session_id, target_agent_id, attachment_id)
            })
            .await
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<PromptQueueItem>,
    ) -> Result<PromptCompletion, DaemonError> {
        self.state
            .with_agent_prompt_mut(|prompt| {
                prompt.complete_agent_prompt(
                    session_id,
                    target_agent_id,
                    next_queued_prompt.as_ref(),
                )
            })
            .await
    }

    pub(crate) fn spawn_prompt_dispatch(&self, dispatch: KernelPromptDispatch) {
        self.dispatch_context().spawn_prompt_dispatch(dispatch);
    }

    pub(crate) fn spawn_remote_prompt_dispatch(&self, dispatch: KernelRemotePromptDispatch) {
        self.dispatch_context()
            .spawn_remote_prompt_dispatch(dispatch);
    }

    pub(crate) fn spawn_prompt_abort(&self, dispatch: KernelPromptAbortDispatch) {
        self.dispatch_context().spawn_prompt_abort(dispatch);
    }

    fn dispatch_context(&self) -> AgentPromptDispatchContext {
        AgentPromptDispatchContext::new(self.state.clone(), self.provider_runtime_lanes.clone())
    }
}
