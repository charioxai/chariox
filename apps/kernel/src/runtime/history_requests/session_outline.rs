//! Hierarchical transcript history outline loading.

use std::collections::BTreeSet;

use crate::error::DaemonError;
use crate::history::{
    HistoryEvent, HistoryEventKind, HistoryEventTurnContext, OperationalHistoryStore,
    SessionHistoryEntry, SessionHistoryEntryKind, STEERING_PROMPT_MERGE_KEY_PREFIX,
};
use crate::local::{
    GetSessionHistoryBlobContentRequest, GetSessionHistoryOutlineRequest, LocalDaemonResponse,
    SessionHistoryOutlineAgent, SessionHistoryOutlineBlob, SessionHistoryOutlineCursor,
    SessionHistoryOutlineTurn,
};
use crate::session::PromptOrigin;
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
                request.cursor.as_ref().map(|cursor| cursor.before_sequence),
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
    before_sequence: Option<u64>,
) -> Result<SessionHistoryOutlineAgent, DaemonError> {
    let mut prompts = operational_history.load_latest_user_prompt_events(
        session_id,
        agent_id,
        before_sequence,
        latest_prompt_count.saturating_add(1),
    )?;
    let has_more = prompts.len() > latest_prompt_count;
    if has_more {
        prompts.remove(0);
    }
    if prompts.is_empty() {
        if before_sequence.is_some() {
            return Ok(SessionHistoryOutlineAgent {
                agent_id: agent_id.to_string(),
                turns: Vec::new(),
                next_cursor: None,
            });
        }
        return load_promptless_agent_outline(operational_history, session_id, agent_id);
    }
    let mut turns = Vec::new();
    let mut seen_turn_ids = BTreeSet::new();
    for (index, prompt) in prompts.iter().enumerate() {
        let has_newer_prompt = prompts.get(index + 1).is_some() || before_sequence.is_some();
        let sequence_end = prompts
            .get(index + 1)
            .map(|event| event.sequence.saturating_sub(1))
            .or_else(|| before_sequence.map(|sequence| sequence.saturating_sub(1)))
            .unwrap_or(i64::MAX as u64);
        let events = operational_history.load_session_events_for_agent_sequence_range(
            session_id,
            agent_id,
            prompt.sequence,
            sequence_end,
        )?;
        if let Some(mut turn) = outline_turn_from_events(prompt, events, has_newer_prompt) {
            ensure_unique_outline_turn_id(&mut turn, &mut seen_turn_ids);
            turns.push(turn);
        }
    }
    let next_cursor =
        has_more
            .then(|| prompts.first())
            .flatten()
            .map(|event| SessionHistoryOutlineCursor {
                before_sequence: event.sequence,
            });
    Ok(SessionHistoryOutlineAgent {
        agent_id: agent_id.to_string(),
        turns,
        next_cursor,
    })
}

fn load_promptless_agent_outline(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
) -> Result<SessionHistoryOutlineAgent, DaemonError> {
    let events = operational_history.load_session_events(session_id, Some(agent_id))?;
    let Some(latest_event) = events.last() else {
        return Ok(SessionHistoryOutlineAgent {
            agent_id: agent_id.to_string(),
            turns: Vec::new(),
            next_cursor: None,
        });
    };
    let latest_key = promptless_turn_group_key(latest_event);
    let turn_events = events
        .into_iter()
        .filter(|event| promptless_turn_group_key(event) == latest_key)
        .collect::<Vec<_>>();
    let Some(first_event) = turn_events.first() else {
        return Ok(SessionHistoryOutlineAgent {
            agent_id: agent_id.to_string(),
            turns: Vec::new(),
            next_cursor: None,
        });
    };
    let synthetic_prompt_entry =
        promptless_synthetic_prompt_entry(session_id, agent_id, &turn_events);
    let synthetic_prompt = HistoryEvent::transcript(
        first_event.sequence.saturating_sub(1),
        &synthetic_prompt_entry,
        HistoryEventTurnContext {
            session_id: Some(session_id.to_string()),
            agent_id: Some(agent_id.to_string()),
            turn_id: Some(latest_key),
            provider_run_id: first_event.provider_run_id.clone(),
            provider_session_id: first_event.provider_session_id.clone(),
            provider: first_event.provider.clone(),
            model: first_event.model.clone(),
            workflow_id: first_event.workflow_id.clone(),
            workflow_run_id: first_event.workflow_run_id.clone(),
            workflow_node_id: first_event.workflow_node_id.clone(),
            worktree_path: first_event.worktree_path.clone(),
            ..HistoryEventTurnContext::default()
        },
    );
    let mut events_with_prompt = Vec::with_capacity(turn_events.len() + 1);
    events_with_prompt.push(synthetic_prompt.clone());
    events_with_prompt.extend(turn_events);
    let turns = outline_turn_from_events(&synthetic_prompt, events_with_prompt, false)
        .into_iter()
        .collect::<Vec<_>>();
    Ok(SessionHistoryOutlineAgent {
        agent_id: agent_id.to_string(),
        turns,
        next_cursor: None,
    })
}

fn promptless_synthetic_prompt_entry(
    session_id: &str,
    agent_id: &str,
    events: &[HistoryEvent],
) -> SessionHistoryEntry {
    const PROMPTLESS_TEXT: &str = "(no recorded prompt; showing recent agent activity)";
    if let Some(identity) = outline_turn_external_identity(events) {
        return SessionHistoryEntry::external_provider_observed(
            session_id,
            events
                .iter()
                .find_map(|event| event.provider_run_id.as_deref()),
            agent_id,
            SessionHistoryEntryKind::UserPrompt,
            PROMPTLESS_TEXT,
            &identity.provider,
            &identity.provider_session_id,
            Some(identity.provider_turn_id),
            events
                .iter()
                .filter_map(|event| event.to_session_history_entry())
                .filter_map(|entry| entry.observed_at_ms)
                .min(),
        );
    }
    SessionHistoryEntry::user_prompt(session_id, "arroba-history", agent_id, PROMPTLESS_TEXT)
}

fn ensure_unique_outline_turn_id(
    turn: &mut SessionHistoryOutlineTurn,
    seen_turn_ids: &mut BTreeSet<String>,
) {
    if seen_turn_ids.insert(turn.turn_id.clone()) {
        return;
    }
    let base = turn.turn_id.clone();
    let sequence = turn.user_prompt.entry_index;
    let candidate = format!("{base}:seq-{sequence}");
    if seen_turn_ids.insert(candidate.clone()) {
        turn.turn_id = candidate;
        return;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}:seq-{sequence}-{suffix}");
        if seen_turn_ids.insert(candidate.clone()) {
            turn.turn_id = candidate;
            return;
        }
        suffix += 1;
    }
}

fn promptless_turn_group_key(event: &HistoryEvent) -> String {
    event
        .turn_id
        .clone()
        .or_else(|| event.prompt_id.clone())
        .or_else(|| event.provider_run_id.clone())
        .unwrap_or_else(|| event.event_id.clone())
}

fn outline_turn_from_events(
    prompt: &HistoryEvent,
    events: Vec<HistoryEvent>,
    has_newer_prompt: bool,
) -> Option<SessionHistoryOutlineTurn> {
    let user_prompt = page_entry_from_event(prompt.clone())?;
    let external_identity = outline_turn_external_identity(&events);
    let prompt_origin = outline_turn_prompt_origin(prompt);
    let completed_at_ms =
        outline_turn_completed_at_ms(prompt, &events, prompt_origin, has_newer_prompt);
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
    let entries = events
        .iter()
        .filter(|event| event.sequence != prompt.sequence)
        .filter(|event| Some(event.sequence) != summary_sequence)
        .filter(|event| has_content(event))
        .filter(|event| event_projects_as_outline_entry(event))
        .cloned()
        .filter_map(page_entry_from_event)
        .collect::<Vec<_>>();
    let blobs = events
        .into_iter()
        .filter(|event| event.sequence != prompt.sequence)
        .filter(|event| Some(event.sequence) != summary_sequence)
        .filter(|event| event_projects_as_outline_blob(event))
        .filter_map(outline_blob_from_event)
        .collect::<Vec<_>>();
    Some(SessionHistoryOutlineTurn {
        turn_id: prompt
            .turn_id
            .clone()
            .or_else(|| prompt.prompt_id.clone())
            .unwrap_or_else(|| format!("turn-{}", prompt.sequence)),
        prompt_id: prompt.prompt_id.clone(),
        prompt_origin,
        external_provider: external_identity
            .as_ref()
            .map(|identity| identity.provider.clone()),
        external_provider_session_id: external_identity
            .as_ref()
            .map(|identity| identity.provider_session_id.clone()),
        external_provider_turn_id: external_identity.map(|identity| identity.provider_turn_id),
        started_at_ms: prompt.timestamp_ms,
        completed_at_ms,
        user_prompt,
        entries,
        summary,
        blobs,
    })
}

fn outline_turn_completed_at_ms(
    prompt: &HistoryEvent,
    events: &[HistoryEvent],
    prompt_origin: PromptOrigin,
    has_newer_prompt: bool,
) -> Option<u64> {
    if let Some(settled_at_ms) = outline_turn_settlement_observed_at_ms(events) {
        return Some(settled_at_ms);
    }
    if prompt_origin == PromptOrigin::External && !has_newer_prompt {
        return None;
    }
    Some(
        events
            .iter()
            .filter(|event| has_content(event))
            .map(|event| event.timestamp_ms)
            .max()
            .unwrap_or(prompt.timestamp_ms),
    )
}

fn outline_turn_settlement_observed_at_ms(events: &[HistoryEvent]) -> Option<u64> {
    events
        .iter()
        .filter_map(|event| {
            let entry = event.to_session_history_entry()?;
            let observation = entry.external_observation.as_ref()?;
            observation
                .settles_active_prompt
                .then_some(entry.observed_at_ms.unwrap_or(event.timestamp_ms))
        })
        .max()
}

fn outline_turn_prompt_origin(prompt: &HistoryEvent) -> PromptOrigin {
    if prompt
        .to_session_history_entry()
        .is_some_and(|entry| entry.is_external_provider_observed())
    {
        return PromptOrigin::External;
    }
    PromptOrigin::Arroba
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutlineExternalIdentity {
    provider: String,
    provider_session_id: String,
    provider_turn_id: String,
}

fn outline_turn_external_identity(events: &[HistoryEvent]) -> Option<OutlineExternalIdentity> {
    events
        .iter()
        .filter_map(|event| event.to_session_history_entry())
        .find_map(|entry| {
            if !entry.is_external_provider_observed() {
                return None;
            }
            Some(OutlineExternalIdentity {
                provider: entry.external_provider?.trim().to_string(),
                provider_session_id: entry.external_provider_session_id?.trim().to_string(),
                provider_turn_id: entry.external_provider_turn_id?.trim().to_string(),
            })
            .filter(|identity| {
                !identity.provider.is_empty()
                    && !identity.provider_session_id.is_empty()
                    && !identity.provider_turn_id.is_empty()
            })
        })
}

fn event_projects_as_outline_entry(event: &HistoryEvent) -> bool {
    match event.kind {
        HistoryEventKind::ProviderOutput => true,
        HistoryEventKind::ProviderStatus => event
            .to_session_history_entry()
            .is_some_and(|entry| entry.is_external_provider_observed()),
        HistoryEventKind::UserPrompt => is_steering_prompt_event(event),
        _ => false,
    }
}

fn event_projects_as_outline_blob(event: &HistoryEvent) -> bool {
    if is_steering_prompt_event(event) {
        return false;
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
    let mut entry = event.to_session_history_entry()?;
    for attachment in &mut entry.attachments {
        attachment.rehydrate_preview_url();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{
        HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryPromptAttachment,
    };
    use crate::terminal::TerminalOutputKind;

    #[test]
    fn outline_turn_uses_transcript_admission_for_provider_status() {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let prompt = HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hello"),
            context.clone(),
        );
        let assistant = HistoryEvent::transcript(
            11,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                None,
                "assistant body before tool",
            ),
            context.clone(),
        );
        let tool = HistoryEvent::transcript(
            12,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderTool,
                Some("tool-1".to_string()),
                r#"{"tool":"bash","status":"completed","input":{"command":"echo ok"},"output":"detail"}"#,
            ),
            context.clone(),
        );
        let status = HistoryEvent::transcript(
            13,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderStatus,
                Some("status-1".to_string()),
                "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}",
            ),
            context.clone(),
        );
        let mut external_status_entry = SessionHistoryEntry::external_provider_observed(
            "session-1",
            None,
            "agent-1",
            SessionHistoryEntryKind::ProviderStatus,
            "codex task_complete",
            "codex",
            "thread-1",
            Some("done-1".to_string()),
            Some(15),
        );
        external_status_entry.external_observation =
            Some(crate::history::SessionHistoryExternalObservation {
                settles_active_prompt: true,
                passive_telemetry: false,
            });
        let external_status = HistoryEvent::transcript(15, &external_status_entry, context.clone());
        let summary = HistoryEvent::transcript(
            14,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                None,
                "final assistant body",
            ),
            context,
        );

        let turn = outline_turn_from_events(
            &prompt,
            vec![
                prompt.clone(),
                assistant,
                tool,
                status,
                external_status,
                summary,
            ],
            false,
        )
        .expect("turn should be outlined");

        assert_eq!(turn.prompt_origin, PromptOrigin::Arroba);
        assert_eq!(turn.external_provider.as_deref(), Some("codex"));
        assert_eq!(
            turn.external_provider_session_id.as_deref(),
            Some("thread-1")
        );
        assert_eq!(turn.external_provider_turn_id.as_deref(), Some("done-1"));
        assert_eq!(turn.completed_at_ms, Some(15));
        assert_eq!(turn.entries.len(), 2);
        assert_eq!(
            turn.entries[0].entry.kind,
            SessionHistoryEntryKind::ProviderOutput
        );
        assert_eq!(turn.entries[0].entry.text, "assistant body before tool");
        assert_eq!(
            turn.entries[1].entry.kind,
            SessionHistoryEntryKind::ProviderStatus
        );
        assert!(turn.entries[1].entry.is_external_provider_observed());
        assert_eq!(
            turn.entries[1]
                .entry
                .external_provider_session_id
                .as_deref(),
            Some("thread-1")
        );
        assert_eq!(
            turn.entries[1]
                .entry
                .external_observation
                .as_ref()
                .map(|observation| observation.settles_active_prompt),
            Some(true)
        );
        assert_eq!(turn.blobs.len(), 1);
        assert_eq!(turn.blobs[0].kind, SessionHistoryEntryKind::ProviderTool);
        assert_eq!(
            turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
            Some("final assistant body")
        );
    }

    #[test]
    fn outline_external_turn_without_settlement_stays_incomplete() {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let external_prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "external prompt",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_000),
        );
        let external_assistant = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            "partial output",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_100),
        );
        let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
        let assistant = HistoryEvent::transcript(11, &external_assistant, context);

        let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), assistant], false)
            .expect("external active turn should be outlined");

        assert_eq!(turn.prompt_origin, PromptOrigin::External);
        assert_eq!(turn.completed_at_ms, None);
        assert_eq!(
            turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
            Some("partial output")
        );
    }

    #[test]
    fn outline_external_turn_without_settlement_completes_when_newer_prompt_exists() {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let external_prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "external prompt",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_000),
        );
        let external_assistant = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            "final output",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_100),
        );
        let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
        let assistant = HistoryEvent::transcript(11, &external_assistant, context);

        let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), assistant], true)
            .expect("external bounded turn should be outlined");

        assert_eq!(turn.prompt_origin, PromptOrigin::External);
        assert_eq!(turn.completed_at_ms, Some(2_100));
    }

    #[test]
    fn agent_outline_completes_bounded_external_turns_without_client_repair() {
        let path = std::env::temp_dir().join(format!(
            "arroba-external-bounded-outline-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        for index in 1..=2 {
            let context = HistoryEventTurnContext {
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                turn_id: Some(format!("turn-{index}")),
                prompt_id: Some(format!("prompt-{index}")),
                provider_run_id: Some(format!("run-{index}")),
                ..HistoryEventTurnContext::default()
            };
            let prompt = SessionHistoryEntry::external_provider_observed(
                "session-1",
                Some(&format!("run-{index}")),
                "agent-1",
                SessionHistoryEntryKind::UserPrompt,
                &format!("external prompt {index}"),
                "codex",
                "thread-1",
                Some(format!("turn-{index}")),
                Some(index * 1_000),
            );
            let assistant = SessionHistoryEntry::external_provider_observed(
                "session-1",
                Some(&format!("run-{index}")),
                "agent-1",
                SessionHistoryEntryKind::ProviderOutput,
                &format!("external output {index}"),
                "codex",
                "thread-1",
                Some(format!("turn-{index}")),
                Some(index * 1_000 + 100),
            );
            store
                .append(&HistoryEvent::transcript(
                    index * 10,
                    &prompt,
                    context.clone(),
                ))
                .expect("external prompt should append");
            store
                .append(&HistoryEvent::transcript(
                    index * 10 + 1,
                    &assistant,
                    context,
                ))
                .expect("external assistant output should append");
        }

        let outline = load_agent_outline(&store, "session-1", "agent-1", 2, None)
            .expect("outline should load");

        assert_eq!(outline.turns.len(), 2);
        assert_eq!(outline.turns[0].prompt_origin, PromptOrigin::External);
        assert_eq!(outline.turns[0].completed_at_ms, Some(1_100));
        assert_eq!(outline.turns[1].prompt_origin, PromptOrigin::External);
        assert_eq!(outline.turns[1].completed_at_ms, None);

        let older = load_agent_outline(&store, "session-1", "agent-1", 1, Some(20))
            .expect("older outline page should load");
        assert_eq!(older.turns.len(), 1);
        assert_eq!(older.turns[0].completed_at_ms, Some(1_100));

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn outline_external_turn_uses_settlement_observed_time_as_completion() {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let external_prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "external prompt",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_000),
        );
        let mut external_status = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderStatus,
            "codex task_complete",
            "codex",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_200),
        );
        external_status.external_observation =
            Some(crate::history::SessionHistoryExternalObservation {
                settles_active_prompt: true,
                passive_telemetry: false,
            });
        let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
        let status = HistoryEvent::transcript(11, &external_status, context);

        let turn = outline_turn_from_events(&prompt, vec![prompt.clone(), status], false)
            .expect("external completed turn should be outlined");

        assert_eq!(turn.prompt_origin, PromptOrigin::External);
        assert_eq!(turn.completed_at_ms, Some(2_200));
    }

    #[test]
    fn outline_external_turn_uses_hidden_state_settlement_without_rendering_it() {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let external_prompt = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::UserPrompt,
            "external prompt",
            "claude",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_000),
        );
        let external_assistant = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            "final output",
            "claude",
            "thread-1",
            Some("turn-1".to_string()),
            Some(2_100),
        );
        let hidden_settlement = SessionHistoryEntry::external_provider_observed_state_signal(
            "session-1",
            Some("run-1"),
            "agent-1",
            "claude",
            "thread-1",
            crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
            "external:claude:thread-1:assistant-1",
            "turn-1".to_string(),
            Some(2_200),
        );
        let prompt = HistoryEvent::transcript(10, &external_prompt, context.clone());
        let assistant = HistoryEvent::transcript(11, &external_assistant, context.clone());
        let settlement = HistoryEvent::transcript(12, &hidden_settlement, context);

        let turn =
            outline_turn_from_events(&prompt, vec![prompt.clone(), assistant, settlement], false)
                .expect("external completed turn should be outlined");

        assert_eq!(turn.prompt_origin, PromptOrigin::External);
        assert_eq!(turn.completed_at_ms, Some(2_200));
        assert!(
            turn.entries.is_empty(),
            "hidden state rows should not render"
        );
        assert!(turn.blobs.is_empty(), "hidden state rows should not render");
        assert_eq!(
            turn.summary.as_ref().map(|entry| entry.entry.text.as_str()),
            Some("final output")
        );
    }

    #[test]
    fn agent_outline_makes_legacy_duplicate_turn_ids_unique() {
        let path = std::env::temp_dir().join(format!(
            "arroba-duplicate-turn-outline-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("prompt-2".to_string()),
            prompt_id: Some("prompt-2".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let first_prompt = HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "attachment-1",
                "agent-1",
                "first prompt",
            ),
            context.clone(),
        );
        let second_prompt = HistoryEvent::transcript(
            20,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "attachment-2",
                "agent-1",
                "second prompt",
            ),
            context,
        );
        store
            .append(&first_prompt)
            .expect("first prompt should append");
        store
            .append(&second_prompt)
            .expect("second prompt should append");

        let outline = load_agent_outline(&store, "session-1", "agent-1", 2, None)
            .expect("outline should load");

        assert_eq!(outline.turns.len(), 2);
        assert_eq!(outline.turns[0].turn_id, "prompt-2");
        assert_eq!(outline.turns[0].prompt_id.as_deref(), Some("prompt-2"));
        assert_eq!(outline.turns[0].user_prompt.entry.text, "first prompt");
        assert_eq!(outline.turns[1].turn_id, "prompt-2:seq-20");
        assert_eq!(outline.turns[1].prompt_id.as_deref(), Some("prompt-2"));
        assert_eq!(outline.turns[1].user_prompt.entry.text, "second prompt");

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn agent_outline_keeps_steering_prompts_inside_turns() {
        let path = std::env::temp_dir().join(format!(
            "arroba-steering-outline-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        let first_context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let second_context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-2".to_string()),
            prompt_id: Some("prompt-2".to_string()),
            provider_run_id: Some("run-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        store
            .append(&HistoryEvent::transcript(
                10,
                &SessionHistoryEntry::user_prompt(
                    "session-1",
                    "attachment-1",
                    "agent-1",
                    "first prompt",
                ),
                first_context.clone(),
            ))
            .expect("first prompt should append");
        store
            .append(&HistoryEvent::operational(
                20,
                HistoryEventKind::UserPrompt,
                Some(crate::history::HistoryEventRole::User),
                Some("steer this turn".to_string()),
                std::collections::BTreeMap::from([
                    (
                        "merge_key".to_string(),
                        serde_json::Value::String(crate::history::steering_prompt_merge_key(
                            "queued-1",
                        )),
                    ),
                    (
                        "source_attachment_id".to_string(),
                        serde_json::Value::String("attachment-1".to_string()),
                    ),
                ]),
                first_context,
            ))
            .expect("steering prompt should append");
        store
            .append(&HistoryEvent::transcript(
                30,
                &SessionHistoryEntry::user_prompt(
                    "session-1",
                    "attachment-1",
                    "agent-1",
                    "second prompt",
                ),
                second_context,
            ))
            .expect("second prompt should append");

        let outline = load_agent_outline(&store, "session-1", "agent-1", 2, None)
            .expect("outline should load");

        assert_eq!(outline.turns.len(), 2);
        assert_eq!(outline.turns[0].user_prompt.entry.text, "first prompt");
        assert_eq!(outline.turns[0].entries.len(), 1);
        assert_eq!(outline.turns[0].entries[0].entry.text, "steer this turn");
        assert_eq!(
            outline.turns[0].entries[0].entry.merge_key.as_deref(),
            Some("steering-prompt:queued-1")
        );
        assert_eq!(outline.turns[1].user_prompt.entry.text, "second prompt");
        assert_eq!(outline.turns[1].entries.len(), 0);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn agent_outline_pages_older_turns_with_cursor() {
        let path = std::env::temp_dir().join(format!(
            "arroba-cursor-outline-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        for index in 1..=5 {
            let sequence = index * 10;
            let context = HistoryEventTurnContext {
                session_id: Some("session-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                turn_id: Some(format!("turn-{index}")),
                prompt_id: Some(format!("prompt-{index}")),
                ..HistoryEventTurnContext::default()
            };
            let prompt = HistoryEvent::transcript(
                sequence,
                &SessionHistoryEntry::user_prompt(
                    "session-1",
                    &format!("attachment-{index}"),
                    "agent-1",
                    &format!("prompt {index}"),
                ),
                context,
            );
            store.append(&prompt).expect("prompt should append");
        }

        let newest = load_agent_outline(&store, "session-1", "agent-1", 2, None)
            .expect("newest page should load");
        assert_eq!(newest.turns.len(), 2);
        assert_eq!(newest.turns[0].user_prompt.entry.text, "prompt 4");
        assert_eq!(newest.turns[1].user_prompt.entry.text, "prompt 5");
        assert_eq!(
            newest
                .next_cursor
                .as_ref()
                .map(|cursor| cursor.before_sequence),
            Some(40)
        );

        let older = load_agent_outline(
            &store,
            "session-1",
            "agent-1",
            2,
            newest
                .next_cursor
                .as_ref()
                .map(|cursor| cursor.before_sequence),
        )
        .expect("older page should load");
        assert_eq!(older.turns.len(), 2);
        assert_eq!(older.turns[0].user_prompt.entry.text, "prompt 2");
        assert_eq!(older.turns[1].user_prompt.entry.text, "prompt 3");
        assert_eq!(
            older
                .next_cursor
                .as_ref()
                .map(|cursor| cursor.before_sequence),
            Some(20)
        );

        let oldest = load_agent_outline(
            &store,
            "session-1",
            "agent-1",
            2,
            older
                .next_cursor
                .as_ref()
                .map(|cursor| cursor.before_sequence),
        )
        .expect("oldest page should load");
        assert_eq!(oldest.turns.len(), 1);
        assert_eq!(oldest.turns[0].user_prompt.entry.text, "prompt 1");
        assert_eq!(oldest.next_cursor, None);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn agent_outline_rehydrates_file_image_attachment_previews() {
        let image_path = std::env::temp_dir().join(format!(
            "arroba-outline-preview-{}-{}.png",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::write(&image_path, b"file-image").expect("fixture image should write");
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let mut entry = SessionHistoryEntry::user_prompt(
            "session-1",
            "attachment-1",
            "agent-1",
            "inspect image",
        );
        entry.attachments = vec![SessionHistoryPromptAttachment {
            url: format!("file://{}", image_path.display()),
            mime: "image/png".to_string(),
            filename: Some("file-screenshot.png".to_string()),
            preview_url: None,
        }];
        let event = HistoryEvent::transcript(10, &entry, context);

        let page_entry = page_entry_from_event(event).expect("page entry should project");

        assert_eq!(
            page_entry
                .entry
                .attachments
                .first()
                .and_then(|attachment| attachment.preview_url.as_deref()),
            Some("data:image/png;base64,ZmlsZS1pbWFnZQ==")
        );

        let _ = std::fs::remove_file(image_path);
    }

    #[test]
    fn agent_outline_synthesizes_turn_for_promptless_provider_activity() {
        let path = std::env::temp_dir().join(format!(
            "arroba-promptless-outline-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let tool = HistoryEvent::transcript(
            1,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderTool,
                Some("tool-1".to_string()),
                r#"{"tool":"bash","status":"completed","input":{"command":"cargo test"}}"#,
            ),
            context,
        );
        store
            .append(&tool)
            .expect("promptless provider activity should append");

        let outline = load_agent_outline(&store, "session-1", "agent-1", 1, None)
            .expect("outline should load");

        assert_eq!(outline.turns.len(), 1);
        assert_eq!(outline.turns[0].turn_id, "run-1");
        assert!(
            outline.turns[0]
                .user_prompt
                .entry
                .text
                .contains("no recorded prompt"),
            "{:?}",
            outline.turns[0].user_prompt
        );
        assert_eq!(outline.turns[0].blobs.len(), 1);
        assert_eq!(
            outline.turns[0].blobs[0].kind,
            SessionHistoryEntryKind::ProviderTool
        );
        assert_eq!(outline.turns[0].blobs[0].summary, "$ cargo test");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn agent_outline_preserves_external_identity_for_promptless_observed_activity() {
        let path = std::env::temp_dir().join(format!(
            "arroba-promptless-external-outline-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open");
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            provider_run_id: Some("run-1".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let tool_entry = SessionHistoryEntry::external_provider_observed(
            "session-1",
            Some("run-1"),
            "agent-1",
            SessionHistoryEntryKind::ProviderTool,
            r#"{"tool":"bash","status":"completed","input":{"command":"cargo test"}}"#,
            "codex",
            "thread-1",
            Some("tool-1".to_string()),
            Some(42),
        );
        let tool = HistoryEvent::transcript(1, &tool_entry, context);
        store
            .append(&tool)
            .expect("promptless external activity should append");

        let outline = load_agent_outline(&store, "session-1", "agent-1", 1, None)
            .expect("outline should load");

        assert_eq!(outline.turns.len(), 1);
        let turn = &outline.turns[0];
        assert_eq!(turn.prompt_origin, PromptOrigin::External);
        assert_eq!(turn.external_provider.as_deref(), Some("codex"));
        assert_eq!(
            turn.external_provider_session_id.as_deref(),
            Some("thread-1")
        );
        assert_eq!(turn.external_provider_turn_id.as_deref(), Some("tool-1"));
        assert!(turn.user_prompt.entry.is_external_provider_observed());
        assert!(
            turn.user_prompt.entry.text.contains("no recorded prompt"),
            "{:?}",
            turn.user_prompt
        );
        assert_eq!(turn.blobs.len(), 1);
        assert_eq!(turn.blobs[0].kind, SessionHistoryEntryKind::ProviderTool);
        assert_eq!(turn.blobs[0].summary, "$ cargo test");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
