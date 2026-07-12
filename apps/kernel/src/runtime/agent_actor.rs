use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::{mpsc, Mutex};

use crate::error::DaemonError;
use crate::local::{BatchOperationFailure, LocalDaemonResponse, PromptBatchSubmissionResult};
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_prompt_service::AgentPromptCommandService;
use crate::runtime::command_latency::CommandTrace;
use crate::runtime::projection::{
    publish_session_runtime_projection, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::runtime::state::KernelRuntimeState;
use crate::session::{PromptIdAllocator, PromptQueueItem, DEFAULT_LOCAL_USER_ID};

const AGENT_COMMAND_QUEUE_LIMIT: usize = 128;
const DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY: usize = 16;
const DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY_PER_SESSION: usize = 8;

mod agent_resolution;
mod command_executor;
mod command_lane;
mod prompt_attachment_materialization;

use command_lane::{AgentCommand, AgentCommandEnvelope, PromptSubmitResponseMode};

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AgentRuntime {
    store: AgentRuntimeStore,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    prompt_state_owner: PromptStateOwner,
    prompt_id_allocator: PromptIdAllocator,
    queue_limit: usize,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<AgentCommandEnvelope>>>>,
}

impl AgentRuntime {
    pub(crate) fn new(
        state: KernelRuntimeState,
        provider_runtime_lanes: ProviderRunOperationLanes,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_state_owner: PromptStateOwner,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self::with_store(
            AgentRuntimeStore::new(state),
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            prompt_state_owner,
            prompt_id_allocator,
        )
    }

    pub(crate) fn with_store(
        store: AgentRuntimeStore,
        provider_runtime_lanes: ProviderRunOperationLanes,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_state_owner: PromptStateOwner,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self {
            store,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            prompt_state_owner,
            prompt_id_allocator,
            queue_limit: AGENT_COMMAND_QUEUE_LIMIT,
            lanes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn dispatch_prompt_submit(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::SubmitPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.dispatch_prompt_submit_with_response_mode(
            command,
            request,
            PromptSubmitResponseMode::Full,
        )
        .await
    }

    async fn dispatch_prompt_submit_with_response_mode(
        &self,
        command: &crate::runtime::command::KernelCommand,
        mut request: crate::local::SubmitPromptRequest,
        response_mode: PromptSubmitResponseMode,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        let agent_id = self
            .resolve_submit_agent_id(&request.session_id, request.target_agent_id.as_deref())
            .await?;
        self.store
            .ensure_agent_prompt_access(&agent_id, &caller_user_id, "submit prompt")
            .await?;
        request.target_agent_id = Some(agent_id.clone());
        let command_trace = CommandTrace::from_command(command);
        self.dispatch_to_agent(
            agent_id,
            command_trace.clone(),
            AgentCommand::SubmitPrompt {
                request,
                trace_id: command_trace.trace_id().to_string(),
                operation_id: command.durable_operation_id(None),
                operation_fingerprint: command.durable_request_fingerprint(),
                response_mode,
            },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_submit_batch(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::SubmitPromptsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.prompts.is_empty() {
            let session = self.store.session_snapshot(&request.session_id).await?;
            return Ok(LocalDaemonResponse::PromptsSubmitted {
                results: Vec::new(),
                failures: Vec::new(),
                agent_activity: self.store.agent_activity_for_session(&session).await,
                agent_activity_revision: self.session_projection.change_sequence(),
                session,
            });
        }
        if let Some(failures) =
            prompt_batch_preflight_failures(&request.session_id, &request.prompts)
        {
            let session = self.store.session_snapshot(&request.session_id).await?;
            return Ok(LocalDaemonResponse::PromptsSubmitted {
                results: Vec::new(),
                failures,
                agent_activity: self.store.agent_activity_for_session(&session).await,
                agent_activity_revision: self.session_projection.change_sequence(),
                session,
            });
        }
        let caller_user_id = command_agent_actor_user_id(command);
        let authorization_failures = self
            .store
            .prompt_batch_authorization_failures(
                &request.session_id,
                &caller_user_id,
                &request.prompts,
            )
            .await;
        if !authorization_failures.is_empty() {
            let session = self.store.session_snapshot(&request.session_id).await?;
            return Ok(LocalDaemonResponse::PromptsSubmitted {
                results: Vec::new(),
                failures: authorization_failures,
                agent_activity: self.store.agent_activity_for_session(&session).await,
                agent_activity_revision: self.session_projection.change_sequence(),
                session,
            });
        }

        let max_concurrency = prompt_batch_effective_concurrency(request.max_concurrency, &request);
        let response_session_id = request.session_id.clone();
        let prompt_session_ids = prompt_batch_session_ids(&request);
        let mut outcomes = futures_util::stream::iter(interleave_prompt_batch_by_session(request))
            .map(|(index, session_id, attachment_id, item)| {
                let runtime = self.clone();
                let command = command.clone();
                let agent_id = item.target_agent_id.clone();
                async move {
                    let submit_request = item.into_submit_prompt_request(session_id, attachment_id);
                    let command_trace = CommandTrace::from_command(&command);
                    let operation_id = command.durable_operation_id(Some(&index.to_string()));
                    let operation_fingerprint = command.durable_request_fingerprint();
                    let result = runtime
                        .dispatch_to_agent(
                            agent_id.clone(),
                            command_trace.clone(),
                            AgentCommand::SubmitPrompt {
                                request: submit_request,
                                trace_id: command_trace.trace_id().to_string(),
                                operation_id,
                                operation_fingerprint,
                                response_mode: PromptSubmitResponseMode::BatchItem,
                            },
                        )
                        .await;
                    (index, agent_id, result)
                }
            })
            .buffer_unordered(max_concurrency)
            .collect::<Vec<_>>()
            .await;
        outcomes.sort_by_key(|(index, _, _)| *index);

        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (index, agent_id, outcome) in outcomes {
            match outcome {
                Ok(LocalDaemonResponse::PromptSubmitted { outcome, .. }) => {
                    results.push(PromptBatchSubmissionResult {
                        index,
                        agent_id,
                        outcome,
                    });
                }
                Ok(other) => failures.push(BatchOperationFailure {
                    index,
                    agent_id: Some(agent_id),
                    message: format!("unexpected prompt response: {other:?}"),
                }),
                Err(error) => failures.push(BatchOperationFailure {
                    index,
                    agent_id: Some(agent_id),
                    message: error.to_string(),
                }),
            }
        }

        let mut response_session = None;
        for session_id in prompt_session_ids {
            let session = self.store.session_snapshot(&session_id).await?;
            publish_session_runtime_projection(
                &self.session_projection,
                &self.agent_runtime_projection,
                &session,
            );
            if session_id == response_session_id {
                response_session = Some(session);
            }
        }
        let session = match response_session {
            Some(session) => session,
            None => {
                let session = self.store.session_snapshot(&response_session_id).await?;
                publish_session_runtime_projection(
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    &session,
                );
                session
            }
        };
        let agent_activity = self.store.agent_activity_for_session_sync(&session);

        Ok(LocalDaemonResponse::PromptsSubmitted {
            results,
            failures,
            session,
            agent_activity,
            agent_activity_revision: self.session_projection.change_sequence(),
        })
    }

    pub(crate) async fn dispatch_prompt_cancel(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CancelActivePromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        let agent_id = if let Some(target_agent_id) = request.target_agent_id.as_deref() {
            target_agent_id.to_string()
        } else {
            self.resolve_active_prompt_agent_id(&request.session_id)
                .await?
        };
        self.store
            .ensure_agent_owner(&agent_id, &caller_user_id, "cancel active prompt")
            .await?;
        self.dispatch_to_agent(
            agent_id.clone(),
            CommandTrace::from_command(command),
            AgentCommand::CancelActivePrompt {
                request,
                target_agent_id: agent_id.clone(),
            },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_steer_queued(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::SteerQueuedPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        self.store
            .ensure_agent_prompt_access(
                &request.target_agent_id,
                &caller_user_id,
                "steer queued prompt",
            )
            .await?;
        self.dispatch_to_agent(
            request.target_agent_id.clone(),
            CommandTrace::from_command(command),
            AgentCommand::SteerQueuedPrompt { request },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_cancel_queued(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CancelQueuedPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        self.store
            .ensure_agent_prompt_access(
                &request.target_agent_id,
                &caller_user_id,
                "cancel queued prompt",
            )
            .await?;
        self.dispatch_to_agent(
            request.target_agent_id.clone(),
            CommandTrace::from_command(command),
            AgentCommand::CancelQueuedPrompt { request },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_update_queued(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::UpdateQueuedPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        self.store
            .ensure_agent_prompt_access(
                &request.target_agent_id,
                &caller_user_id,
                "update queued prompt",
            )
            .await?;
        self.dispatch_to_agent(
            request.target_agent_id.clone(),
            CommandTrace::from_command(command),
            AgentCommand::UpdateQueuedPrompt { request },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_complete(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CompletePromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        let agent_id = self
            .resolve_active_prompt_agent_id(&request.session_id)
            .await?;
        self.store
            .ensure_agent_owner(&agent_id, &caller_user_id, "complete prompt")
            .await?;
        let next_queued_prompt = self
            .session_projection
            .get(&request.session_id)
            .and_then(|session| {
                self.prompt_state_owner
                    .peek_next_queued_prompt(&session, &agent_id)
            })
            .or_else(|| {
                self.agent_runtime_projection
                    .next_queued_prompt(&request.session_id, &agent_id)
            });
        self.dispatch_to_agent(
            agent_id.clone(),
            CommandTrace::from_command(command),
            AgentCommand::CompletePrompt {
                request,
                target_agent_id: agent_id.clone(),
                next_queued_prompt,
            },
        )
        .await
    }

    pub(crate) fn remove_session_state(&self, session_id: &str) {
        self.prompt_state_owner.remove_session(session_id);
        self.agent_runtime_projection.remove_session(session_id);
    }
}

fn prompt_batch_preflight_failures(
    default_session_id: &str,
    prompts: &[crate::local::SubmitPromptsRequestItem],
) -> Option<Vec<BatchOperationFailure>> {
    let mut seen_targets = HashSet::new();
    let duplicate_target = prompts.iter().any(|prompt| {
        !seen_targets.insert((
            prompt.effective_session_id(default_session_id),
            prompt.target_agent_id.as_str(),
        ))
    });
    if !duplicate_target {
        return None;
    }
    Some(
        prompts
            .iter()
            .enumerate()
            .map(|(index, prompt)| BatchOperationFailure {
                index,
                agent_id: Some(prompt.target_agent_id.clone()),
                message: "prompt batch contains duplicate target agents".to_string(),
            })
            .collect(),
    )
}

fn prompt_batch_session_ids(request: &crate::local::SubmitPromptsRequest) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut session_ids = Vec::new();
    if seen.insert(request.session_id.as_str()) {
        session_ids.push(request.session_id.clone());
    }
    for prompt in &request.prompts {
        let session_id = prompt.effective_session_id(&request.session_id);
        if seen.insert(session_id) {
            session_ids.push(session_id.to_string());
        }
    }
    session_ids
}

fn prompt_batch_effective_concurrency(
    requested: Option<usize>,
    request: &crate::local::SubmitPromptsRequest,
) -> usize {
    if request.prompts.is_empty() {
        return 0;
    }
    let requested = requested.unwrap_or(DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY);
    let session_limit = prompt_batch_session_ids(request)
        .len()
        .saturating_mul(DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY_PER_SESSION)
        .max(1);
    requested.clamp(1, request.prompts.len()).min(session_limit)
}

fn interleave_prompt_batch_by_session(
    request: crate::local::SubmitPromptsRequest,
) -> Vec<(
    usize,
    String,
    String,
    crate::local::SubmitPromptsRequestItem,
)> {
    let mut session_order = Vec::new();
    let mut prompts_by_session: HashMap<
        String,
        VecDeque<(
            usize,
            String,
            String,
            crate::local::SubmitPromptsRequestItem,
        )>,
    > = HashMap::new();
    let default_session_id = request.session_id;
    let default_attachment_id = request.attachment_id;
    for (index, prompt) in request.prompts.into_iter().enumerate() {
        let session_id = prompt
            .session_id
            .clone()
            .unwrap_or_else(|| default_session_id.clone());
        let attachment_id = prompt
            .attachment_id
            .clone()
            .unwrap_or_else(|| default_attachment_id.clone());
        if !prompts_by_session.contains_key(&session_id) {
            session_order.push(session_id.clone());
        }
        prompts_by_session
            .entry(session_id.clone())
            .or_default()
            .push_back((index, session_id, attachment_id, prompt));
    }

    let mut interleaved = Vec::new();
    loop {
        let mut advanced = false;
        for session_id in &session_order {
            let Some(queue) = prompts_by_session.get_mut(session_id) else {
                continue;
            };
            let Some(prompt) = queue.pop_front() else {
                continue;
            };
            interleaved.push(prompt);
            advanced = true;
        }
        if !advanced {
            break;
        }
    }
    interleaved
}

#[derive(Clone)]
pub(crate) struct AgentRuntimeStore {
    state: KernelRuntimeState,
}

impl AgentRuntimeStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        self.state.active_prompt_agent_id(session_id).await
    }

    async fn focused_agent_id(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        self.state.focused_agent_id(session_id).await
    }

    async fn ensure_agent_owner(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.state
            .ensure_agent_owner(agent_id, caller_user_id, operation)
            .await
    }

    async fn ensure_agent_prompt_access(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.state
            .ensure_agent_prompt_access(agent_id, caller_user_id, operation)
            .await
    }

    async fn prompt_batch_authorization_failures(
        &self,
        default_session_id: &str,
        caller_user_id: &str,
        prompts: &[crate::local::SubmitPromptsRequestItem],
    ) -> Vec<BatchOperationFailure> {
        let mut sessions = HashMap::new();
        for prompt in prompts {
            let session_id = prompt.effective_session_id(default_session_id);
            if sessions.contains_key(session_id) {
                continue;
            }
            match self.state.session_snapshot(session_id).await {
                Ok(session) => {
                    sessions.insert(session_id.to_string(), Ok(session));
                }
                Err(error) => {
                    sessions.insert(session_id.to_string(), Err(error.to_string()));
                }
            }
        }
        prompts
            .iter()
            .enumerate()
            .filter_map(|(index, prompt)| {
                let session_id = prompt.effective_session_id(default_session_id);
                let session = match sessions.get(session_id) {
                    Some(Ok(session)) => session,
                    Some(Err(message)) => {
                        return Some(BatchOperationFailure {
                            index,
                            agent_id: Some(prompt.target_agent_id.clone()),
                            message: message.clone(),
                        });
                    }
                    None => {
                        return Some(BatchOperationFailure {
                            index,
                            agent_id: Some(prompt.target_agent_id.clone()),
                            message: format!("session `{session_id}` was not checked"),
                        });
                    }
                };
                let Some(agent) = session
                    .agents()
                    .iter()
                    .find(|agent| agent.id() == prompt.target_agent_id.as_str())
                else {
                    return Some(BatchOperationFailure {
                        index,
                        agent_id: Some(prompt.target_agent_id.clone()),
                        message: DaemonError::AgentNotFound {
                            agent_id: prompt.target_agent_id.clone(),
                        }
                        .to_string(),
                    });
                };
                if !session.can_prompt_agent_owned_by(caller_user_id, agent.owner_user_id()) {
                    return Some(BatchOperationFailure {
                        index,
                        agent_id: Some(prompt.target_agent_id.clone()),
                        message: DaemonError::OwnershipAccessDenied {
                            user_id: caller_user_id.to_string(),
                            owner_user_id: agent.owner_user_id().to_string(),
                            resource: format!("agent `{}`", prompt.target_agent_id),
                            operation: "submit prompt batch",
                        }
                        .to_string(),
                    });
                }
                None
            })
            .collect()
    }

    async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.state.session_snapshot(session_id).await
    }

    async fn agent_activity_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> std::collections::BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        self.state.agent_activity_for_session(session)
    }

    fn agent_activity_for_session_sync(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> std::collections::BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        self.state.agent_activity_for_session(session)
    }

    fn prompt_command_service(
        &self,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> AgentPromptCommandService {
        AgentPromptCommandService::new(self.state.clone(), provider_runtime_lanes)
    }
}

fn command_agent_actor_user_id(command: &crate::runtime::command::KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use base64::Engine;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        CancelActivePromptRequest, CancelQueuedPromptRequest, CompletePromptRequest,
        LocalDaemonRequest, LocalDaemonResponse, SteerQueuedPromptRequest, SubmitPromptRequest,
        UpdateQueuedPromptRequest,
    };
    use crate::provider::{LaunchProviderRequest, ProviderRunOperationLanes};
    use crate::runtime::agent_actor::prompt_attachment_materialization::{
        materialize_inline_prompt_attachments, INLINE_PROMPT_ATTACHMENT_DIR,
    };
    use crate::runtime::agent_actor::AgentRuntime;
    use crate::runtime::agent_actor::{
        interleave_prompt_batch_by_session, prompt_batch_effective_concurrency,
        DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY,
        DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY_PER_SESSION,
    };
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::runtime::prompt_state::PromptStateOwner;
    use crate::runtime::session_actor::FocusedAgentProjection;
    use crate::runtime::state::KernelRuntimeState;
    use crate::session::{
        CreateSessionRequest, PromptAttachment, PromptQueueItem, PromptStatus,
        PromptSubmissionOutcome,
    };
    use crate::DaemonError;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn prompt_batch_interleaves_explicit_sessions_without_losing_indexes() {
        let request = crate::local::SubmitPromptsRequest {
            session_id: "session-a".to_string(),
            attachment_id: "attachment-a".to_string(),
            max_concurrency: Some(4),
            prompts: vec![
                test_prompt(None, None, "agent-a-1"),
                test_prompt(None, None, "agent-a-2"),
                test_prompt(Some("session-b"), Some("attachment-b"), "agent-b-1"),
                test_prompt(Some("session-b"), Some("attachment-b"), "agent-b-2"),
                test_prompt(Some("session-c"), Some("attachment-c"), "agent-c-1"),
            ],
        };

        let interleaved = interleave_prompt_batch_by_session(request);

        let order = interleaved
            .iter()
            .map(|(index, session_id, attachment_id, prompt)| {
                (
                    *index,
                    session_id.as_str(),
                    attachment_id.as_str(),
                    prompt.target_agent_id.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                (0, "session-a", "attachment-a", "agent-a-1"),
                (2, "session-b", "attachment-b", "agent-b-1"),
                (4, "session-c", "attachment-c", "agent-c-1"),
                (1, "session-a", "attachment-a", "agent-a-2"),
                (3, "session-b", "attachment-b", "agent-b-2"),
            ]
        );
    }

    #[test]
    fn prompt_batch_concurrency_caps_to_per_session_budget() {
        let single_session = crate::local::SubmitPromptsRequest {
            session_id: "session-a".to_string(),
            attachment_id: "attachment-a".to_string(),
            max_concurrency: Some(32),
            prompts: (0..32)
                .map(|index| test_prompt(None, None, &format!("agent-{index}")))
                .collect(),
        };
        assert_eq!(
            prompt_batch_effective_concurrency(single_session.max_concurrency, &single_session),
            DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY_PER_SESSION
        );

        let mixed_sessions = crate::local::SubmitPromptsRequest {
            session_id: "session-0".to_string(),
            attachment_id: "attachment-0".to_string(),
            max_concurrency: Some(32),
            prompts: (0..30)
                .map(|index| {
                    let session_index = index % 3;
                    test_prompt(
                        Some(&format!("session-{session_index}")),
                        Some(&format!("attachment-{session_index}")),
                        &format!("agent-{index}"),
                    )
                })
                .collect(),
        };
        assert_eq!(
            prompt_batch_effective_concurrency(mixed_sessions.max_concurrency, &mixed_sessions),
            3 * DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY_PER_SESSION
        );
    }

    #[test]
    fn prompt_batch_concurrency_uses_default_and_requested_caps() {
        let request = crate::local::SubmitPromptsRequest {
            session_id: "session-a".to_string(),
            attachment_id: "attachment-a".to_string(),
            max_concurrency: None,
            prompts: (0..32)
                .map(|index| {
                    test_prompt(
                        Some(&format!("session-{index}")),
                        Some(&format!("attachment-{index}")),
                        &format!("agent-{index}"),
                    )
                })
                .collect(),
        };
        assert_eq!(
            prompt_batch_effective_concurrency(None, &request),
            DEFAULT_PROMPT_BATCH_SUBMIT_CONCURRENCY
        );
        assert_eq!(prompt_batch_effective_concurrency(Some(4), &request), 4);
        assert_eq!(prompt_batch_effective_concurrency(Some(0), &request), 1);
    }

    fn test_prompt(
        session_id: Option<&str>,
        attachment_id: Option<&str>,
        agent_id: &str,
    ) -> crate::local::SubmitPromptsRequestItem {
        crate::local::SubmitPromptsRequestItem {
            session_id: session_id.map(str::to_string),
            attachment_id: attachment_id.map(str::to_string),
            target_agent_id: agent_id.to_string(),
            prompt: format!("prompt {agent_id}"),
            attachments: Vec::new(),
        }
    }

    fn launch_dev_stub_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        model: &str,
    ) {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
                    .with_agent_id(agent_id),
            )
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run.clone());
    }

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.history_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    mod agent_resolution;
    mod prompt_attachment_materialization;
    mod prompt_command_execution;
    mod request_surface;
}
