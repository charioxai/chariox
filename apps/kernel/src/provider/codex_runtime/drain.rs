use std::time::Duration;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderNativeInteractionBridge, RuntimeProviderRun};

use super::events::{apply_notification, backfill_external_completed_turn};
use super::run_config::codex_client_for_run;
use super::turn::maybe_finalize_terminal_signal;
use super::{CodexPollResult, CodexRuntimeState};

const CODEX_EVENT_DRAIN_READ_TIMEOUT: Duration = Duration::from_millis(1);
const CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS: usize = 64;

pub fn drain_codex_events(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexPollResult, DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), native_interaction_bridge)?;
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    for notification in std::mem::take(&mut state.buffered_notifications) {
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut state.text_items,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
    }

    let mut drained_to_quiet = true;
    for _ in 0..CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS {
        let Some(notification) =
            client.read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
        else {
            break;
        };
        drained_to_quiet = false;
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut state.text_items,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
    }
    if !drained_to_quiet {
        drained_to_quiet = client
            .read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
            .map(|notification| {
                state.buffered_notifications.push(notification);
            })
            .is_none();
    }
    if run.endpoint_mode() == AgentEndpointMode::External
        && state.active_turn_id.is_some()
        && !state.turn_tracker.has_pending_terminal()
    {
        backfill_external_completed_turn(
            &client,
            state,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        )?;
    }
    if drained_to_quiet {
        maybe_finalize_terminal_signal(
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
    }

    Ok(CodexPollResult {
        chunks,
        completions,
        prompt_completed,
        terminal_failure,
        notices,
        resolved_usage,
    })
}
