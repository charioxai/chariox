//! The structured Claude prompt hook requests context for a provider-started turn
//! and reports delivery of the context the kernel sent.
use std::io::{Read, Seek, SeekFrom};

use crate::provider::{ClaudeNotificationContext, ProviderPromptSignalBatch};

use super::ClaudeRuntimeState;

const EVENTS_FILE_ENV: &str = "CHARIOX_CLAUDE_NATIVE_EVENTS";
const DELIVERED_EVENT: &str = "CharioxContextDelivered";
const NOTIFICATION_REQUEST: &str = "\"notification_context\"";
/// One poll reads at most this much of new events; the rest waits for the next poll.
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn read_prompt_hook_events(
    state: &mut ClaudeRuntimeState,
    batch: &mut ProviderPromptSignalBatch,
) -> std::io::Result<()> {
    let Some(path) = state.env.get(EVENTS_FILE_ENV) else {
        return Ok(());
    };
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    if length < state.hook_events_offset {
        state.hook_events_offset = 0;
    }
    if length == state.hook_events_offset {
        return Ok(());
    }
    file.seek(SeekFrom::Start(state.hook_events_offset))?;
    let mut fresh = Vec::new();
    file.take(MAX_READ_BYTES).read_to_end(&mut fresh)?;
    // The hook may still be writing the last line.
    let Some(complete) = fresh.iter().rposition(|byte| *byte == b'\n') else {
        return Ok(());
    };
    for line in fresh[..complete].split(|byte| *byte == b'\n') {
        let event = [DELIVERED_EVENT, NOTIFICATION_REQUEST]
            .into_iter()
            .any(|key| {
                line.windows(key.len())
                    .any(|window| window == key.as_bytes())
            })
            .then(|| serde_json::from_slice::<serde_json::Value>(line).ok())
            .flatten();
        if let Some(event) = event {
            if event["hook_event_name"] == DELIVERED_EVENT {
                batch.hook_context_delivered = true;
            }
            if let Some(request) = notification_request(&event) {
                // Leave the request unread until the preceding turn's completion
                // batch has settled. The hook is still waiting on this request.
                if batch.prompt_completed {
                    break;
                }
                let dispatched = state.active_prompt_message.as_ref().is_some_and(|message| {
                    message["message"]["content"][0]["text"]
                        .as_str()
                        .zip(event["prompt"].as_str())
                        .is_some_and(|(left, right)| left.trim() == right.trim())
                });
                if dispatched {
                    // A person may paste the same markup Claude uses for notifications.
                    reply_with_dispatched_context(state, &request)?;
                } else {
                    if state.active_turn_id.is_none() {
                        super::events::begin_own_turn(state, batch);
                    }
                    batch.claude_notification_context = Some(request);
                }
            }
        }
        state.hook_events_offset += line.len() as u64 + 1;
    }
    Ok(())
}

fn notification_request(event: &serde_json::Value) -> Option<ClaudeNotificationContext> {
    if event["hook_event_name"] != "UserPromptSubmit" || event["notification_context"] != true {
        return None;
    }
    let deadline_ms = event["hook_context_deadline_ms"].as_u64()?;
    (deadline_ms > crate::session::unix_epoch_ms()).then_some(ClaudeNotificationContext {
        request_id: event["hook_context_request_id"].as_str()?.into(),
        deadline_ms,
        ended_tool_use_id: event["prompt"]
            .as_str()
            .and_then(super::task_notification::finished_background_task_text)
            .and_then(|end| end.tool_use_id)
            .map(str::to_owned),
    })
}

fn reply_with_dispatched_context(
    state: &ClaudeRuntimeState,
    request: &ClaudeNotificationContext,
) -> std::io::Result<()> {
    let file = state
        .context_file
        .as_deref()
        .and_then(std::path::Path::to_str)
        .ok_or_else(|| std::io::Error::other("Missing Claude dispatched context file"))?;
    let context = std::fs::read_to_string(file)?;
    crate::app::write_claude_hook_context_response(file, &request.request_id, &context)
        .map_err(std::io::Error::other)
}
