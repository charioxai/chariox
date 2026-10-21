//! Session identity for restarting the official Claude stream-json process.
use super::{ClaudeRuntimeState, RuntimeProviderRun};

pub(super) fn claude_restart_session_argument<'a>(
    run: &'a RuntimeProviderRun,
    state: &'a ClaudeRuntimeState,
) -> Option<(&'static str, &'a str)> {
    // A generated ID alone has no provider conversation. Selection/config
    // changes before the first submitted prompt must keep starting that ID.
    if state.next_turn_number > 1 {
        if let Some(session) = state.session_id.as_deref() {
            return Some(("--resume", session));
        }
    }
    if let Some(session) = run.resume_state().claude_session_id() {
        return Some(("--resume", session));
    }
    state
        .session_id
        .as_deref()
        .map(|session| ("--session-id", session))
        .or_else(|| {
            state
                .args
                .windows(2)
                .find(|pair| pair[0] == "--session-id")
                .map(|pair| ("--session-id", pair[1].as_str()))
        })
}
