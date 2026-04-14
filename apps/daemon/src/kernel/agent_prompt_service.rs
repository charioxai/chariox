use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::app::{
    serialize_remote_prompt_attachments, DaemonApp, KernelPreparedPromptSubmission,
    KernelPromptAbortDispatch, KernelPromptCancellation, KernelPromptDispatch,
    KernelPromptSubmission, KernelRemotePromptDispatch,
};
use crate::error::DaemonError;
use crate::provider::ProviderRunOperationLanes;
use crate::session::{PromptCompletion, PromptQueueItem};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

#[derive(Clone)]
pub(crate) struct AgentPromptCommandService {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
}

struct AgentPromptCommandContext<'a> {
    app: &'a mut DaemonApp,
}

#[derive(Clone)]
struct AgentPromptDispatchContext {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
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

impl AgentPromptDispatchContext {
    fn new(app: Arc<Mutex<DaemonApp>>, provider_runtime_lanes: ProviderRunOperationLanes) -> Self {
        Self {
            app,
            provider_runtime_lanes,
        }
    }

    fn spawn_prompt_dispatch(&self, dispatch: KernelPromptDispatch) {
        let app = Arc::clone(&self.app);
        let provider_runtime_lanes = self.provider_runtime_lanes.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            let mut app = app.lock().await;
            if let Err(error) = app.enqueue_kernel_prompt_dispatch(&dispatch) {
                let _ = app.fail_kernel_prompt_dispatch(dispatch, error);
            }
        });
    }

    fn spawn_remote_prompt_dispatch(&self, dispatch: KernelRemotePromptDispatch) {
        let app = Arc::clone(&self.app);
        tokio::spawn(async move {
            let config = {
                let app = app.lock().await;
                app.config().clone()
            };
            let attachments = dispatch.attachments.clone();
            let serialized_attachments = match tokio::task::spawn_blocking(move || {
                serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => Err(DaemonError::LocalTransport {
                    operation: "serialize remote prompt attachments",
                    message: error.to_string(),
                }),
            };
            let result = match serialized_attachments {
                Ok(attachments) => {
                    match send_peer_request_via_temporary_connection(
                        &config,
                        ClientTarget {
                            daemon_id: Some(dispatch.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::SubmitLeasedPrompt {
                            leased_agent_id: dispatch.leased_agent_id.clone(),
                            prompt: dispatch.prompt.clone(),
                            attachments,
                            workflow_context: dispatch.workflow_context.clone(),
                        },
                    )
                    .await
                    {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => Ok(provider_run_id),
                        Ok(other) => Err(DaemonError::LocalTransport {
                            operation: "submit remote prepared prompt",
                            message: format!("unexpected remote prompt response: {other:?}"),
                        }),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            };
            let mut app = app.lock().await;
            let _ = app.finish_kernel_remote_prompt_dispatch(dispatch, result);
        });
    }

    fn spawn_prompt_abort(&self, dispatch: KernelPromptAbortDispatch) {
        let app = Arc::clone(&self.app);
        let provider_runtime_lanes = self.provider_runtime_lanes.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            loop {
                let mut app = app.lock().await;
                match app.enqueue_kernel_prompt_abort(&dispatch) {
                    Ok(()) => break,
                    Err(_) if app.structured_prompt_io_in_flight(&dispatch.provider_run_id) => {
                        drop(app);
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        continue;
                    }
                    Err(error) => {
                        let _ = app.fail_kernel_prompt_abort(dispatch, error);
                        return;
                    }
                }
            }
        });
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
        AgentPromptDispatchContext::new(Arc::clone(&self.app), self.provider_runtime_lanes.clone())
    }
}
