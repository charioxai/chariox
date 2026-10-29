use std::collections::{BTreeSet, HashMap};

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
    if event.is_internal_dispatch_diagnostic() {
        return false;
    }
    match event.kind {
        HistoryEventKind::ProviderOutput => true,
        HistoryEventKind::ProviderStatus => event_provider_status_projects_as_outline_entry(event),
        HistoryEventKind::Notice => true,
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
    !matches!(
        event.kind,
        HistoryEventKind::ProviderOutput
            | HistoryEventKind::ProviderStatus
            | HistoryEventKind::Notice
    )
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
    if event.is_internal_dispatch_diagnostic() {
        return None;
    }
    let entry = event.to_session_history_entry()?;
    if entry.is_external_provider_observed_state_signal() {
        return None;
    }
    let total_chars = entry.text.chars().count();
    Some(SessionHistoryOutlineBlob {
        blob_id: blob_id(event.sequence, event.sequence),
        steering: is_steering_prompt_event(&event),
        kind: entry.kind,
        title: blob_title(entry.kind),
        summary: blob_summary(entry.kind, &entry.text),
        sequence_start: event.sequence,
        sequence_end: event.sequence,
        entry_count: 1,
        total_chars,
        timestamp_ms: event.timestamp_ms,
        preview: outline_blob_preview(&[&event]),
    })
}

pub(super) fn outline_blobs_from_events(
    events: &[HistoryEvent],
    prompt_sequence: u64,
    summary_sequences: &BTreeSet<u64>,
    forced_blob_sequences: &BTreeSet<u64>,
) -> Vec<SessionHistoryOutlineBlob> {
    let is_candidate = |event: &HistoryEvent| {
        event.sequence != prompt_sequence
            && (!summary_sequences.contains(&event.sequence)
                || event_needs_outline_blob(event)
                || forced_blob_sequences.contains(&event.sequence))
            && (forced_blob_sequences.contains(&event.sequence)
                || event_projects_as_outline_blob(event))
            && event
                .to_session_history_entry()
                .is_some_and(|entry| !entry.is_external_provider_observed_state_signal())
    };
    let mut blobs = Vec::new();
    for group in outline_blob_event_groups(events, is_candidate) {
        blobs.extend(
            group
                .chunks(MAX_OUTLINE_EVENTS_PER_BLOB)
                .filter_map(outline_blob_from_event_group),
        );
    }
    blobs
}

/// One blob per tool call and per thought (provider merge key); other kinds
/// group by kind. A blob never spans a transcript entry rendered inline, so
/// its content stays in output order between the entries around it. The one
/// exception is a later snapshot of a tool call already seen (OpenCode stores
/// "running" and "completed" as rows a sentence can fall between): it joins
/// the call's blob, which keeps the call's first position.
fn outline_blob_event_groups(
    events: &[HistoryEvent],
    is_candidate: impl Fn(&HistoryEvent) -> bool,
) -> Vec<Vec<&HistoryEvent>> {
    let mut groups = Vec::<Vec<&HistoryEvent>>::new();
    let mut tool_call_groups = HashMap::<String, usize>::new();
    let mut open_group = false;
    for event in events {
        if !is_candidate(event) {
            if event_projects_as_outline_entry(event) || is_steering_prompt_event(event) {
                open_group = false;
            }
            continue;
        }
        let identity = outline_blob_identity(event);
        if let Some(index) =
            outline_blob_tool_call_id(&identity).and_then(|call_id| tool_call_groups.get(call_id))
        {
            groups[*index].push(event);
            open_group = *index == groups.len() - 1;
            continue;
        }
        let continues = open_group
            && groups
                .last()
                .and_then(|group| group.first())
                .is_some_and(|first| outline_blob_identity(first) == identity);
        match groups.last_mut() {
            Some(group) if continues => group.push(event),
            _ => groups.push(vec![event]),
        }
        if let Some(call_id) = outline_blob_tool_call_id(&identity) {
            tool_call_groups.insert(call_id.to_string(), groups.len() - 1);
        }
        open_group = true;
    }
    groups
}

fn outline_blob_tool_call_id(
    identity: &(Option<SessionHistoryEntryKind>, Option<String>),
) -> Option<&str> {
    match identity {
        (Some(SessionHistoryEntryKind::ProviderTool), Some(call_id)) => Some(call_id),
        _ => None,
    }
}

/// A blob's events: a tool call's blob may span rows of other entries
/// (see `outline_blob_event_groups`), which belong to those entries.
pub(super) fn outline_blob_content_events(events: Vec<HistoryEvent>) -> Vec<HistoryEvent> {
    let call_id = events
        .first()
        .map(outline_blob_identity)
        .and_then(|identity| outline_blob_tool_call_id(&identity).map(str::to_string));
    match call_id {
        Some(call_id) => events
            .into_iter()
            .filter(|event| {
                outline_blob_tool_call_id(&outline_blob_identity(event)) == Some(call_id.as_str())
            })
            .collect(),
        None => events,
    }
}

fn outline_blob_identity(
    event: &HistoryEvent,
) -> (Option<SessionHistoryEntryKind>, Option<String>) {
    let entry = event.to_session_history_entry();
    let identity = entry.as_ref().and_then(|entry| match entry.kind {
        SessionHistoryEntryKind::ProviderTool => provider_tool_call_id(entry),
        SessionHistoryEntryKind::ProviderReasoning => entry.merge_key.clone(),
        SessionHistoryEntryKind::UserPrompt => entry.merge_key.clone(),
        _ => None,
    });
    (entry.map(|entry| entry.kind), identity)
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
        steering: is_steering_prompt_event(first_event),
        kind: first_entry.kind,
        title: grouped_blob_title(
            first_entry.kind,
            if first_entry.kind == SessionHistoryEntryKind::ProviderTool {
                unique_provider_tool_call_count(events)
            } else {
                events.len()
            },
        ),
        summary: match first_entry.kind {
            SessionHistoryEntryKind::ProviderTool => events
                .last()
                .and_then(|event| event.to_session_history_entry())
                .map(|entry| blob_summary(entry.kind, &entry.text))
                .unwrap_or_default(),
            SessionHistoryEntryKind::ProviderError => compact_blob_summary(&first_entry.text),
            _ => format!("{} entries, {} chars", events.len(), total_chars),
        },
        sequence_start,
        sequence_end,
        entry_count: events.len(),
        total_chars,
        timestamp_ms,
        preview: outline_blob_preview(events),
    })
}

/// A tool call's snapshot without its output (later snapshots win), or the
/// first line of a thought; enough for clients to title the row.
fn outline_blob_preview(events: &[&HistoryEvent]) -> Option<String> {
    let entries = events
        .iter()
        .filter_map(|event| event.to_session_history_entry())
        .collect::<Vec<_>>();
    match entries.first()?.kind {
        SessionHistoryEntryKind::ProviderTool => {
            let mut snapshot = serde_json::Map::new();
            for entry in &entries {
                if let Some(serde_json::Value::Object(fields)) = provider_tool_payload(&entry.text)
                {
                    snapshot.extend(fields);
                }
            }
            if snapshot.is_empty() {
                return None;
            }
            for key in [
                "output",
                "text",
                "error",
                "aggregatedOutput",
                "contentItems",
                "result",
                "agentsStates",
                "images",
            ] {
                snapshot.remove(key);
            }
            // Patch rows keep only the changed paths, not the diffs.
            if let Some(raw) = snapshot.remove("raw") {
                if let Some(paths) = changed_paths_preview(&raw) {
                    snapshot.insert("raw".to_string(), serde_json::Value::String(paths));
                }
            }
            let mut snapshot = serde_json::Value::Object(snapshot);
            truncate_preview_strings(&mut snapshot);
            serde_json::to_string(&snapshot).ok()
        }
        SessionHistoryEntryKind::ProviderReasoning => {
            let text = entries
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<String>();
            Some(first_line(&text)).filter(|line| !line.is_empty())
        }
        _ => None,
    }
}

fn changed_paths_preview(raw: &serde_json::Value) -> Option<String> {
    let changes = serde_json::from_str::<Vec<serde_json::Value>>(raw.as_str()?).ok()?;
    let paths = changes
        .iter()
        .filter_map(|change| {
            let path = change.get("path")?.clone();
            let mut preview = serde_json::Map::from_iter([("path".to_string(), path)]);
            if let Some(kind) = change.get("kind") {
                preview.insert("kind".to_string(), kind.clone());
            }
            Some(serde_json::Value::Object(preview))
        })
        .collect::<Vec<_>>();
    (!paths.is_empty()).then(|| serde_json::Value::Array(paths).to_string())
}

fn truncate_preview_strings(value: &mut serde_json::Value) {
    const MAX_PREVIEW_STRING_CHARS: usize = 512;
    match value {
        serde_json::Value::String(text) if text.chars().count() > MAX_PREVIEW_STRING_CHARS => {
            *text = format!(
                "{}…",
                text.chars()
                    .take(MAX_PREVIEW_STRING_CHARS)
                    .collect::<String>()
            );
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(truncate_preview_strings),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(truncate_preview_strings),
        _ => {}
    }
}

fn unique_provider_tool_call_count(events: &[&HistoryEvent]) -> usize {
    events
        .iter()
        .map(|event| {
            let entry = event.to_session_history_entry();
            let call_id = entry
                .as_ref()
                .and_then(provider_tool_call_id)
                .unwrap_or_else(|| format!("history-event:{}", event.sequence));
            (event.provider_run_id.clone(), call_id)
        })
        .collect::<BTreeSet<_>>()
        .len()
}

fn provider_tool_call_id(entry: &crate::history::SessionHistoryEntry) -> Option<String> {
    provider_tool_payload(&entry.text)
        .and_then(|value| {
            value
                .get("call_id")
                .or_else(|| value.get("id"))
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
        .or_else(|| truncated_provider_tool_payload_field(&entry.text, "call_id"))
        .or_else(|| truncated_provider_tool_payload_field(&entry.text, "id"))
        .or_else(|| {
            entry
                .merge_key
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
}

fn truncated_provider_tool_payload_field(text: &str, field: &str) -> Option<String> {
    let (_, payload) = text.split_once('\n')?;
    let prefix = format!("  \"{field}\":");
    payload
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix(&prefix)?.trim_start();
            serde_json::Deserializer::from_str(value)
                .into_iter::<String>()
                .next()?
                .ok()
        })
        .filter(|value| !value.trim().is_empty())
}

fn provider_tool_payload(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let (_, payload) = text.split_once('\n')?;
        serde_json::from_str(payload).ok()
    })
}

fn grouped_blob_title(kind: SessionHistoryEntryKind, entry_count: usize) -> String {
    match kind {
        SessionHistoryEntryKind::ProviderTool => format!(
            "{entry_count} {} called",
            if entry_count == 1 { "tool" } else { "tools" },
        ),
        SessionHistoryEntryKind::ProviderReasoning => "thinking".to_string(),
        SessionHistoryEntryKind::ProviderError => "error".to_string(),
        SessionHistoryEntryKind::ProviderStatus => "status".to_string(),
        SessionHistoryEntryKind::Notice => "note".to_string(),
        SessionHistoryEntryKind::ProviderOutput => "assistant output".to_string(),
        SessionHistoryEntryKind::UserPrompt => "prompt".to_string(),
    }
}

pub(super) fn page_entry_from_event(event: HistoryEvent) -> Option<SessionHistoryPageEntry> {
    page_entry_from_event_with_inline_limit(event, None)
}

pub(super) fn outline_page_entry_from_event(
    event: HistoryEvent,
) -> Option<SessionHistoryPageEntry> {
    page_entry_from_event_with_inline_limit(event, Some(MAX_OUTLINE_INLINE_CHARS))
}

pub(super) fn outline_page_entry_from_event_group(
    events: &[&HistoryEvent],
) -> Option<SessionHistoryPageEntry> {
    let first = events.first()?;
    let mut page_entry = outline_page_entry_from_event((*first).clone())?;
    let mut ordered_events = events.to_vec();
    ordered_events.sort_by_key(|event| (event.timestamp_ms, event.sequence));
    let text = ordered_events
        .iter()
        .filter_map(|event| event.to_session_history_entry())
        .map(|entry| entry.text)
        .collect::<String>();
    let total_chars = text.chars().count();
    // MP-08 / MP-10: complete inline groups need every original replay identity too.
    if total_chars <= MAX_OUTLINE_INLINE_CHARS {
        let rows = crate::session_history_page::coalesce_history_page_entries(
            ordered_events
                .iter()
                .filter_map(|event| page_entry_from_event((**event).clone()))
                .collect(),
        );
        if rows.len() == 1 && rows[0].sequence_end.is_some() {
            return rows.into_iter().next();
        }
    }
    let fragment_end = total_chars.min(MAX_OUTLINE_INLINE_CHARS);
    page_entry.entry.text = text.chars().take(fragment_end).collect();
    page_entry.fragment_start = 0;
    page_entry.fragment_end = fragment_end;
    page_entry.total_chars = total_chars;
    Some(page_entry)
}

fn page_entry_from_event_with_inline_limit(
    event: HistoryEvent,
    max_inline_chars: Option<usize>,
) -> Option<SessionHistoryPageEntry> {
    if event.is_internal_dispatch_diagnostic() {
        return None;
    }
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
        sequence_end: None,
        sequence_byte_lengths: Vec::new(),
        sequence_indices: Vec::new(),
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

fn blob_title(kind: SessionHistoryEntryKind) -> String {
    match kind {
        SessionHistoryEntryKind::ProviderTool => grouped_blob_title(kind, 1),
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
    if matches!(
        kind,
        SessionHistoryEntryKind::ProviderStatus | SessionHistoryEntryKind::ProviderError
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    use crate::terminal::TerminalOutputKind;

    // MP-08 / MP-10 / MP-11: a hydrated failure must still explain itself.
    #[test]
    fn mp08_mp11_grouped_error_blob_retains_exit_cause_and_recovery() {
        let cause = "Provider exited before the prompt was delivered: provider process exited with status 143. Retry the prompt to start a new provider run.";
        let event = |sequence, text: &str| {
            HistoryEvent::transcript(
                sequence,
                &SessionHistoryEntry::provider_output(
                    "session-1",
                    "provider-run-1",
                    Some("agent-1"),
                    TerminalOutputKind::ProviderError,
                    None,
                    text.to_string(),
                ),
                HistoryEventTurnContext::default(),
            )
        };
        let first = event(1, cause);
        let second = event(2, "Request not carried out; it was dropped.");
        let blob = outline_blob_from_event_group(&[&first, &second]).unwrap();
        assert!(blob.summary.contains("status 143"), "{}", blob.summary);
        assert!(
            blob.summary.contains("Retry the prompt"),
            "{}",
            blob.summary
        );
        assert_eq!(blob.entry_count, 2);
        assert!(blob.summary.chars().count() <= 240);
        let single = outline_blob_from_event(first).unwrap();
        assert!(single.summary.contains("Retry the prompt"));
    }

    #[test]
    fn grouped_blob_titles_are_user_facing() {
        assert_eq!(
            grouped_blob_title(SessionHistoryEntryKind::ProviderTool, 2),
            "2 tools called"
        );
        assert_eq!(
            grouped_blob_title(SessionHistoryEntryKind::ProviderTool, 1),
            "1 tool called"
        );
        assert_eq!(
            grouped_blob_title(SessionHistoryEntryKind::ProviderOutput, 4),
            "assistant output"
        );
    }

    #[test]
    fn provider_tool_blob_counts_running_and_completed_records_as_one_call() {
        let running = provider_tool_event(1, "call-1", "running");
        let completed = provider_tool_event(2, "call-1", "completed");

        let blob = outline_blob_from_event_group(&[&running, &completed]).expect("tool blob");

        assert_eq!(blob.entry_count, 2);
        assert_eq!(blob.title, "1 tool called");
    }

    #[test]
    fn provider_tool_blob_counts_distinct_calls_once_each() {
        let first_running = provider_tool_event(1, "call-1", "running");
        let first_completed = provider_tool_event(2, "call-1", "completed");
        let second_running = provider_tool_event(3, "call-2", "running");
        let second_completed = provider_tool_event(4, "call-2", "completed");

        let blob = outline_blob_from_event_group(&[
            &first_running,
            &first_completed,
            &second_running,
            &second_completed,
        ])
        .expect("tool blob");

        assert_eq!(blob.entry_count, 4);
        assert_eq!(blob.title, "2 tools called");
    }

    #[test]
    fn provider_tool_blob_counts_codex_call_and_output_as_one_call() {
        let call = provider_tool_event_with_text(
            1,
            "custom_tool_call-ctc-1",
            format!(
                "codex tool item\n{}",
                serde_json::json!({
                    "type": "custom_tool_call",
                    "id": "ctc-1",
                    "call_id": "call-1",
                })
            ),
        );
        let output = provider_tool_event_with_text(
            2,
            "custom_tool_call_output-call-1",
            format!(
                "codex tool item\n{}",
                serde_json::json!({
                    "type": "custom_tool_call_output",
                    "call_id": "call-1",
                })
            ),
        );

        let blob = outline_blob_from_event_group(&[&call, &output]).expect("tool blob");

        assert_eq!(blob.entry_count, 2);
        assert_eq!(blob.title, "1 tool called");
    }

    #[test]
    fn provider_tool_blob_counts_truncated_codex_output_with_its_call() {
        let call = provider_tool_event_with_text(
            1,
            "custom_tool_call-ctc-large",
            format!(
                "codex tool item\n{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "type": "custom_tool_call",
                    "id": "ctc-large",
                    "call_id": "call-large",
                }))
                .expect("tool call payload should serialize")
            ),
        );
        let full_output = serde_json::to_string_pretty(&serde_json::json!({
            "type": "custom_tool_call_output",
            "call_id": "call-large",
            "output": [
                { "type": "input_text", "text": "x".repeat(20_000) },
            ],
        }))
        .expect("tool output payload should serialize");
        let truncated_output = full_output.chars().take(16 * 1024).collect::<String>();
        assert!(serde_json::from_str::<serde_json::Value>(&truncated_output).is_err());
        let output = provider_tool_event_with_text(
            2,
            "custom_tool_call_output-call-large",
            format!("codex tool item\n{truncated_output}"),
        );

        let blob = outline_blob_from_event_group(&[&call, &output]).expect("tool blob");

        assert_eq!(blob.entry_count, 2);
        assert_eq!(blob.title, "1 tool called");
    }

    // MP-08 / MP-10 (panes R7): the Codex outline blob `history:760:787`
    // spanned tool 3, the assistant sentence after it and tool 4; clients then
    // merged both calls into one row and moved the sentence below it.
    #[test]
    fn tool_blobs_are_one_per_call_and_never_span_inline_text() {
        let events = vec![
            command_event(1, "call-3", "running", "cat README.md"),
            command_event(2, "call-3", "completed", "cat README.md"),
            transcript_event(3, TerminalOutputKind::ProviderOutput, "msg-1", "Read it."),
            command_event(4, "call-4", "running", "ls -la"),
            command_event(5, "call-4", "completed", "ls -la"),
        ];

        let blobs = outline_blobs_from_events(&events, 0, &BTreeSet::new(), &BTreeSet::new());

        assert_eq!(
            blobs
                .iter()
                .map(|blob| (blob.sequence_start, blob.sequence_end))
                .collect::<Vec<_>>(),
            vec![(1, 2), (4, 5)]
        );
        assert_eq!(blobs[0].summary, "$ cat README.md");
        let preview: serde_json::Value =
            serde_json::from_str(blobs[1].preview.as_deref().expect("tool preview")).unwrap();
        assert_eq!(
            preview,
            serde_json::json!({
                "id": "call-4",
                "tool": "bash",
                "status": "completed",
                "input": { "command": "ls -la" },
            })
        );
    }

    #[test]
    fn a_tool_blob_preview_leaves_out_result_images() {
        let events = vec![provider_tool_event_with_text(
            1,
            "call-img",
            serde_json::json!({
                "id": "call-img",
                "tool": "Read",
                "status": "completed",
                "input": { "file_path": "chart.png" },
                "images": ["data:image/png;base64,iVBORw0KGgo="],
            })
            .to_string(),
        )];

        let blobs = outline_blobs_from_events(&events, 0, &BTreeSet::new(), &BTreeSet::new());

        let preview = blobs[0].preview.as_deref().expect("tool preview");
        assert!(!preview.contains("images") && !preview.contains("base64"));
    }

    #[test]
    fn reasoning_blobs_are_one_per_thought_with_its_first_line() {
        let events = vec![
            transcript_event(
                1,
                TerminalOutputKind::ProviderReasoning,
                "r-1",
                "**Planning**\n",
            ),
            transcript_event(2, TerminalOutputKind::ProviderReasoning, "r-1", "more"),
            transcript_event(
                3,
                TerminalOutputKind::ProviderReasoning,
                "r-2",
                "Checking output",
            ),
        ];

        let blobs = outline_blobs_from_events(&events, 0, &BTreeSet::new(), &BTreeSet::new());

        assert_eq!(
            blobs
                .iter()
                .map(|blob| (
                    blob.sequence_start,
                    blob.sequence_end,
                    blob.preview.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                (1, 2, Some("**Planning**")),
                (3, 3, Some("Checking output"))
            ]
        );
    }

    fn command_event(sequence: u64, call_id: &str, status: &str, command: &str) -> HistoryEvent {
        provider_tool_event_with_text(
            sequence,
            call_id,
            serde_json::json!({
                "id": call_id,
                "tool": "bash",
                "status": status,
                "input": { "command": command },
                "output": "x".repeat(2_000),
            })
            .to_string(),
        )
    }

    fn transcript_event(
        sequence: u64,
        kind: TerminalOutputKind,
        merge_key: &str,
        text: &str,
    ) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "provider-run-1",
                Some("agent-1"),
                kind,
                Some(merge_key.to_string()),
                text.to_string(),
            ),
            HistoryEventTurnContext {
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                prompt_id: Some("prompt-1".to_string()),
                provider_run_id: Some("provider-run-1".to_string()),
                ..HistoryEventTurnContext::default()
            },
        )
    }

    fn provider_tool_event(sequence: u64, call_id: &str, status: &str) -> HistoryEvent {
        provider_tool_event_with_text(
            sequence,
            call_id,
            serde_json::json!({
                "id": call_id,
                "tool": "bash",
                "status": status,
            })
            .to_string(),
        )
    }

    fn provider_tool_event_with_text(sequence: u64, merge_key: &str, text: String) -> HistoryEvent {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "provider-run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderTool,
                Some(merge_key.to_string()),
                text,
            ),
            HistoryEventTurnContext {
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                prompt_id: Some("prompt-1".to_string()),
                provider_run_id: Some("provider-run-1".to_string()),
                ..HistoryEventTurnContext::default()
            },
        )
    }
}
