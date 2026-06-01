//! Hierarchical transcript history outline loading.

use crate::error::DaemonError;
use crate::history::{
    HistoryEvent, HistoryEventKind, OperationalHistoryStore, SessionHistoryEntryKind,
};
use crate::local::{
    GetSessionHistoryBlobContentRequest, GetSessionHistoryOutlineRequest, LocalDaemonResponse,
    SessionHistoryOutlineAgent, SessionHistoryOutlineBlob, SessionHistoryOutlineCursor,
    SessionHistoryOutlineTurn,
};
use crate::session_history_page::SessionHistoryPageEntry;

const DEFAULT_LATEST_PROMPT_COUNT: usize = 4;
const MAX_LATEST_PROMPT_COUNT: usize = 20;
const BLOB_ID_PREFIX: &str = "history";

pub(crate) async fn execute_session_history_outline_request(
    operational_history: OperationalHistoryStore,
    request: GetSessionHistoryOutlineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let agent_ids = outline_agent_ids(&operational_history, &request)?;
        let latest_prompt_count = request
            .latest_prompt_count
            .unwrap_or(DEFAULT_LATEST_PROMPT_COUNT)
            .clamp(1, MAX_LATEST_PROMPT_COUNT);
        let mut agents = Vec::new();
        for agent_id in agent_ids {
            agents.push(load_agent_outline(
                &operational_history,
                &request.session_id,
                &agent_id,
                latest_prompt_count,
            )?);
        }
        Ok(LocalDaemonResponse::SessionHistoryOutline { agents })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history outline",
        message: error.to_string(),
    })?
}

pub(crate) async fn execute_session_history_blob_content_request(
    operational_history: OperationalHistoryStore,
    request: GetSessionHistoryBlobContentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let (sequence_start, sequence_end) = parse_blob_id(&request.blob_id)?;
        let events = operational_history.load_session_events_for_agent_sequence_range(
            &request.session_id,
            &request.agent_id,
            sequence_start,
            sequence_end,
        )?;
        let entries = events
            .into_iter()
            .filter_map(page_entry_from_event)
            .collect::<Vec<_>>();
        Ok(LocalDaemonResponse::SessionHistoryBlobContent {
            blob_id: request.blob_id,
            entries,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history blob content",
        message: error.to_string(),
    })?
}

fn outline_agent_ids(
    operational_history: &OperationalHistoryStore,
    request: &GetSessionHistoryOutlineRequest,
) -> Result<Vec<String>, DaemonError> {
    if let Some(agent_ids) = request.agent_ids.as_ref() {
        let mut normalized = agent_ids
            .iter()
            .filter_map(|agent_id| {
                let trimmed = agent_id.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.dedup();
        return Ok(normalized);
    }
    operational_history.list_session_history_agent_ids(&request.session_id)
}

fn load_agent_outline(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    latest_prompt_count: usize,
) -> Result<SessionHistoryOutlineAgent, DaemonError> {
    let prompts = operational_history.load_latest_user_prompt_events(
        session_id,
        agent_id,
        latest_prompt_count,
    )?;
    let mut turns = Vec::new();
    for (index, prompt) in prompts.iter().enumerate() {
        let sequence_end = prompts
            .get(index + 1)
            .map(|event| event.sequence.saturating_sub(1))
            .unwrap_or(i64::MAX as u64);
        let events = operational_history.load_session_events_for_agent_sequence_range(
            session_id,
            agent_id,
            prompt.sequence,
            sequence_end,
        )?;
        if let Some(turn) = outline_turn_from_events(prompt, events) {
            turns.push(turn);
        }
    }
    let next_cursor = prompts.first().map(|event| SessionHistoryOutlineCursor {
        before_sequence: event.sequence,
    });
    Ok(SessionHistoryOutlineAgent {
        agent_id: agent_id.to_string(),
        turns,
        next_cursor,
    })
}

fn outline_turn_from_events(
    prompt: &HistoryEvent,
    events: Vec<HistoryEvent>,
) -> Option<SessionHistoryOutlineTurn> {
    let user_prompt = page_entry_from_event(prompt.clone())?;
    let summary_sequence = events
        .iter()
        .rev()
        .find(|event| event.kind == HistoryEventKind::ProviderOutput && has_content(event))
        .map(|event| event.sequence);
    let summary = summary_sequence.and_then(|sequence| {
        events
            .iter()
            .find(|event| event.sequence == sequence)
            .cloned()
            .and_then(page_entry_from_event)
    });
    let blobs = events
        .into_iter()
        .filter(|event| event.sequence != prompt.sequence)
        .filter(|event| Some(event.sequence) != summary_sequence)
        .filter_map(outline_blob_from_event)
        .collect::<Vec<_>>();
    Some(SessionHistoryOutlineTurn {
        turn_id: prompt
            .turn_id
            .clone()
            .or_else(|| prompt.prompt_id.clone())
            .unwrap_or_else(|| format!("turn-{}", prompt.sequence)),
        prompt_id: prompt.prompt_id.clone(),
        started_at_ms: prompt.timestamp_ms,
        user_prompt,
        summary,
        blobs,
    })
}

fn outline_blob_from_event(event: HistoryEvent) -> Option<SessionHistoryOutlineBlob> {
    let entry = event.to_session_history_entry()?;
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

fn page_entry_from_event(event: HistoryEvent) -> Option<SessionHistoryPageEntry> {
    let entry = event.to_session_history_entry()?;
    let total_chars = entry.text.chars().count();
    Some(SessionHistoryPageEntry {
        entry_index: event.sequence as usize,
        fragment_start: 0,
        fragment_end: total_chars,
        total_chars,
        entry,
    })
}

fn has_content(event: &HistoryEvent) -> bool {
    event
        .content
        .as_deref()
        .is_some_and(|content| !content.trim().is_empty())
}

fn blob_id(sequence_start: u64, sequence_end: u64) -> String {
    format!("{BLOB_ID_PREFIX}:{sequence_start}:{sequence_end}")
}

fn parse_blob_id(blob_id: &str) -> Result<(u64, u64), DaemonError> {
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
    first_line(text)
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
