//! The structured Claude process runs the Chariox prompt hook. The hook appends one
//! JSON line per event to a file; its `CharioxContextDelivered` line is the only
//! proof that the dispatched additionalContext, and any skill body in it, reached
//! the model.
use std::io::{Read, Seek, SeekFrom};

use super::ClaudeRuntimeState;

const EVENTS_FILE_ENV: &str = "CHARIOX_CLAUDE_NATIVE_EVENTS";
const DELIVERED_EVENT: &str = "CharioxContextDelivered";
/// One poll reads at most this much of new events; the rest waits for the next poll.
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

/// Whether a complete delivery receipt line has appeared since the previous call.
pub(super) fn take_context_delivered(state: &mut ClaudeRuntimeState) -> std::io::Result<bool> {
    let Some(path) = state.env.get(EVENTS_FILE_ENV) else {
        return Ok(false);
    };
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    if length < state.hook_events_offset {
        state.hook_events_offset = 0;
    }
    if length == state.hook_events_offset {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(state.hook_events_offset))?;
    let mut fresh = Vec::new();
    file.take(MAX_READ_BYTES).read_to_end(&mut fresh)?;
    // The hook may still be writing the last line.
    let Some(complete) = fresh.iter().rposition(|byte| *byte == b'\n') else {
        return Ok(false);
    };
    state.hook_events_offset += complete as u64 + 1;
    Ok(fresh[..complete]
        .split(|byte| *byte == b'\n')
        .filter(|line| {
            line.windows(DELIVERED_EVENT.len())
                .any(|w| w == DELIVERED_EVENT.as_bytes())
        })
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .any(|event| event["hook_event_name"] == DELIVERED_EVENT))
}
