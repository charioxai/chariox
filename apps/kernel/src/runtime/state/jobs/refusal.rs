use crate::{error::DaemonError, transport::runtime_tools::RuntimeToolResult};

const JOB_REFUSED: &str = "job_refused";

pub(in crate::runtime::state) fn is_job_tool(name: &str) -> bool {
    name.starts_with("chariox.jobs.")
}

pub(super) fn for_relay(error: DaemonError) -> DaemonError {
    match error {
        DaemonError::LocalTransport {
            operation: "agent.lifecycle",
            message,
        } => DaemonError::ExternalRequestFailed {
            code: JOB_REFUSED.into(),
            message,
            retryable: false,
        },
        other => other,
    }
}

pub(in crate::runtime::state) fn tool_reply(
    name: &str,
    reply: Result<RuntimeToolResult, DaemonError>,
) -> Result<RuntimeToolResult, DaemonError> {
    match reply {
        Err(error) if is_job_tool(name) => match for_relay(error) {
            DaemonError::MachineRequest { code, message }
            | DaemonError::ExternalRequestFailed {
                code,
                message,
                retryable: false,
            }
            | DaemonError::RelayTransport {
                code,
                message,
                retryable: false,
                ..
            } if matches!(code.as_str(), JOB_REFUSED | "job_invalid") => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": crate::secret_redaction::redact_secrets(&message),
                    "retryable": false,
                }),
            }),
            error => Err(error),
        },
        reply => reply,
    }
}
