use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::provider::ProviderRunTokenUsage;
use crate::terminal::TerminalOutputKind;

use super::events::apply_notification;
use super::transcript::{render_codex_tool_transcript_update, CodexToolTranscriptState};
use super::turn::{maybe_finalize_terminal_signal, CodexTurnTracker};
use super::{CodexAssistantCompletion, CodexOutputChunk};
use crate::provider::CodexNotification;

fn flush_quiet_terminal_for_test(
    active_turn_id: &mut Option<String>,
    turn_tracker: &mut CodexTurnTracker,
    completions: &mut Vec<CodexAssistantCompletion>,
    notices: &mut Vec<String>,
    prompt_completed: &mut bool,
    terminal_failure: &mut Option<String>,
) {
    turn_tracker.force_pending_terminal_quiet_for_tests();
    maybe_finalize_terminal_signal(
        active_turn_id,
        turn_tracker,
        completions,
        notices,
        prompt_completed,
        terminal_failure,
    );
}

fn parse_tool_chunk(chunk: &CodexOutputChunk) -> Value {
    serde_json::from_slice(&chunk.bytes).expect("tool chunk should be JSON")
}

mod transcript_projection;
mod turn_completion;
