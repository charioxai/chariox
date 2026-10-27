use std::collections::VecDeque;
use std::sync::Arc;

use super::json_size::{scoped_output_record, terminal_output_record_scoped_json_bytes};
use super::{
    TerminalOutputAppend, TerminalOutputExternalObservationMetadata, TerminalOutputKind,
    TerminalOutputRecord, TerminalStreamService, TerminalStreamStore,
};
use crate::history::{
    SessionHistoryEntryKind, SessionHistoryEntrySource, SessionHistoryExternalObservation,
};

fn external_observed_metadata(
    provider_turn_id: &str,
) -> (
    String,
    TerminalOutputExternalObservationMetadata,
    Option<String>,
) {
    let entry = crate::history::SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("provider-run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "external output",
        "codex",
        "thread-1",
        Some(provider_turn_id.to_string()),
        Some(1_234),
    );
    (
        entry
            .merge_key
            .clone()
            .expect("external observed entry should have merge key"),
        TerminalOutputExternalObservationMetadata::from_session_history_entry(&entry)
            .expect("external observed entry should produce terminal metadata"),
        entry.source_attachment_id.clone(),
    )
}

mod service;
mod store;
