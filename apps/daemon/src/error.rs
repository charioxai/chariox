use thiserror::Error;

use crate::provider::ProviderRunState;
use crate::session::SessionStatus;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("invalid daemon configuration for `{field}`: {message}")]
    InvalidConfig {
        field: &'static str,
        message: &'static str,
    },
    #[error("failed while waiting for daemon shutdown signal: {0}")]
    ShutdownSignal(std::io::Error),
    #[error("session `{session_id}` was not found")]
    SessionNotFound { session_id: String },
    #[error("invalid session transition for `{session_id}`: {from} -> {to}")]
    InvalidSessionTransition {
        session_id: String,
        from: SessionStatus,
        to: SessionStatus,
    },
    #[error("attachment `{attachment_id}` was not found")]
    AttachmentNotFound { attachment_id: String },
    #[error("attachment `{attachment_id}` does not belong to session `{session_id}`")]
    AttachmentNotInSession {
        session_id: String,
        attachment_id: String,
    },
    #[error("session `{session_id}` cannot perform `{operation}` while {status}")]
    SessionOperationNotAllowed {
        session_id: String,
        status: SessionStatus,
        operation: &'static str,
    },
    #[error("session `{session_id}` has no active prompt")]
    NoActivePrompt { session_id: String },
    #[error("session `{session_id}` rejected the config change while a prompt is running")]
    ConfigChangeRejectedWhilePromptRunning { session_id: String },
    #[error("provider adapter `{adapter_key}` was not found")]
    ProviderAdapterNotFound { adapter_key: String },
    #[error("provider run `{provider_run_id}` was not found")]
    ProviderRunNotFound { provider_run_id: String },
    #[error("provider run `{provider_run_id}` does not belong to session `{session_id}`")]
    ProviderRunNotInSession {
        session_id: String,
        provider_run_id: String,
    },
    #[error("provider run `{provider_run_id}` cannot perform `{operation}` while {state}")]
    InvalidProviderRunState {
        provider_run_id: String,
        state: ProviderRunState,
        operation: &'static str,
    },
    #[error(
        "session `{session_id}` has inconsistent active provider run state: active={active_provider_run_id:?}, requested={requested_provider_run_id}`"
    )]
    InconsistentActiveProviderRun {
        session_id: String,
        active_provider_run_id: Option<String>,
        requested_provider_run_id: String,
    },
    #[error("session `{session_id}` has no active provider run")]
    NoActiveProviderRun { session_id: String },
    #[error("provider run `{provider_run_id}` has no PTY process")]
    PtyProcessNotFound { provider_run_id: String },
    #[error("failed to spawn PTY for provider run `{provider_run_id}`: {message}")]
    PtySpawn {
        provider_run_id: String,
        message: String,
    },
    #[error("failed to clean up PTY for provider run `{provider_run_id}`: {message}")]
    PtyCleanup {
        provider_run_id: String,
        message: String,
    },
    #[error("failed to write PTY input for provider run `{provider_run_id}`: {message}")]
    PtyWrite {
        provider_run_id: String,
        message: String,
    },
    #[error(
        "failed to resize PTY for provider run `{provider_run_id}` to {cols}x{rows}: {message}"
    )]
    PtyResize {
        provider_run_id: String,
        cols: u16,
        rows: u16,
        message: String,
    },
    #[error("shell command `{command}` failed for session `{session_id}`: {message}")]
    ShellCommandFailed {
        session_id: String,
        command: String,
        message: String,
    },
    #[error("local harness timed out waiting for terminal output for session `{session_id}` after {timeout_ms}ms")]
    LocalHarnessTimeout { session_id: String, timeout_ms: u64 },
}
