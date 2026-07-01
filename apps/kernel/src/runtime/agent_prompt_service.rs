use crate::app::{
    KernelPreparedPromptSubmission, KernelPromptAbortDispatch, KernelPromptCancellation,
    KernelPromptDispatch, KernelPromptSubmission, KernelQueuedPromptCancellation,
    KernelQueuedPromptSteer, KernelQueuedPromptUpdate, KernelRemotePromptDispatch,
};
use crate::error::DaemonError;
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::state::KernelRuntimeState;
use crate::session::{PromptCompletion, PromptQueueItem};

#[derive(Clone)]
pub(crate) struct AgentPromptCommandService {
    state: KernelRuntimeState,
    provider_runtime_lanes: ProviderRunOperationLanes,
}

#[derive(Clone)]
struct AgentPromptDispatchContext {
    state: KernelRuntimeState,
    provider_runtime_lanes: ProviderRunOperationLanes,
}

impl AgentPromptDispatchContext {
    fn new(state: KernelRuntimeState, provider_runtime_lanes: ProviderRunOperationLanes) -> Self {
        Self {
            state,
            provider_runtime_lanes,
        }
    }

    fn spawn_prompt_dispatch(&self, dispatch: KernelPromptDispatch) {
        self.state
            .spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
    }

    fn spawn_queued_prompt_steer_dispatch(&self, dispatch: KernelPromptDispatch) {
        self.state
            .spawn_queued_prompt_steer_dispatch(dispatch, self.provider_runtime_lanes.clone());
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
        state: KernelRuntimeState,
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
        self.state.submit_prepared_prompt(prepared).await
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        self.state
            .cancel_agent_prompt(session_id, target_agent_id, attachment_id)
            .await
    }

    pub(crate) async fn steer_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<KernelQueuedPromptSteer, DaemonError> {
        self.state
            .steer_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)
            .await
    }

    pub(crate) async fn cancel_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<KernelQueuedPromptCancellation, DaemonError> {
        self.state
            .cancel_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)
            .await
    }

    pub(crate) async fn update_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prompt: &str,
    ) -> Result<KernelQueuedPromptUpdate, DaemonError> {
        self.state
            .update_queued_prompt(
                session_id,
                target_agent_id,
                attachment_id,
                prompt_id,
                prompt,
            )
            .await
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<PromptQueueItem>,
    ) -> Result<PromptCompletion, DaemonError> {
        self.state
            .complete_agent_prompt(session_id, target_agent_id, next_queued_prompt.as_ref())
            .await
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.state.session_snapshot(session_id).await
    }

    pub(crate) fn start_metaagent_task_for_prompt(
        &self,
        session_id: &str,
        metaagent_id: &str,
        prompt: &str,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        self.state
            .start_metaagent_task_for_prompt(session_id, metaagent_id, prompt)
    }

    pub(crate) async fn activate_meta_mode_for_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        task_prompt: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.state
            .activate_meta_mode_for_prompt(session_id, agent_id, task_prompt)
            .await
    }

    pub(crate) fn meta_mode_entered_hidden_context(&self) -> Result<String, DaemonError> {
        crate::runtime::state::KernelRuntimeState::meta_mode_entered_hidden_context()
    }

    pub(crate) fn start_active_turn_with_trace_id(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        trace_id: &str,
    ) {
        self.state.start_active_turn_with_trace_id(
            session_id,
            agent_id,
            prompt_id,
            provider_run_id,
            trace_id,
        );
    }

    pub(crate) fn agent_activity_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> std::collections::BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        self.state.agent_activity_for_session(session)
    }

    pub(crate) fn inject_metaagent_turn_completion_event(
        &self,
        session_id: &str,
        completed_agent_id: &str,
        completion: &PromptCompletion,
    ) -> Result<(), DaemonError> {
        self.state.inject_metaagent_turn_completion_event(
            session_id,
            completed_agent_id,
            completion,
        )
    }

    pub(crate) fn spawn_prompt_dispatch(&self, dispatch: KernelPromptDispatch) {
        self.dispatch_context().spawn_prompt_dispatch(dispatch);
    }

    pub(crate) fn spawn_queued_prompt_steer_dispatch(&self, dispatch: KernelPromptDispatch) {
        self.dispatch_context()
            .spawn_queued_prompt_steer_dispatch(dispatch);
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
