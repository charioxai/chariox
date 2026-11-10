//! MP-08 / MP-09 / MP-10 / MP-11: after a Stop, nothing more of the stopped
//! turn may land. Claude keeps writing its transcript for a moment after the
//! interrupt (or until it notices it); those lines belong to no turn.
//!
//! The fence names the stopped prompt. It holds until another prompt is handed
//! to Claude, whenever that prompt was submitted (a prompt queued before the
//! Stop keeps its submission time), and from that moment on what Claude writes
//! is that prompt's.
use std::fs;

use crate::provider::RuntimeProviderRun;
use crate::session::unix_epoch_ms;

const CANCELLED_TURN_FILE: &str = "cancelled-turn";

fn fence_file(context_file: &str) -> std::path::PathBuf {
    std::path::Path::new(context_file).with_file_name(CANCELLED_TURN_FILE)
}

/// The stopped prompt and, once the next prompt was handed to Claude, when.
struct CancelledTurn {
    prompt_id: String,
    released_at_ms: Option<u64>,
}

fn read_fence(context_file: &str) -> Option<CancelledTurn> {
    let raw = fs::read_to_string(fence_file(context_file)).ok()?;
    let mut lines = raw.lines();
    let prompt_id = lines.next()?.trim().to_string();
    let released_at_ms = lines.next().and_then(|line| line.trim().parse().ok());
    (!prompt_id.is_empty()).then_some(CancelledTurn {
        prompt_id,
        released_at_ms,
    })
}

/// Fences the turn of `prompt_id`, which the person just stopped.
pub(crate) fn fence_cancelled_claude_turn(run: &RuntimeProviderRun, prompt_id: &str) {
    let Some(context_file) = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
        return;
    };
    let _ = fs::write(fence_file(context_file), format!("{prompt_id}\n"));
}

/// `prompt_id` is being handed to Claude: if it is not the stopped prompt,
/// what Claude writes from now on is its own.
pub(super) fn release_claude_cancelled_turn(context_file: &str, prompt_id: &str) {
    let Some(fence) = read_fence(context_file) else {
        return;
    };
    if fence.prompt_id == prompt_id || fence.released_at_ms.is_some() {
        return;
    }
    let _ = fs::write(
        fence_file(context_file),
        format!("{}\n{}\n", fence.prompt_id, unix_epoch_ms()),
    );
}

/// What the transcript drain may keep while a stopped turn is fenced.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum StoppedTurn {
    /// No turn was stopped, or its fence was lifted.
    None,
    /// Everything Claude writes now is the stopped turn's tail.
    Dropping,
    /// The active prompt was handed to Claude at this time; what came before is the tail.
    ReleasedAt(u64),
}

/// The stopped turn as the prompt now active sees it. Once that prompt owns
/// the transcript the fence is lifted.
pub(super) fn claude_stopped_turn(
    context_file: &str,
    active_prompt_id: Option<&str>,
) -> StoppedTurn {
    let Some(fence) = read_fence(context_file) else {
        return StoppedTurn::None;
    };
    match (active_prompt_id, fence.released_at_ms) {
        (Some(active), Some(released_at_ms)) if active != fence.prompt_id => {
            let _ = fs::remove_file(fence_file(context_file));
            StoppedTurn::ReleasedAt(released_at_ms)
        }
        _ => StoppedTurn::Dropping,
    }
}

