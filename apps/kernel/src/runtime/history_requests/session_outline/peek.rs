//! MP-08 / MP-10 / MP-11 peek: an agent's transcript as the pane shows it, one title per row
//! (the outline's prompt, inline entries and blobs), and one row's content by pages. It reads
//! the outline the pane reads; nothing is stored for it.
use serde_json::{Map, Value};

use super::projection::{blob_id, provider_tool_payload};
use super::*;
use crate::history::SessionHistoryEntryKind;
use crate::local::{
    AgentTranscriptEntryKind as Kind, AgentTranscriptEntryPage,
    AgentTranscriptEntryStatus as Status, AgentTranscriptPeek, AgentTranscriptTitle,
    PeekAgentTranscriptRequest, ReadAgentTranscriptEntryRequest, DEFAULT_AGENT_PEEK_ENTRIES,
    DEFAULT_AGENT_PEEK_PAGE_CHARS, MAX_AGENT_PEEK_ENTRIES, MAX_AGENT_PEEK_PAGE_CHARS,
};
use crate::session_history_page::SessionHistoryPageEntry;

mod title;

pub(super) const WAIT_TOOL: &str = "chariox.wait";

pub(crate) async fn execute_agent_transcript_peek(
    operational_history: OperationalHistoryStore,
    request: PeekAgentTranscriptRequest,
    agent_import: Option<ExternalProviderImportMetadata>,
    now_ms: u64,
) -> Result<AgentTranscriptPeek, DaemonError> {
    tokio::task::spawn_blocking(move || {
        peek(
            &operational_history,
            &request,
            agent_import.as_ref(),
            now_ms,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "peek agent transcript",
        message: error.to_string(),
    })?
}

pub(crate) async fn execute_agent_transcript_entry_read(
    operational_history: OperationalHistoryStore,
    request: ReadAgentTranscriptEntryRequest,
    agent_import: Option<ExternalProviderImportMetadata>,
) -> Result<AgentTranscriptEntryPage, DaemonError> {
    tokio::task::spawn_blocking(move || {
        read_entry(&operational_history, &request, agent_import.as_ref())
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "read agent transcript entry",
        message: error.to_string(),
    })?
}

fn peek_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "peek agent transcript",
        message,
    }
}

/// A row and the sequences it covers.
struct Row {
    sequence_start: u64,
    sequence_end: u64,
    title: AgentTranscriptTitle,
}

fn peek(
    operational_history: &OperationalHistoryStore,
    request: &PeekAgentTranscriptRequest,
    agent_import: Option<&ExternalProviderImportMetadata>,
    now_ms: u64,
) -> Result<AgentTranscriptPeek, DaemonError> {
    let limit = request
        .limit
        .unwrap_or(DEFAULT_AGENT_PEEK_ENTRIES)
        .clamp(1, MAX_AGENT_PEEK_ENTRIES);
    let before = request
        .before
        .as_deref()
        .map(|cursor| {
            cursor.parse::<u64>().map_err(|_| {
                peek_error(format!(
                    "invalid before cursor `{cursor}`: pass the `before` of an earlier peek"
                ))
            })
        })
        .transpose()?;
    // Turns hold any number of rows: read more of them until the page is full.
    let mut prompt_count = 2;
    loop {
        let outline = load_scoped_agent_outline(
            operational_history,
            &request.session_id,
            &request.agent_id,
            prompt_count,
            before,
            agent_import,
        )?;
        let older_turns = outline.next_cursor.as_ref().map(|c| c.before_sequence);
        let mut rows = Vec::new();
        for turn in &outline.turns {
            rows.extend(turn_rows(
                operational_history,
                &request.session_id,
                &request.agent_id,
                agent_import,
                turn,
                now_ms,
            )?);
        }
        if rows.len() < limit && older_turns.is_some() && prompt_count < MAX_LATEST_PROMPT_COUNT {
            prompt_count = (prompt_count * 2).min(MAX_LATEST_PROMPT_COUNT);
            continue;
        }
        rows.sort_by_key(|row| (row.sequence_start, row.sequence_end));
        let older_rows = rows.len() > limit;
        let rows = rows.split_off(rows.len().saturating_sub(limit));
        let before = match rows.first() {
            Some(oldest) if older_rows || older_turns.is_some() => {
                Some(oldest.sequence_start.to_string())
            }
            None => older_turns.map(|sequence| sequence.to_string()),
            _ => None,
        };
        return Ok(AgentTranscriptPeek {
            agent_id: request.agent_id.clone(),
            entries: rows.into_iter().map(|row| row.title).collect(),
            before,
        });
    }
}

fn turn_rows(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    agent_import: Option<&ExternalProviderImportMetadata>,
    turn: &SessionHistoryOutlineTurn,
    now_ms: u64,
) -> Result<Vec<Row>, DaemonError> {
    let open = turn.lifecycle == SessionHistoryOutlineTurnLifecycle::Open;
    let turn_status = match turn.lifecycle {
        SessionHistoryOutlineTurnLifecycle::Open => Status::Running,
        SessionHistoryOutlineTurnLifecycle::Completed => Status::Completed,
        SessionHistoryOutlineTurnLifecycle::Cancelled => Status::Cancelled,
        SessionHistoryOutlineTurnLifecycle::Failed => Status::Failed,
    };
    let mut rows = Vec::new();
    if turn.user_prompt.total_chars > 0 {
        let ended = turn.completed_at_ms.unwrap_or(now_ms);
        rows.push(entry_row(
            &turn.user_prompt,
            Kind::Prompt,
            title::text_title(&turn.user_prompt.entry.text),
            turn_status,
            turn.started_at_ms,
            Some(ended.saturating_sub(turn.started_at_ms)),
        ));
    }
    for blob in &turn.blobs {
        rows.push(blob_row(
            operational_history,
            session_id,
            agent_id,
            agent_import,
            blob,
            turn_status,
            now_ms,
        )?);
    }
    // A long message is both a blob and its truncated inline entry: the blob is the row.
    let covered = |entry: &SessionHistoryPageEntry| {
        let sequence = entry.entry_index as u64;
        turn.blobs
            .iter()
            .any(|blob| (blob.sequence_start..=blob.sequence_end).contains(&sequence))
    };
    for entry in turn
        .entries
        .iter()
        .chain(&turn.summary)
        .filter(|e| !covered(*e))
    {
        let kind = page_entry_kind(entry.entry.kind);
        rows.push(entry_row(
            entry,
            kind,
            title::text_title(&entry.entry.text),
            Status::Completed,
            entry.entry.timestamp_ms,
            None,
        ));
    }
    // The open turn's latest message or thought is the one being written.
    if open {
        if let Some(latest) = rows
            .iter_mut()
            .filter(|row| row.title.kind != Kind::Prompt)
            .max_by_key(|row| row.sequence_start)
            .filter(|row| matches!(row.title.kind, Kind::Assistant | Kind::Thinking))
        {
            latest.title.status = Status::Running;
            latest.title.duration_ms = Some(now_ms.saturating_sub(latest.title.at_ms));
            if latest.title.kind == Kind::Thinking {
                latest.title.title = latest.title.title.replacen("Thought", "Thinking", 1);
            }
        }
    }
    Ok(rows)
}

fn entry_row(
    entry: &SessionHistoryPageEntry,
    kind: Kind,
    title: String,
    status: Status,
    at_ms: u64,
    duration_ms: Option<u64>,
) -> Row {
    let sequence_start = entry.entry_index as u64;
    let sequence_end = entry.sequence_end.map_or(sequence_start, |end| end as u64);
    Row {
        sequence_start,
        sequence_end,
        title: AgentTranscriptTitle {
            entry_id: blob_id(sequence_start, sequence_end),
            kind,
            title,
            status,
            at_ms,
            duration_ms,
            chars: entry.total_chars,
        },
    }
}

fn page_entry_kind(kind: SessionHistoryEntryKind) -> Kind {
    match kind {
        SessionHistoryEntryKind::UserPrompt => Kind::Prompt,
        SessionHistoryEntryKind::ProviderOutput => Kind::Assistant,
        SessionHistoryEntryKind::ProviderReasoning => Kind::Thinking,
        SessionHistoryEntryKind::ProviderTool => Kind::ToolCall,
        SessionHistoryEntryKind::ProviderError => Kind::Error,
        SessionHistoryEntryKind::ProviderStatus | SessionHistoryEntryKind::Notice => Kind::Notice,
    }
}

fn blob_row(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    agent_import: Option<&ExternalProviderImportMetadata>,
    blob: &crate::local::SessionHistoryOutlineBlob,
    turn_status: Status,
    now_ms: u64,
) -> Result<Row, DaemonError> {
    let mut kind = page_entry_kind(blob.kind);
    let mut status = Status::Completed;
    let mut at_ms = blob.timestamp_ms;
    let mut duration_ms = None;
    let title = match kind {
        Kind::ToolCall => {
            let snapshot = blob
                .preview
                .as_deref()
                .and_then(|preview| serde_json::from_str::<Map<String, Value>>(preview).ok())
                .unwrap_or_default();
            let background = snapshot.get("background").and_then(Value::as_bool) == Some(true);
            if snapshot.get("tool").and_then(Value::as_str) == Some(WAIT_TOOL) {
                kind = Kind::Wait;
            }
            status = tool_status(&snapshot);
            // A call the turn left behind without its end ended with the turn.
            if status == Status::Running && !background && turn_status != Status::Running {
                status = turn_status;
            }
            let started = snapshot.get("started_at_ms").and_then(Value::as_u64);
            let finished = snapshot.get("finished_at_ms").and_then(Value::as_u64);
            at_ms = started.unwrap_or(blob.timestamp_ms);
            duration_ms = match (status, finished) {
                (Status::Running, _) => Some(now_ms.saturating_sub(at_ms)),
                (_, Some(finished)) => Some(finished.saturating_sub(at_ms)),
                _ => started.map(|started| blob.timestamp_ms.saturating_sub(started)),
            };
            title::tool_title(&snapshot, status == Status::Running)
        }
        Kind::Thinking => title::thought_title(blob.preview.as_deref().unwrap_or_default(), false),
        _ => {
            // A grouped message's summary counts its fragments: title it by its words.
            let entries = load_blob_content_entries(
                operational_history,
                session_id,
                agent_id,
                blob.sequence_start,
                blob.sequence_end,
                agent_import,
            )?;
            let text = entries
                .iter()
                .map(|entry| entry.entry.text.as_str())
                .take(8)
                .collect::<String>();
            title::text_title(&text)
        }
    };
    Ok(Row {
        sequence_start: blob.sequence_start,
        sequence_end: blob.sequence_end,
        title: AgentTranscriptTitle {
            entry_id: blob.blob_id.clone(),
            kind,
            title,
            status,
            at_ms,
            duration_ms,
            chars: blob.total_chars,
        },
    })
}

fn tool_status(snapshot: &Map<String, Value>) -> Status {
    if title::woken_by(snapshot) == Some("deadline_reached") {
        return Status::TimedOut;
    }
    let status = snapshot
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match status.as_str() {
        "running" | "pending" | "in_progress" | "inprogress" | "started" | "queued" => {
            Status::Running
        }
        "failed" | "error" | "errored" => Status::Failed,
        "cancelled" | "canceled" | "interrupted" | "aborted" | "declined" => Status::Cancelled,
        _ => Status::Completed,
    }
}

fn read_entry(
    operational_history: &OperationalHistoryStore,
    request: &ReadAgentTranscriptEntryRequest,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Result<AgentTranscriptEntryPage, DaemonError> {
    let (sequence_start, sequence_end) = parse_blob_id(&request.entry_id)?;
    let entries = load_blob_content_entries(
        operational_history,
        &request.session_id,
        &request.agent_id,
        sequence_start,
        sequence_end,
        agent_import,
    )?;
    let Some(first) = entries.first() else {
        return Err(peek_error(format!(
            "entry `{}` is not in this agent's transcript: pass an entry_id from its peek",
            request.entry_id
        )));
    };
    let mut kind = page_entry_kind(first.entry.kind);
    let (title, content) = if kind == Kind::ToolCall {
        let snapshot = merged_tool_snapshot(&entries);
        if snapshot.get("tool").and_then(Value::as_str) == Some(WAIT_TOOL) {
            kind = Kind::Wait;
        }
        let running = tool_status(&snapshot) == Status::Running;
        (
            title::tool_title(&snapshot, running),
            tool_content(&snapshot),
        )
    } else {
        let text = entries
            .iter()
            .map(|entry| entry.entry.text.as_str())
            .collect::<String>();
        let title = if kind == Kind::Thinking {
            title::thought_title(&text, false)
        } else {
            title::text_title(&text)
        };
        (title, text)
    };
    let limit = request
        .limit
        .unwrap_or(DEFAULT_AGENT_PEEK_PAGE_CHARS)
        .clamp(1, MAX_AGENT_PEEK_PAGE_CHARS);
    let total_chars = content.chars().count();
    let offset = request.offset.min(total_chars);
    let text = content.chars().skip(offset).take(limit).collect::<String>();
    let end = offset + text.chars().count();
    Ok(AgentTranscriptEntryPage {
        entry_id: request.entry_id.clone(),
        kind,
        title,
        offset,
        text,
        total_chars,
        next_offset: (end < total_chars).then_some(end),
    })
}

/// A call's snapshots, later fields winning; an imported result record keeps the call's tool.
fn merged_tool_snapshot(entries: &[SessionHistoryPageEntry]) -> Map<String, Value> {
    let mut snapshot = Map::new();
    for entry in entries {
        if let Some(Value::Object(mut fields)) = provider_tool_payload(&entry.entry.text) {
            if fields.get("tool").and_then(Value::as_str) == Some("function_call_output") {
                fields.remove("tool");
            }
            snapshot.extend(fields);
        }
    }
    snapshot
}

/// A call reads as its input, then its output as the tool printed it.
fn tool_content(snapshot: &Map<String, Value>) -> String {
    let mut content = String::new();
    if let Some(input) = snapshot.get("input").filter(|input| !input.is_null()) {
        content.push_str(&format!("input: {input}\n"));
    }
    for key in ["output", "aggregatedOutput", "result", "text", "error"] {
        match snapshot.get(key) {
            Some(Value::String(text)) if !text.is_empty() => content.push_str(text),
            Some(value) if !value.is_null() && !value.as_str().is_some_and(str::is_empty) => {
                content.push_str(&value.to_string())
            }
            _ => continue,
        }
        if !content.ends_with('\n') {
            content.push('\n');
        }
    }
    content
}

#[cfg(test)]
mod tests;
