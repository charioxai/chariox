//! MP-08 / MP-09 / MP-10 / MP-11: after a Stop, nothing more of the stopped
//! turn may land. Claude keeps writing its transcript for a moment after the
//! interrupt (or until it notices it); those lines belong to no turn.
use std::fs;

use crate::provider::RuntimeProviderRun;
use crate::session::unix_epoch_ms;

const CANCELLED_TURN_FILE: &str = "cancelled-turn";

fn fence_file(context_file: &str) -> std::path::PathBuf {
    std::path::Path::new(context_file).with_file_name(CANCELLED_TURN_FILE)
}

/// Marks the moment the person stopped the turn this run is working on.
pub(crate) fn fence_cancelled_claude_turn(run: &RuntimeProviderRun) {
    let Some(context_file) = run.pty_env().get("CHARIOX_CLAUDE_NATIVE_CONTEXT") else {
        return;
    };
    let _ = fs::write(fence_file(context_file), unix_epoch_ms().to_string());
}

/// When the stopped turn was cancelled, while its tail may still arrive.
pub(super) fn claude_cancelled_turn_fence_ms(context_file: &str) -> Option<u64> {
    fs::read_to_string(fence_file(context_file))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// A prompt started after the Stop owns everything Claude writes from now on.
pub(super) fn lift_claude_cancelled_turn_fence(context_file: &str) {
    let _ = fs::remove_file(fence_file(context_file));
}
