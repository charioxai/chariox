//! One request an admitted origin sends to another kernel, with its audit. The Unix
//! socket door and the agents' reach tools both end here.
use super::{ledger, GatewayError, Origin};
use crate::local::LocalDaemonRequest;
use crate::runtime::router::CommandRouter;
use serde_json::Value;

pub(crate) async fn forward_audited(
    router: &CommandRouter,
    origin: &Origin,
    follow_origin: &Origin,
    command_id: &str,
    kernel: &str,
    request: &LocalDaemonRequest,
) -> Result<Value, GatewayError> {
    if crate::runtime::state::is_outside_event_request(request) {
        return Err(GatewayError::Remote {
            name: kernel.into(),
            code: "events_local_only".into(),
            message: "event requests run on your local kernel; use chariox events subscribe --kernel NAME to follow another kernel".into(),
            retryable: false,
        });
    }
    let started = std::time::Instant::now();
    let gateway = router.kernel_gateway();
    if matches!(request, LocalDaemonRequest::RequestKernelSudo(_))
        || matches!(request, LocalDaemonRequest::RoomTerminal(terminal) if matches!(terminal.operation, crate::local::RoomTerminalOperation::ExecOpen { .. }))
    {
        gateway.watch_requester_grant(kernel, origin);
    }
    let forwarded = gateway.forward(
        kernel,
        origin,
        command_id,
        request,
        ledger::deadline_for(request),
    );
    // A wait that outlasts the owner revoking the gateway ends with the revoke.
    let mut answered = if ledger::is_long_wait(request) {
        tokio::select! {
            answered = forwarded => answered,
            error = gateway.until_unaccepted(kernel, origin) => Err(error),
        }
    } else {
        forwarded.await
    };
    // Authorization and audit keep the sudo identity; reach events belong to its live run.
    if let Ok(response) = &mut answered {
        gateway.note_forwarded(kernel, origin, request, response);
        gateway
            .follow_forwarded(follow_origin, kernel, request, response)
            .await;
    }
    if !ledger::is_repeatable(request) {
        router.runtime_state().audit_gateway_forward(
            &origin.grant_id,
            kernel,
            &ledger::variant_name(request),
            answered
                .as_ref()
                .map_or_else(|failure| failure.code(), |_| "ok"),
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
    }
    answered
}

/// The caller owns a forwarded command's retry identity: one retry with the same command id,
/// which the target's ledger answers instead of running the command again.
pub(crate) async fn forward_with_retry<F, Fut>(
    command_id: String,
    mut forward: F,
) -> Result<serde_json::Value, GatewayError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, GatewayError>>,
{
    match forward(command_id.clone()).await {
        Err(error) if matches!(error.code(), "kernel_request_timeout" | "kernel_connection_lost") => {
            match forward(command_id).await {
                Ok(value) => Ok(value),
                Err(_) => Err(GatewayError::Remote {
                    name: "remote kernel".into(),
                    code: "kernel_outcome_unknown".into(),
                    message: "The kernel may have completed this request, but its reply could not be recovered with the original command ID. Read the state before starting another operation.".into(),
                    retryable: false,
                }),
            }
        }
        result => result,
    }
}
