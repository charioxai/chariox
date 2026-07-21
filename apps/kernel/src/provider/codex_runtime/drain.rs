use std::time::Duration;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderNativeInteractionBridge, RuntimeProviderRun};

use super::events::{apply_notification_with_manifest, backfill_completed_turn};
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
        apply_notification_with_manifest(
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
            run.remote_extension_manifest(),
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
        apply_notification_with_manifest(
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
            run.remote_extension_manifest(),
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
    if codex_turn_should_backfill(
        run.endpoint_mode(),
        state.active_turn_id.is_some(),
        &state.turn_tracker,
        drained_to_quiet,
    ) {
        backfill_completed_turn(
            &client,
            state,
            run.remote_extension_manifest(),
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        )?;
    }
    if drained_to_quiet && !prompt_completed {
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

pub(super) fn codex_turn_should_backfill(
    endpoint_mode: AgentEndpointMode,
    has_active_turn: bool,
    turn_tracker: &super::turn::CodexTurnTracker,
    drained_to_quiet: bool,
) -> bool {
    has_active_turn
        && (endpoint_mode == AgentEndpointMode::External
            || turn_tracker.has_pending_terminal()
            || (drained_to_quiet && turn_tracker.has_terminal_assistant_evidence()))
}
