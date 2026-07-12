use std::collections::BTreeSet;

use crate::error::DaemonError;
use crate::history::{
    HistoryEvent, HistoryEventKind, SessionHistoryEntryKind, STEERING_PROMPT_MERGE_KEY_PREFIX,
};
use crate::local::SessionHistoryOutlineBlob;
use crate::session_history_page::SessionHistoryPageEntry;

const BLOB_ID_PREFIX: &str = "history";
pub(super) const MAX_OUTLINE_INLINE_CHARS: usize = 16 * 1024;
pub(super) const MAX_OUTLINE_EVENTS_PER_BLOB: usize = 256;
pub(super) const MAX_OUTLINE_INLINE_ENTRIES_PER_TURN: usize = 32;

pub(super) fn event_projects_as_outline_entry(event: &HistoryEvent) -> bool {
    match event.kind {
        HistoryEventKind::ProviderOutput => true,
        HistoryEventKind::ProviderStatus => event_provider_status_projects_as_outline_entry(event),
        HistoryEventKind::UserPrompt => is_steering_prompt_event(event),
        _ => false,
    }
}

fn event_provider_status_projects_as_outline_entry(event: &HistoryEvent) -> bool {
    let Some(entry) = event.to_session_history_entry() else {
        return false;
    };
    if !entry.is_external_provider_observed() {
        return false;
    }
    if entry
        .external_observation
        .as_ref()
        .is_some_and(|observation| observation.passive_telemetry)
    {
        return false;
    }
    true
}

fn event_projects_as_outline_blob(event: &HistoryEvent) -> bool {
    if is_steering_prompt_event(event) {
        return false;
    }
    if event_needs_outline_blob(event) {
        return true;
    }
    match event.kind {
        HistoryEventKind::ProviderOutput | HistoryEventKind::ProviderStatus => false,
        _ => true,
    }
}

fn is_steering_prompt_event(event: &HistoryEvent) -> bool {
    event.kind == HistoryEventKind::UserPrompt
        && event
            .metadata
            .get("merge_key")
            .and_then(|value| value.as_str())
            .is_some_and(|merge_key| merge_key.starts_with(STEERING_PROMPT_MERGE_KEY_PREFIX))
}

fn outline_blob_from_event(event: HistoryEvent) -> Option<SessionHistoryOutlineBlob> {
    let entry = event.to_session_history_entry()?;
    if entry.is_external_provider_observed_state_signal() {
        return None;
    }
    let total_chars = entry.text.chars().count();
    Some(SessionHistoryOutlineBlob {
        blob_id: blob_id(event.sequence, event.sequence),
        kind: entry.kind,
        title: blob_title(entry.kind, &entry.text),
        summary: blob_summary(entry.kind, &entry.text),
        sequence_start: event.sequence,
        sequence_end: event.sequence,
        entry_count: 1,
        total_chars,
        timestamp_ms: event.timestamp_ms,
    })
}

pub(super) fn outline_blobs_from_events(
    events: &[HistoryEvent],
    prompt_sequence: u64,
    summary_sequence: Option<u64>,
    forced_blob_sequences: &BTreeSet<u64>,
) -> Vec<SessionHistoryOutlineBlob> {
    let candidates = events
        .iter()
        .filter(|event| event.sequence != prompt_sequence)
        .filter(|event| Some(event.sequence) != summary_sequence || event_needs_outline_blob(event))
        .filter(|event| {
            forced_blob_sequences.contains(&event.sequence) || event_projects_as_outline_blob(event)
        })
        .filter(|event| {
            event
                .to_session_history_entry()
                .is_some_and(|entry| !entry.is_external_provider_observed_state_signal())
        })
        .collect::<Vec<_>>();
    let mut blobs = Vec::new();
    for group in outline_blob_event_groups(&candidates) {
        blobs.extend(
            group
                .chunks(MAX_OUTLINE_EVENTS_PER_BLOB)
                .filter_map(outline_blob_from_event_group),
        );
    }
    blobs
}

fn outline_blob_event_groups<'a>(events: &[&'a HistoryEvent]) -> Vec<Vec<&'a HistoryEvent>> {
    let mut groups = Vec::<Vec<&HistoryEvent>>::new();
    for event in events {
        let kind = event.to_session_history_entry().map(|entry| entry.kind);
        let same_kind = groups
            .last()
            .and_then(|group| group.first())
            .and_then(|first| first.to_session_history_entry().map(|entry| entry.kind))
            == kind;
        if same_kind {
            if let Some(group) = groups.last_mut() {
                group.push(*event);
            }
        } else {
            groups.push(vec![*event]);
        }
    }
    groups
}

fn outline_blob_from_event_group(events: &[&HistoryEvent]) -> Option<SessionHistoryOutlineBlob> {
    let first_event = events.first()?;
    if events.len() == 1 {
        return outline_blob_from_event((*first_event).clone());
    }
    let first_entry = first_event.to_session_history_entry()?;
    let sequence_start = events.iter().map(|event| event.sequence).min()?;
    let sequence_end = events.iter().map(|event| event.sequence).max()?;
    let total_chars = events
        .iter()
        .filter_map(|event| event.to_session_history_entry())
        .map(|entry| entry.text.chars().count())
        .sum();
    let timestamp_ms = events
        .iter()
        .map(|event| event.timestamp_ms)
        .max()
        .unwrap_or(first_event.timestamp_ms);
    Some(SessionHistoryOutlineBlob {
        blob_id: blob_id(sequence_start, sequence_end),
        kind: first_entry.kind,
        title: format!("{} trace entries", events.len()),
        summary: format!("{} entries, {} chars", events.len(), total_chars),
        sequence_start,
        sequence_end,
        entry_count: events.len(),
        total_chars,
        timestamp_ms,
    })
}

pub(super) fn page_entry_from_event(event: HistoryEvent) -> Option<SessionHistoryPageEntry> {
    page_entry_from_event_with_inline_limit(event, None)
}

pub(super) fn outline_page_entry_from_event(
    event: HistoryEvent,
) -> Option<SessionHistoryPageEntry> {
    page_entry_from_event_with_inline_limit(event, Some(MAX_OUTLINE_INLINE_CHARS))
}

fn page_entry_from_event_with_inline_limit(
    event: HistoryEvent,
    max_inline_chars: Option<usize>,
) -> Option<SessionHistoryPageEntry> {
    let mut entry = event.to_session_history_entry()?;
    if entry.is_external_provider_observed_state_signal() {
        return None;
    }
    for attachment in &mut entry.attachments {
        attachment.rehydrate_preview_url();
    }
    let total_chars = entry.text.chars().count();
    let fragment_end = max_inline_chars
        .map(|limit| total_chars.min(limit))
        .unwrap_or(total_chars);
    if fragment_end < total_chars {
        entry.text = entry.text.chars().take(fragment_end).collect();
    }
    Some(SessionHistoryPageEntry {
        entry_index: event.sequence as usize,
        fragment_start: 0,
        fragment_end,
        total_chars,
        entry,
    })
}

pub(super) fn event_needs_outline_blob(event: &HistoryEvent) -> bool {
    event.to_session_history_entry().is_some_and(|entry| {
        !entry.is_external_provider_observed_state_signal()
            && entry.text.chars().count() > MAX_OUTLINE_INLINE_CHARS
    })
}

pub(super) fn has_content(event: &HistoryEvent) -> bool {
    event
        .content
        .as_deref()
        .is_some_and(|content| !content.trim().is_empty())
}

pub(super) fn blob_id(sequence_start: u64, sequence_end: u64) -> String {
    format!("{BLOB_ID_PREFIX}:{sequence_start}:{sequence_end}")
}

pub(super) fn parse_blob_id(blob_id: &str) -> Result<(u64, u64), DaemonError> {
    let mut parts = blob_id.split(':');
    let prefix = parts.next();
    let sequence_start = parts.next().and_then(|value| value.parse::<u64>().ok());
    let sequence_end = parts.next().and_then(|value| value.parse::<u64>().ok());
    if prefix == Some(BLOB_ID_PREFIX) && parts.next().is_none() {
        if let (Some(sequence_start), Some(sequence_end)) = (sequence_start, sequence_end) {
            return Ok((sequence_start, sequence_end));
        }
    }
    Err(DaemonError::LocalTransport {
        operation: "parse history blob id",
        message: format!("invalid history blob id `{blob_id}`"),
    })
}

fn blob_title(kind: SessionHistoryEntryKind, text: &str) -> String {
    match kind {
        SessionHistoryEntryKind::ProviderTool => tool_title(text),
        SessionHistoryEntryKind::ProviderReasoning => "thinking".to_string(),
        SessionHistoryEntryKind::ProviderError => "error".to_string(),
        SessionHistoryEntryKind::ProviderStatus => "status".to_string(),
        SessionHistoryEntryKind::Notice => "note".to_string(),
        SessionHistoryEntryKind::ProviderOutput => "assistant".to_string(),
        SessionHistoryEntryKind::UserPrompt => "prompt".to_string(),
    }
}

fn blob_summary(kind: SessionHistoryEntryKind, text: &str) -> String {
    if kind == SessionHistoryEntryKind::ProviderTool {
        if let Some(summary) = tool_summary(text) {
            return summary;
        }
    }
    if kind == SessionHistoryEntryKind::ProviderStatus {
        return compact_blob_summary(text);
    }
    first_line(text)
}

fn compact_blob_summary(text: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 240;
    let mut summary = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if summary.chars().count() <= MAX_SUMMARY_CHARS {
        return summary;
    }
    summary = summary
        .chars()
        .take(MAX_SUMMARY_CHARS.saturating_sub(3))
        .collect();
    summary.push_str("...");
    summary
}

fn tool_title(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return "tool".to_string();
    };
    let tool = value
        .get("tool")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("tool");
    let status = value
        .get("status")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty());
    match status {
        Some(status) => format!("{tool} · {}", status.to_uppercase()),
        None => tool.to_string(),
    }
}

fn tool_summary(text: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let command = value
        .pointer("/input/command")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty());
    if let Some(command) = command {
        return Some(truncate_single_line(&format!("$ {command}")));
    }
    value
        .get("description")
        .or_else(|| value.get("title"))
        .or_else(|| value.get("output"))
        .and_then(|value| value.as_str())
        .map(first_line)
        .filter(|value| !value.is_empty())
}

fn first_line(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let line = normalized
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    truncate_single_line(line)
}

fn truncate_single_line(line: &str) -> String {
    const MAX_CHARS: usize = 120;
    let mut chars = line.chars();
    let truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
