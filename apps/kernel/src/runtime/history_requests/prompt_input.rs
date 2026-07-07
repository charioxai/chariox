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
        HistoryEventKind::UserPrompt => {
            if !user_prompt_event_counts_as_prompt_input_history(&event) {
                return None;
            }
            PromptInputHistoryEntryKind::Prompt
        }
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

fn user_prompt_event_counts_as_prompt_input_history(event: &HistoryEvent) -> bool {
    let Some(entry) = event.to_session_history_entry() else {
        return true;
    };
    match entry.prompt_origin {
        Some(crate::session::PromptOrigin::Arroba) => true,
        Some(crate::session::PromptOrigin::External) => false,
        None => !entry.is_external_provider_observed(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryEntryKind};

    #[test]
    fn prompt_input_history_excludes_external_observed_prompts() {
        let path = std::env::temp_dir().join(format!(
            "arroba-prompt-input-history-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store =
            OperationalHistoryStore::open(path.clone()).expect("operational history should open");
        let arroba_prompt =
            SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "arroba");
        let external_origin_prompt = SessionHistoryEntry::user_prompt(
            "session-1",
            "attachment-1",
            "agent-1",
            "external origin",
        )
        .with_prompt_origin(crate::session::PromptOrigin::External);
        let external_observed_prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            None,
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "external observed",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_000),
        );
        for (sequence, entry) in [
            (1, arroba_prompt),
            (2, external_origin_prompt),
            (3, external_observed_prompt),
        ] {
            store
                .append(&HistoryEvent::transcript(
                    sequence,
                    &entry,
                    HistoryEventTurnContext {
                        session_id: Some("session-1".to_string()),
                        agent_id: Some("agent-1".to_string()),
                        prompt_id: Some(format!("prompt-{sequence}")),
                        ..HistoryEventTurnContext::default()
                    },
                ))
                .expect("prompt event should append");
        }
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime
            .block_on(execute_record_prompt_input_history_request(
                store.clone(),
                RecordPromptInputHistoryRequest {
                    session_id: "session-1".to_string(),
                    attachment_id: Some("attachment-1".to_string()),
                    kind: PromptInputHistoryEntryKind::Prompt,
                    text: "draft input".to_string(),
                },
            ))
            .expect("draft input should record");

        let response = runtime
            .block_on(execute_prompt_input_history_request(
                store,
                GetPromptInputHistoryRequest {
                    session_id: "session-1".to_string(),
                    after_sequence: None,
                    limit: None,
                },
            ))
            .expect("prompt input history should load");
        let LocalDaemonResponse::PromptInputHistory { entries } = response else {
            panic!("unexpected response");
        };

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["arroba", "draft input"]
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
