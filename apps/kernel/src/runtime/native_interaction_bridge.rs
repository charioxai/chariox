use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, NativeProviderInteractionResolution,
    RequestNativeProviderTurnInteractionRequest,
};
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
    ProviderProcessServiceStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{RuntimeInteraction, RuntimeInteractionKind};
use crate::transport::relay_peer::RemoteNativeInteractionContext;

pub(crate) const REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER: Duration = Duration::from_secs(15);
// A Claude permission hook outwaits a forwarded interaction's timeout and relay
// buffer, with room for event pickup and the relay connection, so the deny it
// resolves to still reaches Claude.
const _: () = assert!(
    crate::provider::CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS
        >= crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS
            + REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER.as_secs()
            + 15
);

struct RuntimeStateNativeInteractionBridge {
    handle: Handle,
    state: KernelRuntimeState,
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
        let remote_target = self.handle.block_on(async {
            state
                .remote_native_interaction_context(&session_id, &interaction_agent_id, &origin)
                .await
        })?;
        if let Some((config, target_daemon_id, context)) = remote_target {
            return self.handle.block_on(forward_native_interaction_to_home(
                &state,
                config,
                target_daemon_id,
                context,
                interaction,
            ));
        }
        let state = self.state.clone();
        self.handle
            .block_on(request_runtime_interaction_with_timeout(
                &state,
                &session_id,
                interaction,
                "provider_native_interaction_bridge",
                None,
            ))
    }
}

pub(crate) async fn forward_native_interaction_to_home(
    state: &KernelRuntimeState,
    config: crate::config::DaemonConfig,
    target_daemon_id: String,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let response_timeout =
        remote_native_interaction_response_timeout(&interaction, config.relay_request_timeout_ms);
    forward_native_interaction_with_timeout(
        state,
        config,
        target_daemon_id,
        context,
        interaction,
        response_timeout,
    )
    .await
}

/// Forwards one interaction with its original identity, retrying recoverable
/// transport failures until `response_timeout`.
pub(crate) async fn forward_native_interaction_with_timeout(
    state: &KernelRuntimeState,
    config: crate::config::DaemonConfig,
    target_daemon_id: String,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
    response_timeout: Duration,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let request = crate::transport::relay_peer::RelayPeerRequest::ForwardNativeTurnInteraction {
        context,
        interaction,
    };
    let response = retry::retry_forwarded_native_interaction(response_timeout, |remaining| {
        state.send_worker_home_runtime_request(
            &config,
            &target_daemon_id,
            request.clone(),
            remaining,
        )
    })
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
    if let Some((config, target_daemon_id, context)) = state
        .remote_native_interaction_context(session_id, &agent_id, &origin)
        .await?
    {
        return forward_native_interaction_to_home(
            state,
            config,
            target_daemon_id,
            context,
            interaction,
        )
        .await;
    }
    request_runtime_interaction_with_timeout(state, session_id, interaction, operation, None).await
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

fn remote_native_interaction_response_timeout(
    interaction: &RuntimeInteraction,
    relay_request_timeout_ms: u64,
) -> Duration {
    interaction
        .timeout_sec()
        .map(Duration::from_secs)
        .map(|timeout| timeout.saturating_add(REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER))
        .unwrap_or_else(|| Duration::from_millis(relay_request_timeout_ms))
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

/// `vault-…` ids are the kernel's own Vault prompts, whose lifetime rules (and
/// panel exemption) a worker- or API-chosen id must not borrow.
fn reject_vault_interaction_id(id: &str) -> Result<(), DaemonError> {
    if id.starts_with("vault-") {
        return Err(DaemonError::LocalTransport {
            operation: "native_provider_interaction",
            message: "interaction ids starting with `vault-` are reserved".to_string(),
        });
    }
    Ok(())
}

pub(crate) async fn execute_native_provider_interaction_request(
    state: &KernelRuntimeState,
    request: RequestNativeProviderTurnInteractionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    reject_vault_interaction_id(&request.interaction_id)?;
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
    reject_vault_interaction_id(interaction.id())?;
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
    state
        .resumable_forwarded_native_interaction(context, interaction)
        .await
}

pub(super) async fn request_runtime_interaction_with_timeout(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
    forwarding: Option<&RemoteNativeInteractionContext>,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
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
        match state
            .create_runtime_interaction_with_forwarding(session_id, interaction.clone(), forwarding)
            .await
        {
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
    // Waiting for another tool consumes this decision's original budget.
    let resolution = if let Some(deadline) = deadline {
        tokio::select! {
            biased;
            resolution = &mut receiver => resolution,
            _ = tokio::time::sleep_until(deadline) => {
                state.timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id).await?;
                receiver.await
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

mod retry;

#[cfg(test)]
mod tests;
