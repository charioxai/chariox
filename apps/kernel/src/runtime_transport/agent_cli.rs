//! The CLI inside a provider process speaks for that provider's agent.
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::local::{KernelConnectionClass, LocalDaemonRequest};
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::{KernelOutgoingFrame, KernelTransportError};

const PREFIX: &str = "chx_act_";
const LABEL: &[u8] = b"chariox-agent-cli-v1\0";

pub(crate) fn token_for_run(run: &RuntimeProviderRun) -> Option<String> {
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(authenticator(run)?.finalize().into_bytes());
    Some(format!("{PREFIX}{}.{signature}", run.id()))
}

pub(super) fn admit(router: &CommandRouter, authorization: &str) -> Option<String> {
    let credential = authorization
        .strip_prefix("Bearer ")?
        .strip_prefix(PREFIX)?;
    let (run_id, signature) = credential.split_once('.')?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature)
        .ok()?;
    let run = router.active_agent_cli_run(run_id)?;
    authenticator(&run)?.verify_slice(&signature).ok()?;
    Some(run_id.to_string())
}

fn authenticator(run: &RuntimeProviderRun) -> Option<Hmac<Sha256>> {
    // Every run of an agent has its own credential, whatever the way the kernel
    // talks to the provider (managed terminal or structured endpoint): the
    // default Claude, Codex and OpenCode launches are structured endpoints.
    if run.state() == ProviderRunState::Ended {
        return None;
    }
    run.runtime_mcp_server_url()?;
    let agent = run.agent_instance_id()?;
    let key = run.runtime_mcp_auth_token()?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).ok()?;
    // Separate this credential from the MCP bearer and bind it to the launch.
    for component in [LABEL, run.id().as_bytes(), b"\0", agent.as_bytes()] {
        mac.update(component);
    }
    Some(mac)
}

pub(super) fn caller(run: &RuntimeProviderRun) -> KernelCaller {
    let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    caller.caller_id = format!("agent-cli:{}", run.id());
    caller.caller_kind = KernelCallerKind::Metaagent;
    caller.user_id = Some(run.owner_user_id().to_string());
    caller.metaagent_id = run.agent_instance_id().map(str::to_string);
    caller
}

pub(super) fn permits_peer_request(
    router: &CommandRouter,
    run: &RuntimeProviderRun,
    request: &LocalDaemonRequest,
) -> bool {
    if !matches!(
        request,
        LocalDaemonRequest::SpawnAgent(_)
            | LocalDaemonRequest::CancelActivePrompt(_)
            | LocalDaemonRequest::GetAgentDeadline(_)
            | LocalDaemonRequest::SetAgentDeadline(_)
            | LocalDaemonRequest::DestroyAgent(_)
    ) {
        return false;
    }
    let state = router.runtime_state();
    state.room_agent_tools_enabled()
        && run
            .agent_instance_id()
            .is_some_and(|agent| state.authorize_room_agent_request(agent, request).is_ok())
}

pub(super) fn room_refusal(request_id: String) -> KernelOutgoingFrame {
    KernelOutgoingFrame::Response {
        kernel: None,
        request_id,
        response: Box::new(None),
        error: Some(KernelTransportError {
            code: "kernel_access_denied".into(),
            message: "This agent CLI credential is scoped to its own Room.".into(),
            retryable: false,
        }),
    }
}
