use std::time::Duration;

use crate::error::DaemonError;
use crate::session::PromptAttachment;
use crate::terminal::TerminalOutputKind;

use super::{CodexClient, CodexNotification, CodexRunSelection, CodexSocket, RuntimeProviderRun};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexPollResult {
    pub chunks: Vec<CodexOutputChunk>,
    pub prompt_completed: bool,
    pub provider_idle: bool,
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexOutputChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

pub struct CodexRuntimeState {
    endpoint: String,
    thread_id: String,
    socket: CodexSocket,
    next_request_id: u64,
    buffered_notifications: Vec<CodexNotification>,
    active_turn_id: Option<String>,
}

impl std::fmt::Debug for CodexRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexRuntimeState")
            .field("endpoint", &self.endpoint)
            .field("thread_id", &self.thread_id)
            .field("next_request_id", &self.next_request_id)
            .field("buffered_notifications", &self.buffered_notifications)
            .field("active_turn_id", &self.active_turn_id)
            .finish()
    }
}

impl CodexRuntimeState {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }
}

pub fn initialize_codex_runtime(
    run: &RuntimeProviderRun,
) -> Result<(CodexRuntimeState, CodexRunSelection), DaemonError> {
    let endpoint = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_endpoint_missing",
            message: "codex run did not expose a structured endpoint".to_string(),
        })?
        .to_string();
    let client = CodexClient::new(run.id(), &endpoint)?;
    let mut socket = client.connect_initialized()?;
    let mut next_request_id = 1;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let thread = client.thread_start(
        &mut socket,
        &mut next_request_id,
        cwd.as_deref(),
        model.as_deref(),
    )?;
    let selection = CodexRunSelection {
        model: Some(format!("codex/{}", thread.model)),
        variant: thread.reasoning_effort,
    };
    Ok((
        CodexRuntimeState {
            endpoint,
            thread_id: thread.thread.id,
            socket,
            next_request_id,
            buffered_notifications: Vec::new(),
            active_turn_id: None,
        },
        selection,
    ))
}

pub fn submit_codex_prompt(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Result<(), DaemonError> {
    let client = CodexClient::new(run.id(), state.endpoint())?;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    let input = codex_input(prompt, attachments);
    let thread_id = state.thread_id.clone();
    let _ = client.turn_start(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        cwd.as_deref(),
        model.as_deref(),
        effort.as_deref(),
        input,
        &mut state.buffered_notifications,
    )?;
    Ok(())
}

pub fn abort_codex_turn(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<(), DaemonError> {
    let Some(turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    client.turn_interrupt(&mut state.socket, &mut state.next_request_id, &turn_id)
}

pub fn drain_codex_events(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<CodexPollResult, DaemonError> {
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    let mut chunks = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut provider_idle = false;

    while let Some(notification) = state.buffered_notifications.pop() {
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut chunks,
            &mut notices,
            &mut prompt_completed,
            &mut provider_idle,
        );
    }

    loop {
        let Some(notification) =
            client.read_notification(&mut state.socket, Duration::from_millis(25))?
        else {
            break;
        };
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut chunks,
            &mut notices,
            &mut prompt_completed,
            &mut provider_idle,
        );
    }

    Ok(CodexPollResult {
        chunks,
        prompt_completed,
        provider_idle,
        notices,
    })
}

fn apply_notification(
    notification: CodexNotification,
    active_turn_id: &mut Option<String>,
    chunks: &mut Vec<CodexOutputChunk>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    provider_idle: &mut bool,
) {
    match notification {
        CodexNotification::AgentMessageDelta { delta, .. } => {
            if delta.is_empty() {
                return;
            }
            chunks.push(CodexOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("codex-agent-message".to_string()),
                bytes: delta.into_bytes(),
            });
        }
        CodexNotification::TurnStarted { turn_id } => {
            if !turn_id.is_empty() {
                *active_turn_id = Some(turn_id);
            }
        }
        CodexNotification::TurnCompleted {
            turn_id,
            status,
            error_message,
        } => {
            if !turn_id.is_empty() {
                *active_turn_id = None;
            }
            *prompt_completed = true;
            *provider_idle = true;
            if let Some(message) = error_message {
                notices.push(message);
            } else if status == "failed" {
                notices.push("Codex turn failed".to_string());
            }
        }
        CodexNotification::Error { message } => {
            notices.push(message);
        }
        _ => {}
    }
}

fn codex_input(prompt: &str, attachments: &[PromptAttachment]) -> Vec<serde_json::Value> {
    let mut input = Vec::new();
    if !prompt.trim().is_empty() {
        input.push(serde_json::json!({
            "type": "text",
            "text": prompt,
        }));
    }

    let mut attachment_notes = Vec::new();
    for attachment in attachments {
        if attachment.mime().starts_with("image/") {
            if attachment.url().starts_with('/') {
                input.push(serde_json::json!({
                    "type": "localImage",
                    "path": attachment.url(),
                }));
            } else {
                input.push(serde_json::json!({
                    "type": "image",
                    "url": attachment.url(),
                }));
            }
            continue;
        }
        let label = attachment
            .filename()
            .map(str::to_string)
            .unwrap_or_else(|| attachment.url().to_string());
        attachment_notes.push(format!(
            "Attachment: {label} ({}) at {}",
            attachment.mime(),
            attachment.url()
        ));
    }

    if !attachment_notes.is_empty() {
        input.push(serde_json::json!({
            "type": "text",
            "text": attachment_notes.join("\n"),
        }));
    }

    input
}

fn normalize_codex_model(model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() || model == "default" {
        return None;
    }
    Some(model.strip_prefix("codex/").unwrap_or(model).to_string())
}

fn normalize_variant(variant: Option<&str>) -> Option<String> {
    variant
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default")
        .map(str::to_string)
}
