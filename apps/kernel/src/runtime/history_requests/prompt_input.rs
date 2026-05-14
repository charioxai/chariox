//! Prompt-input history recording and retrieval.

use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::history::{
    HistoryEvent, HistoryEventKind, HistoryEventQuery, HistoryEventRole, OperationalHistoryStore,
};
use crate::local::{
    GetPromptInputHistoryRequest, LocalDaemonResponse, PromptInputHistoryEntry,
    PromptInputHistoryEntryKind, RecordPromptInputHistoryRequest,
};

pub(crate) async fn execute_prompt_input_history_request(
    history: OperationalHistoryStore,
    request: GetPromptInputHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let session_id = request.session_id.clone();
    let limit = request.limit.unwrap_or(5000).clamp(1, 5000);
    let after_sequence = request.after_sequence;
    tokio::task::spawn_blocking(move || {
        let mut events = prompt_input_history_events_for_kind(
            &history,
            &session_id,
            "user_prompt",
            after_sequence,
            limit,
        )?;
        events.extend(prompt_input_history_events_for_kind(
            &history,
            &session_id,
            "prompt_input",
            after_sequence,
            limit,
        )?);
        events.sort_by_key(|event| event.sequence);
        events.truncate(limit);
        Ok(LocalDaemonResponse::PromptInputHistory {
            entries: events
                .into_iter()
                .filter_map(prompt_input_history_entry_from_event)
                .collect(),
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load prompt input history",
        message: error.to_string(),
    })?
}

pub(crate) async fn execute_record_prompt_input_history_request(
    history: OperationalHistoryStore,
    request: RecordPromptInputHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.text.trim().is_empty() {
        return Ok(LocalDaemonResponse::PromptInputHistoryRecorded {
            entry: PromptInputHistoryEntry {
                sequence: 0,
                timestamp_ms: 0,
                session_id: request.session_id,
                source_attachment_id: request.attachment_id,
                kind: request.kind,
                text: String::new(),
            },
        });
    }
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "input_kind".to_string(),
        serde_json::Value::String(
            match request.kind {
                PromptInputHistoryEntryKind::Prompt => "prompt",
                PromptInputHistoryEntryKind::Command => "command",
            }
            .to_string(),
        ),
    );
    if let Some(attachment_id) = request.attachment_id.clone() {
        metadata.insert(
            "source_attachment_id".to_string(),
            serde_json::Value::String(attachment_id),
        );
    }
    let event = history.append_operational_event(
        HistoryEventKind::PromptInput,
        Some(HistoryEventRole::User),
        Some(request.text),
        metadata,
        crate::history::HistoryEventTurnContext {
            session_id: Some(request.session_id),
            ..Default::default()
        },
    )?;
    let entry = prompt_input_history_entry_from_event(event).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "record prompt input history",
            message: "recorded event could not be converted".to_string(),
        }
    })?;
    Ok(LocalDaemonResponse::PromptInputHistoryRecorded { entry })
}

fn prompt_input_history_entry_from_event(event: HistoryEvent) -> Option<PromptInputHistoryEntry> {
    let session_id = event.session_id.clone()?;
    let kind = match event.kind {
        HistoryEventKind::UserPrompt => PromptInputHistoryEntryKind::Prompt,
        HistoryEventKind::PromptInput => match event
            .metadata
            .get("input_kind")
            .and_then(|value| value.as_str())
        {
            Some("command") => PromptInputHistoryEntryKind::Command,
            _ => PromptInputHistoryEntryKind::Prompt,
        },
        _ => return None,
    };
    Some(PromptInputHistoryEntry {
        sequence: event.sequence,
        timestamp_ms: event.timestamp_ms,
        session_id,
        source_attachment_id: event
            .metadata
            .get("source_attachment_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        kind,
        text: event.content.unwrap_or_default(),
    })
}

fn prompt_input_history_events_for_kind(
    history: &OperationalHistoryStore,
    session_id: &str,
    kind: &str,
    after_sequence: Option<u64>,
    limit: usize,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    let mut events = Vec::new();
    let mut cursor = after_sequence;
    while events.len() < limit {
        let batch = history.query_events(HistoryEventQuery {
            session_id: Some(session_id.to_string()),
            kind: Some(kind.to_string()),
            after_sequence: cursor,
            limit: Some((limit - events.len()).min(500)),
            ..HistoryEventQuery::default()
        })?;
        let Some(last_sequence) = batch.last().map(|event| event.sequence) else {
            break;
        };
        let batch_len = batch.len();
        events.extend(batch);
        cursor = Some(last_sequence);
        if batch_len < 500 {
            break;
        }
    }
    Ok(events)
}
