use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, NativeProviderInteractionResolution,
    RequestNativeProviderTurnInteractionRequest,
};
use crate::provider::{
    CodexApprovalHandover, CodexApprovalOutcome, ProviderNativeInteractionBridge,
    ProviderNativeInteractionResolution, ProviderProcessServiceStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{RuntimeInteraction, RuntimeInteractionKind};
use crate::transport::relay_peer::RemoteNativeInteractionContext;

pub(crate) const REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER: Duration = Duration::from_secs(15);
/// Backoff ceiling between forwards of one worker interaction while its home
/// is unreachable.
const REMOTE_NATIVE_INTERACTION_RETRY_MAX: Duration = Duration::from_secs(15);
/// How long a home keeps a forwarded decision's answer for a worker whose
/// reply was lost and that forwards the same interaction again.
const FORWARDED_DECISION_RETENTION: Duration = Duration::from_secs(600);
/// How often a card held open for a provider terminal checks the terminal is still there; the
/// card's own timeout applies once it is gone.
const PROVIDER_TERMINAL_RECHECK: Duration = Duration::from_secs(5);
/// How long past its own deadline a card waits for the person at a provider terminal. The
/// kernel then settles it, so the provider's handler, which outwaits this, always gets an answer.
pub(crate) const PROVIDER_TERMINAL_HOLD_LIMIT_SECS: u64 = 23 * 60 * 60;
const PROVIDER_TERMINAL_HOLD_LIMIT: Duration =
    Duration::from_secs(PROVIDER_TERMINAL_HOLD_LIMIT_SECS);
// A Claude permission hook outwaits a held card and a forwarded interaction's timeout and
// relay buffer, with room for event pickup and the relay connection, so the answer or deny the
// kernel settles it with always reaches Claude.
const _: () = assert!(
    crate::provider::CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS
        >= PROVIDER_TERMINAL_HOLD_LIMIT_SECS
            + crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS
            + REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER.as_secs()
            + 15
);

struct RuntimeStateNativeInteractionBridge {
    handle: Handle,
    state: KernelRuntimeState,
}

pub(in crate::runtime) type CodexApprovalTransfer = Arc<Mutex<Option<CodexApprovalHandover>>>;

pub(in crate::runtime) struct CodexApprovalRequest {
    pub(in crate::runtime) adopted: Option<CodexApprovalHandover>,
    pub(in crate::runtime) transfer: CodexApprovalTransfer,
}

impl ProviderNativeInteractionBridge for RuntimeStateNativeInteractionBridge {
    fn capture_turn_origin(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Option<crate::session::NativeInteractionOrigin> {
        self.state
            .capture_native_interaction_origin(session_id, agent_id, provider_run_id)
    }

    fn request_blocking(
        &self,
        session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError> {
        self.request(session_id, interaction, None)
    }

    fn request_codex_workspace_approval_blocking(
        &self,
        session_id: &str,
        interaction: RuntimeInteraction,
        adopted: Option<CodexApprovalHandover>,
    ) -> Result<CodexApprovalOutcome, DaemonError> {
        let request = CodexApprovalRequest {
            adopted,
            transfer: Default::default(),
        };
        let resolution = self.request(session_id, interaction, Some(&request))?;
        let handed_over = request
            .transfer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(match handed_over {
            Some(handover) => CodexApprovalOutcome::HandingOver(handover),
            None => CodexApprovalOutcome::Resolved(resolution),
        })
    }
}

impl RuntimeStateNativeInteractionBridge {
    fn request(
        &self,
        session_id: &str,
        interaction: RuntimeInteraction,
        codex_approval: Option<&CodexApprovalRequest>,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError> {
        let Some(origin) = interaction.native_origin().cloned() else {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            });
        };
        let session_id = session_id.to_string();
        let interaction_agent_id = interaction
            .agent_id()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "provider_native_interaction_bridge",
                message: "Provider interactions require an agent subject".into(),
            })?
            .to_string();
        let state = self.state.clone();
        // Shutdown withdraws every pending decision: a provider thread blocked here is released
        // with a timed-out answer instead of holding the drain.
        self.handle.block_on(async {
            tokio::select! {
                biased;
                () = state.wait_for_provider_decisions_closed() => Ok(shutdown_resolution()),
                resolution = async {
                    if let Some(target) = state
                        .remote_native_interaction_context(&session_id, &interaction_agent_id, &origin)
                        .await?
                    {
                        return forward_native_interaction_to_home(
                            &state,
                            &session_id,
                            &origin,
                            target,
                            interaction,
                        )
                        .await;
                    }
                    wait_for_runtime_interaction(
                        &state,
                        &session_id,
                        interaction,
                        "provider_native_interaction_bridge",
                        None,
                        codex_approval,
                    )
                    .await
                } => resolution,
            }
        })
    }
}

fn shutdown_resolution() -> ProviderNativeInteractionResolution {
    ProviderNativeInteractionResolution {
        status: "timed_out".into(),
        choice_id: None,
        reply: None,
    }
}

type RemoteNativeInteractionTarget = (
    crate::config::DaemonConfig,
    String,
    RemoteNativeInteractionContext,
);

/// Forwards a worker interaction to its home until the home answers. A lost
/// request or reply (relay hiccup, timeout) forwards the same interaction
/// again, which joins the home's pending decision, while the lease still
/// serves the run that asked.
async fn forward_native_interaction_to_home(
    state: &KernelRuntimeState,
    session_id: &str,
    origin: &crate::session::NativeInteractionOrigin,
    mut target: RemoteNativeInteractionTarget,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    // The home's card expires at `expires_at`, and the home may hold it past that for the person at a
    // provider terminal: the worker outwaits the card by that hold and the buffer, so an answer at
    // any time the home still holds the card reaches the provider operation.
    let expires_at = interaction
        .timeout_sec()
        .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds));
    let deadline = expires_at.map(forwarding_deadline);
    let mut backoff = Duration::from_secs(1);
    let mut attempt = 1_u32;
    loop {
        if deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
            return Ok(timed_out_resolution());
        }
        let forwarded = match expires_at {
            Some(at) => interaction.clone().with_decision_expiry(at),
            None => interaction.clone(),
        };
        let (config, target_daemon_id, context) = &target;
        let forward = forward_native_interaction_once(
            state,
            config,
            target_daemon_id,
            context.clone(),
            forwarded,
        );
        // Connection setup and writes count against the same decision budget.
        let outcome = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, forward).await {
                Ok(outcome) => outcome,
                Err(_) => return Ok(timed_out_resolution()),
            },
            None => forward.await,
        };
        let error = match outcome {
            Ok(resolution) => return Ok(resolution),
            Err(error) if !remote_native_interaction_forward_is_retryable(&error) => {
                return Err(error)
            }
            Err(error) => error,
        };
        crate::logging::warn_with_fields(
            "daemon.native_interaction",
            "forwarding a native interaction to the home failed; retrying",
            serde_json::json!({
                "interaction_id": interaction.id(),
                "attempt": attempt,
                "error": error.to_string(),
            }),
        );
        let next = tokio::time::Instant::now() + backoff;
        if deadline.is_some_and(|deadline| next >= deadline) {
            return Ok(timed_out_resolution());
        }
        tokio::time::sleep_until(next).await;
        backoff = (backoff * 2).min(REMOTE_NATIVE_INTERACTION_RETRY_MAX);
        attempt = attempt.saturating_add(1);
        let Some(agent_id) = interaction.agent_id() else {
            return Ok(timed_out_resolution());
        };
        match state
            .remote_native_interaction_context(session_id, agent_id, origin)
            .await?
        {
            Some(current) => target = current,
            None => return Ok(timed_out_resolution()),
        }
    }
}

/// Only transport failures repeat a forward; a home refusal is final.
fn remote_native_interaction_forward_is_retryable(error: &DaemonError) -> bool {
    match error {
        DaemonError::RelayTransport { retryable, .. } => *retryable,
        DaemonError::LocalTransport { operation, .. } => {
            *operation != "provider_native_interaction_bridge"
        }
        _ => false,
    }
}

fn timed_out_resolution() -> ProviderNativeInteractionResolution {
    ProviderNativeInteractionResolution {
        status: "timed_out".into(),
        choice_id: None,
        reply: None,
    }
}

async fn forward_native_interaction_once(
    state: &KernelRuntimeState,
    config: &crate::config::DaemonConfig,
    target_daemon_id: &str,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let response_timeout =
        remote_native_interaction_response_timeout(&interaction, config.relay_request_timeout_ms);
    let response = state
        .send_worker_home_runtime_request(
            config,
            target_daemon_id,
            crate::transport::relay_peer::RelayPeerRequest::ForwardNativeTurnInteraction {
                context,
                interaction,
            },
            response_timeout,
        )
        .await?;
    match response {
        crate::transport::relay_peer::RelayPeerResponse::NativeInteractionResolved {
            resolution,
        } => Ok(resolution),
        other => Err(DaemonError::LocalTransport {
            operation: "provider_native_interaction_bridge",
            message: format!("unexpected relay response for remote native interaction: {other:?}"),
        }),
    }
}

/// Async counterpart of the provider bridge for runtime MCP tools: forwards a
/// leased worker agent's interaction to its home kernel, otherwise creates it
/// here. Dropping the future (the provider abandoned the call) closes a local
/// interaction through its timeout path.
pub(crate) async fn request_provider_native_interaction(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let agent_id = interaction
        .agent_id()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Provider interactions require an agent subject".into(),
        })?
        .to_string();
    let Some(origin) = interaction.native_origin().cloned() else {
        return Ok(ProviderNativeInteractionResolution {
            status: "timed_out".into(),
            choice_id: None,
            reply: None,
        });
    };
    tokio::select! {
        biased;
        () = state.wait_for_provider_decisions_closed() => Ok(shutdown_resolution()),
        resolution = async {
            if let Some(target) = state
                .remote_native_interaction_context(session_id, &agent_id, &origin)
                .await?
            {
                return forward_native_interaction_to_home(
                    state, session_id, &origin, target, interaction,
                )
                .await;
            }
            request_runtime_interaction_with_timeout(state, session_id, interaction, operation, None)
                .await
        } => resolution,
    }
}

struct AbandonedInteractionGuard(Option<(KernelRuntimeState, String, String)>);

impl Drop for AbandonedInteractionGuard {
    fn drop(&mut self) {
        let Some((state, session_id, interaction_id)) = self.0.take() else {
            return;
        };
        if let Ok(handle) = Handle::try_current() {
            handle.spawn(async move {
                let _ = state
                    .timeout_runtime_interaction(&session_id, &interaction_id)
                    .await;
            });
        }
    }
}

/// One forward outlives the user's decision deadline by a buffer, but a retry
/// never waits past the deadline the first forward started.
fn remote_native_interaction_response_timeout(
    interaction: &RuntimeInteraction,
    relay_request_timeout_ms: u64,
) -> Duration {
    interaction
        .timeout_sec()
        .map(Duration::from_secs)
        .map(|timeout| {
            timeout
                .saturating_add(PROVIDER_TERMINAL_HOLD_LIMIT)
                .saturating_add(REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER)
        })
        .unwrap_or_else(|| Duration::from_millis(relay_request_timeout_ms))
}

/// When the worker gives up waiting for the home: the card's own deadline, the longest the home
/// holds it past that, and the buffer for the home's answer to come back.
fn forwarding_deadline(expires_at: tokio::time::Instant) -> tokio::time::Instant {
    expires_at + PROVIDER_TERMINAL_HOLD_LIMIT + REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER
}

pub(crate) fn install_provider_native_interaction_bridge(
    state: KernelRuntimeState,
    provider_store: &ProviderProcessServiceStore,
) -> Option<Arc<dyn ProviderNativeInteractionBridge>> {
    let handle = Handle::try_current().ok()?;
    let bridge: Arc<dyn ProviderNativeInteractionBridge> =
        Arc::new(RuntimeStateNativeInteractionBridge { handle, state });
    // The router owns this bridge. A strong reference in the provider store
    // would retain the runtime, which itself owns that same provider store.
    provider_store.set_native_interaction_bridge(bridge.clone());
    Some(bridge)
}

pub(crate) async fn execute_native_provider_interaction_request(
    state: &KernelRuntimeState,
    caller_user_id: &str,
    request: RequestNativeProviderTurnInteractionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    // The approval asks the room in the agent's name, so only who acts for its owner raises it.
    state
        .ensure_agent_owner(
            &request.agent_id,
            caller_user_id,
            "request native interaction",
        )
        .await?;
    let session_id = request.session_id.clone();
    let interaction = RuntimeInteraction::new(
        request.interaction_id,
        request.agent_id,
        RuntimeInteractionKind::Permission,
        request.level,
        request.title,
        request.message,
        request.choices,
        request.custom_choice,
        request.timeout_sec,
        request.default_on_timeout,
    )
    .with_native_origin(Some(request.origin));
    let resolution = request_runtime_interaction_with_timeout(
        state,
        &session_id,
        interaction,
        "native_provider_interaction_request",
        None,
    )
    .await?;
    Ok(LocalDaemonResponse::NativeProviderInteractionResolved {
        resolution: NativeProviderInteractionResolution {
            status: resolution.status,
            choice_id: resolution.choice_id,
            reply: resolution.reply,
        },
    })
}

pub(crate) async fn forward_relay_native_interaction(
    state: &KernelRuntimeState,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    use crate::session::NativeInteractionOrigin;
    if interaction
        .native_origin()
        .is_none_or(|origin| origin.provider_run_id() != context.worker_provider_run_id)
    {
        return Ok(ProviderNativeInteractionResolution {
            status: "timed_out".into(),
            choice_id: None,
            reply: None,
        });
    }
    let origin = match interaction.native_origin() {
        Some(NativeInteractionOrigin::ProviderStartup { .. }) => {
            NativeInteractionOrigin::ProviderStartup {
                provider_run_id: context.worker_provider_run_id.clone(),
            }
        }
        Some(_) => NativeInteractionOrigin::Prompt {
            provider_run_id: context.worker_provider_run_id.clone(),
            prompt_id: context.home_prompt_id.clone().unwrap_or_default(),
        },
        None => {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            })
        }
    };
    let interaction = interaction
        .with_agent_id(context.home_agent_id.clone())
        .with_native_origin(Some(origin));
    join_forwarded_decision(state, context, interaction).await
}

type ForwardedDecisionKey = (String, String, String);
type ForwardedDecisionAnswer = Option<Result<ProviderNativeInteractionResolution, String>>;

struct ForwardedDecision {
    answer: tokio::sync::watch::Receiver<ForwardedDecisionAnswer>,
    /// When the answer was recorded: a held card can wait for hours, so its answer is kept for the
    /// retry window from here, not from `opened_at`.
    settled_at: Option<Instant>,
}

/// Keeps a decision while it is pending, and its answer for the retry window.
fn prune_forwarded_decisions(decisions: &mut HashMap<ForwardedDecisionKey, ForwardedDecision>) {
    decisions.retain(|_, decision| {
        if decision.answer.borrow().is_some() {
            decision
                .settled_at
                .is_none_or(|settled| settled.elapsed() < FORWARDED_DECISION_RETENTION)
        } else {
            decision.answer.has_changed().is_ok()
        }
    });
}

fn forwarded_decisions() -> &'static Mutex<HashMap<ForwardedDecisionKey, ForwardedDecision>> {
    static DECISIONS: OnceLock<Mutex<HashMap<ForwardedDecisionKey, ForwardedDecision>>> =
        OnceLock::new();
    DECISIONS.get_or_init(Default::default)
}

/// One home decision per forwarded worker interaction: a worker that lost its
/// request or reply and forwards the same interaction again joins the pending
/// decision, or gets its answer, instead of asking the human twice.
async fn join_forwarded_decision(
    state: &KernelRuntimeState,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let key = (
        context.home_session_id.clone(),
        context.worker_provider_run_id.clone(),
        interaction.id().to_string(),
    );
    let opened = {
        let mut decisions = forwarded_decisions()
            .lock()
            .expect("forwarded decisions mutex poisoned");
        prune_forwarded_decisions(&mut decisions);
        match decisions.get(&key) {
            Some(decision) => Err(decision.answer.clone()),
            None => {
                let (sender, answer) = tokio::sync::watch::channel(None);
                decisions.insert(
                    key.clone(),
                    ForwardedDecision {
                        answer,
                        settled_at: None,
                    },
                );
                Ok(sender)
            }
        }
    };
    let mut answer = match opened {
        Ok(sender) => {
            let result = request_runtime_interaction_with_timeout(
                state,
                &context.home_session_id,
                interaction,
                "relay_forward_native_interaction",
                Some(&context),
            )
            .await;
            if result.is_err() {
                // A refused decision is not remembered: a later forward
                // revalidates it from scratch.
                forwarded_decisions()
                    .lock()
                    .expect("forwarded decisions mutex poisoned")
                    .remove(&key);
            }
            let _ = sender.send(Some(
                result
                    .as_ref()
                    .map(Clone::clone)
                    .map_err(ToString::to_string),
            ));
            if let Some(decision) = forwarded_decisions()
                .lock()
                .expect("forwarded decisions mutex poisoned")
                .get_mut(&key)
            {
                decision.settled_at = Some(Instant::now());
            }
            return result;
        }
        Err(answer) => answer,
    };
    let answered = answer
        .wait_for(Option::is_some)
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "relay_forward_native_interaction",
            message: "forwarded decision dropped before resolution".into(),
        })?
        .clone();
    answered
        .expect("waited for an answer")
        .map_err(|message| DaemonError::LocalTransport {
            operation: "relay_forward_native_interaction",
            message,
        })
}

async fn request_runtime_interaction_with_timeout(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
    forwarding: Option<&RemoteNativeInteractionContext>,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    wait_for_runtime_interaction(state, session_id, interaction, operation, forwarding, None).await
}

async fn wait_for_runtime_interaction(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
    forwarding: Option<&RemoteNativeInteractionContext>,
    codex_approval: Option<&CodexApprovalRequest>,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    request_runtime_interaction_holding(
        state,
        session_id,
        interaction,
        operation,
        forwarding,
        codex_approval,
        PROVIDER_TERMINAL_HOLD_LIMIT,
    )
    .await
}

/// `hold_limit` bounds how long a card waits past its own deadline for the person at a
/// provider terminal; tests pass a short one.
async fn request_runtime_interaction_holding(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
    forwarding: Option<&RemoteNativeInteractionContext>,
    codex_approval: Option<&CodexApprovalRequest>,
    hold_limit: Duration,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    if let Some(resolution) =
        state.sudo_native_permission_resolution(session_id, &interaction, forwarding)
    {
        return Ok(resolution);
    }
    // MP-08 / MP-10 / MP-11: parallel provider tools retain the one-decision
    // subject gate. Wait only for its temporary refusal; every attempt keeps
    // the original identity/origin and revalidates current runtime authority.
    let deadline = interaction
        .timeout_sec()
        .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds));
    let timeout_session_id = session_id.to_string();
    let timeout_interaction_id = interaction.id().to_string();
    let mut receiver = loop {
        if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            });
        }
        let receiver = if let Some(request) = codex_approval {
            state
                .create_codex_workspace_approval(
                    crate::runtime::state::InteractionSurface::native(&interaction)?,
                    session_id,
                    interaction.clone(),
                    request,
                )
                .await
        } else {
            state
                .create_runtime_interaction_with_forwarding(
                    crate::runtime::state::InteractionSurface::native(&interaction)?,
                    session_id,
                    interaction.clone(),
                    forwarding,
                )
                .await
        };
        match receiver {
            Ok(receiver) => break receiver,
            Err(error) if crate::runtime::state::interaction_waits(&error) => {
                let next = tokio::time::Instant::now() + Duration::from_millis(50);
                tokio::time::sleep_until(deadline.map_or(next, |deadline| deadline.min(next)))
                    .await;
            }
            Err(error) => return Err(error),
        }
    };
    // Cancellation can retire only an interaction this invocation admitted,
    // never the card whose subject/identity caused its admission to wait.
    let mut abandoned = AbandonedInteractionGuard(Some((
        state.clone(),
        timeout_session_id.clone(),
        timeout_interaction_id.clone(),
    )));
    // Waiting for another tool consumes this decision's original budget. While a provider's
    // own terminal runs the pane the registered card shows in, the person sits there: the card
    // waits for them, up to the hold limit, and the kernel then settles it like any timeout.
    let resolution = if let Some(mut deadline) = deadline {
        let hold_ends = deadline + hold_limit;
        loop {
            tokio::select! {
                biased;
                resolution = &mut receiver => break resolution,
                _ = tokio::time::sleep_until(deadline) => {
                    if tokio::time::Instant::now() < hold_ends
                        && state.provider_terminal_holds_card(&timeout_session_id, &timeout_interaction_id)
                    {
                        deadline = (tokio::time::Instant::now() + PROVIDER_TERMINAL_RECHECK).min(hold_ends);
                        continue;
                    }
                    state.timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id).await?;
                    break receiver.await;
                }
            }
        }
    } else {
        receiver.await
    }.map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!("interaction dropped before resolution: {error}"),
    })?;
    abandoned.0 = None;
    Ok(ProviderNativeInteractionResolution {
        status: resolution.status.to_string(),
        choice_id: resolution.choice_id,
        reply: resolution.reply,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{RuntimeInteractionChoice, RuntimeInteractionLevel};

    fn interaction(timeout_sec: Option<u64>) -> RuntimeInteraction {
        RuntimeInteraction::new(
            "interaction-1",
            "agent-1",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            None,
            "Approve?",
            Vec::<RuntimeInteractionChoice>::new(),
            None,
            timeout_sec,
            None,
        )
    }

    #[test]
    fn remote_response_timeout_outlives_the_user_interaction_deadline() {
        // The card's timeout, the longest the home holds it past that for the person at a provider
        // terminal, and the buffer for the answer to come back.
        let held = PROVIDER_TERMINAL_HOLD_LIMIT_SECS;
        assert_eq!(
            remote_native_interaction_response_timeout(&interaction(Some(300)), 60_000),
            Duration::from_secs(300 + held + 15)
        );
        assert_eq!(
            remote_native_interaction_response_timeout(&interaction(Some(5)), 60_000),
            Duration::from_secs(5 + held + 15)
        );
    }

    /// A leased child asks permission in its native-terminal parent's pane: the home holds the card
    /// past its 300 s deadline, so the worker must still be waiting when the person answers late.
    #[tokio::test(start_paused = true)]
    async fn the_worker_keeps_waiting_while_the_home_holds_the_card() {
        let expires_at = tokio::time::Instant::now() + Duration::from_secs(300);
        let deadline = forwarding_deadline(expires_at);
        tokio::time::advance(Duration::from_secs(316)).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "a worker that gave up at the card's timeout plus the buffer loses a later answer"
        );
        tokio::time::advance(Duration::from_secs(PROVIDER_TERMINAL_HOLD_LIMIT_SECS - 300)).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "the home settles the card at the end of its hold: the worker is still there to receive that"
        );
        tokio::time::advance(deadline - tokio::time::Instant::now()).await;
        assert!(tokio::time::Instant::now() >= deadline);
    }

    fn ago(seconds: u64) -> Instant {
        Instant::now()
            .checked_sub(Duration::from_secs(seconds))
            .expect("the clock has run this long")
    }

    fn answered_decision(settled_ago: u64) -> ForwardedDecision {
        let (_sender, answer) =
            tokio::sync::watch::channel(Some(Ok(ProviderNativeInteractionResolution {
                status: "answered".into(),
                choice_id: Some("allow".into()),
                reply: None,
            })));
        ForwardedDecision {
            answer,
            settled_at: Some(ago(settled_ago)),
        }
    }

    /// A card the home held for half an hour and the person answered a minute ago: the worker's
    /// retry after a lost reply must get that answer back, not open the permission again.
    #[test]
    fn an_answer_to_a_held_card_is_kept_for_the_retry_window_from_when_it_settled() {
        let key = |name: &str| ("room".to_string(), "run".to_string(), name.to_string());
        let mut decisions = HashMap::new();
        decisions.insert(key("held-then-answered"), answered_decision(60));
        decisions.insert(key("answered-long-ago"), answered_decision(700));
        decisions.insert(key("answered-just-now"), answered_decision(5));
        prune_forwarded_decisions(&mut decisions);
        let mut kept = decisions
            .keys()
            .map(|key| key.2.clone())
            .collect::<Vec<_>>();
        kept.sort();
        assert_eq!(kept, ["answered-just-now", "held-then-answered"]);
    }

    #[tokio::test(start_paused = true)]
    async fn forwarded_interaction_restates_the_remaining_budget_at_every_rebase() {
        let expires_at = tokio::time::Instant::now() + Duration::from_secs(300);
        let mut forwarded = interaction(Some(300)).with_decision_expiry(expires_at);
        assert_eq!(forwarded.timeout_sec(), Some(300));
        tokio::time::advance(Duration::from_secs(40)).await;
        assert_eq!(forwarded.timeout_sec(), Some(300));
        forwarded.rebase_decision_budget();
        assert_eq!(forwarded.timeout_sec(), Some(260));
        tokio::time::advance(Duration::from_millis(259_500)).await;
        forwarded.rebase_decision_budget();
        assert_eq!(forwarded.timeout_sec(), Some(1));
        let mut open_ended = interaction(None);
        open_ended.rebase_decision_budget();
        assert_eq!(open_ended.timeout_sec(), None);
    }

    #[test]
    fn only_transport_failures_repeat_a_forwarded_interaction() {
        let relay = |retryable| DaemonError::RelayTransport {
            operation: "read temporary relay peer response",
            code: "transport_error".into(),
            message: "relay hiccup".into(),
            retryable,
        };
        let local = |operation| DaemonError::LocalTransport {
            operation,
            message: "timed out waiting for relay peer response after 60000ms".into(),
        };
        assert!(remote_native_interaction_forward_is_retryable(&relay(true)));
        assert!(remote_native_interaction_forward_is_retryable(&local(
            "read temporary relay peer response"
        )));
        assert!(!remote_native_interaction_forward_is_retryable(&relay(
            false
        )));
        assert!(!remote_native_interaction_forward_is_retryable(&local(
            "provider_native_interaction_bridge"
        )));
    }

    #[test]
    fn remote_response_timeout_uses_relay_default_without_interaction_deadline() {
        assert_eq!(
            remote_native_interaction_response_timeout(&interaction(None), 42_000),
            Duration::from_secs(42)
        );
    }

    #[tokio::test]
    async fn native_interaction_bridge_releases_runtime_after_last_router_drops() {
        let config = crate::config::DaemonConfig::for_tests();
        let app = Arc::new(tokio::sync::Mutex::new(
            crate::app::DaemonApp::bootstrap(config.clone()).unwrap(),
        ));
        let weak_app = Arc::downgrade(&app);
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app, 1);
        let clone = router.clone();
        assert!(crate::app::DaemonApp::bootstrap(config.clone()).is_err());
        drop(router);
        assert!(weak_app.upgrade().is_some());
        assert!(crate::app::DaemonApp::bootstrap(config.clone()).is_err());
        drop(clone);
        assert!(
            weak_app.upgrade().is_none(),
            "permission bridge must not retain its own runtime"
        );
        assert!(crate::app::DaemonApp::bootstrap(config).is_ok());
    }

    async fn permission_queue_fixture() -> (
        crate::runtime::router::CommandRouter,
        String,
        String,
        std::path::PathBuf,
    ) {
        let root = crate::test_support::test_workspace_root().join(format!(
            "native-permission-queue-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let worktree = root.to_string_lossy().into_owned();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                &worktree, &worktree,
            ))
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        (router, session.id().into(), agent.id().into(), root)
    }

    fn queued_permission(id: &str, agent: &str, timeout: u64) -> RuntimeInteraction {
        RuntimeInteraction::new(
            id,
            agent,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve documentation fetch?".into()),
            "Fetch the requested public documentation",
            vec![
                RuntimeInteractionChoice::new("allow", "Allow once", "allow", None),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(timeout),
            Some("deny".into()),
        )
    }

    #[tokio::test]
    async fn native_permission_queue_waits_for_the_existing_subject_decision() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let first_id = format!("native-queue-{session}-first");
        let second_id = format!("native-queue-{session}-second");
        let first = state
            .create_runtime_interaction(
                crate::runtime::state::InteractionSurface::named("provider_native_permission"),
                &session,
                queued_permission(&first_id, &agent, 10),
            )
            .await
            .unwrap();
        let pending_state = state.clone();
        let pending_session = session.clone();
        let pending_second_id = second_id.clone();
        let second_task = tokio::spawn(async move {
            request_runtime_interaction_with_timeout(
                &pending_state,
                &pending_session,
                queued_permission(&pending_second_id, &agent, 5),
                "native_permission_queue_test",
                None,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !second_task.is_finished(),
            "MP-10/MP-11 parallel permission must wait for the current decision"
        );
        state
            .resolve_runtime_interaction(&session, &first_id, "allow", None)
            .await
            .unwrap();
        assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if state
                .resolve_runtime_interaction(&session, &second_id, "deny", None)
                .await
                .is_ok()
            {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let resolution = second_task.await.unwrap().unwrap();
        assert_eq!(resolution.choice_id.as_deref(), Some("deny"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A Room whose top agent a provider terminal runs, with a Chariox-run sub-agent below it.
    async fn native_terminal_fixture() -> (
        crate::runtime::router::CommandRouter,
        String,
        String,
        String,
        std::path::PathBuf,
    ) {
        let root = crate::test_support::test_workspace_root().join(format!(
            "native-permission-terminal-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let worktree = root.to_string_lossy().into_owned();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                &worktree, &worktree,
            ))
            .unwrap();
        let (session, parent) = (session.id().to_string(), agent.id().to_string());
        let child = format!("child-{:016x}", rand::random::<u64>());
        let mut child_agent = crate::agent::AgentInstance::new(
            child.clone(),
            "child",
            session.clone(),
            None,
            "dev-stub",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 1, 1, 1),
        );
        child_agent.set_parent_agent(Some(crate::agent::AgentRef::local(parent.clone())));
        app.agents_mut().restore_agent(child_agent);
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    &session,
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(&parent)
                .with_client_interface(crate::provider::ProviderClientInterface::NativeTui),
            )
            .unwrap();
        app.update_provider_run_projection(run);
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        (router, session, parent, child, root)
    }

    fn wait_for_permission(
        state: &KernelRuntimeState,
        session: &str,
        id: &str,
        agent: &str,
        hold_limit: Duration,
    ) -> tokio::task::JoinHandle<Result<ProviderNativeInteractionResolution, DaemonError>> {
        let (state, session, id, agent) = (
            state.clone(),
            session.to_owned(),
            id.to_owned(),
            agent.to_owned(),
        );
        tokio::spawn(async move {
            request_runtime_interaction_holding(
                &state,
                &session,
                queued_permission(&id, &agent, 1),
                "native_permission_terminal_test",
                None,
                None,
                hold_limit,
            )
            .await
        })
    }

    #[tokio::test]
    async fn a_permission_card_has_no_deadline_while_a_provider_terminal_runs_its_pane() {
        let (router, session, parent, _child, root) = native_terminal_fixture().await;
        let state = router.runtime_state();
        let id = format!("native-terminal-{session}");
        let waiting =
            wait_for_permission(&state, &session, &id, &parent, PROVIDER_TERMINAL_HOLD_LIMIT);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !waiting.is_finished(),
            "the person at the provider terminal answers; the 1 s deadline does not deny for them"
        );
        state
            .resolve_runtime_interaction(&session, &id, "allow", None)
            .await
            .unwrap();
        let resolution = waiting.await.unwrap().unwrap();
        assert_eq!(resolution.choice_id.as_deref(), Some("allow"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_sub_agent_card_shown_in_a_provider_terminal_pane_waits_for_its_person() {
        let (router, session, parent, child, root) = native_terminal_fixture().await;
        let state = router.runtime_state();
        let id = format!("native-terminal-child-{session}");
        let waiting =
            wait_for_permission(&state, &session, &id, &child, PROVIDER_TERMINAL_HOLD_LIMIT);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let shown_in = state
            .list_session_health_snapshots()
            .into_iter()
            .find(|room| room.id() == session)
            .unwrap()
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == id)
            .and_then(|interaction| interaction.shown_in_agent_id().map(str::to_owned));
        assert_eq!(
            shown_in.as_deref(),
            Some(parent.as_str()),
            "the child's card shows in its top parent's pane"
        );
        assert!(
            !waiting.is_finished(),
            "the parent's provider terminal holds the card its pane shows, though the child runs in Chariox"
        );
        state
            .resolve_runtime_interaction(&session, &id, "deny", None)
            .await
            .unwrap();
        assert_eq!(
            waiting.await.unwrap().unwrap().choice_id.as_deref(),
            Some("deny")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_held_card_is_settled_by_the_kernel_once_the_hold_limit_passes() {
        let (router, session, parent, _child, root) = native_terminal_fixture().await;
        let state = router.runtime_state();
        let id = format!("native-terminal-limit-{session}");
        // 1 s deadline + 1 s hold stands in for the 300 s timeout and the 23 h hold: the provider's
        // handler outwaits both, so the kernel's settlement is what it reads.
        let waiting = wait_for_permission(&state, &session, &id, &parent, Duration::from_secs(1));
        let resolution = tokio::time::timeout(Duration::from_secs(6), waiting)
            .await
            .expect("the kernel settles a held card at the hold limit; no card waits for nobody")
            .unwrap()
            .unwrap();
        assert_eq!(resolution.choice_id.as_deref(), Some("deny"));
        assert!(!state
            .list_session_health_snapshots()
            .into_iter()
            .find(|room| room.id() == session)
            .unwrap()
            .active_interactions()
            .iter()
            .any(|interaction| interaction.id() == id));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_claude_permission_hook_outwaits_every_card_the_kernel_holds() {
        let handler = include_str!("../provider/claude/native_tui_hook.mjs");
        let wait_ms = crate::provider::CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS * 1000;
        assert!(handler.contains(&format!("const deadline = Date.now() + {wait_ms}")));
        assert!(
            Duration::from_millis(wait_ms)
                > PROVIDER_TERMINAL_HOLD_LIMIT
                    + Duration::from_secs(crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS)
                    + REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER,
            "the hook still waits when the kernel settles a held card"
        );
    }

    #[tokio::test]
    async fn native_permission_queue_counts_waiting_time_against_its_deadline() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let first_id = format!("native-queue-{session}-first");
        let second_id = format!("native-queue-{session}-second");
        let _first = state
            .create_runtime_interaction(
                crate::runtime::state::InteractionSurface::named("provider_native_permission"),
                &session,
                queued_permission(&first_id, &agent, 10),
            )
            .await
            .unwrap();
        let started = tokio::time::Instant::now();
        let resolution = request_runtime_interaction_with_timeout(
            &state,
            &session,
            queued_permission(&second_id, &agent, 1),
            "native_permission_queue_test",
            None,
        )
        .await
        .unwrap();
        assert_eq!(resolution.status, "timed_out");
        assert!(started.elapsed() < Duration::from_millis(1500));
        state
            .resolve_runtime_interaction(&session, &first_id, "deny", None)
            .await
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn native_permission_queue_cancellation_keeps_the_existing_decision() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let shared = format!("shared-{agent}");
        let first = state
            .create_runtime_interaction(
                crate::runtime::state::InteractionSurface::named("provider_native_permission"),
                &session,
                queued_permission(&shared, &agent, 10),
            )
            .await
            .unwrap();
        let waiting_state = state.clone();
        let waiting_session = session.clone();
        let waiting_shared = shared.clone();
        let waiting = tokio::spawn(async move {
            request_runtime_interaction_with_timeout(
                &waiting_state,
                &waiting_session,
                queued_permission(&waiting_shared, &agent, 5),
                "native_permission_queue_test",
                None,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiting.is_finished());
        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        state
            .resolve_runtime_interaction(&session, &shared, "allow", None)
            .await
            .unwrap();
        assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn native_permission_queue_does_not_retry_foreign_session_admission() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let started = tokio::time::Instant::now();
        let result = request_runtime_interaction_with_timeout(
            &state,
            &format!("{session}-foreign"),
            queued_permission("foreign", &agent, 30),
            "native_permission_queue_test",
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        std::fs::remove_dir_all(root).unwrap();
    }
}
