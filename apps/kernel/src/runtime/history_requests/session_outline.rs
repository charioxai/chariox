//! Hierarchical transcript history outline loading.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::DaemonError;
use crate::history::{
    HistoryEvent, HistoryEventKind, HistoryEventTurnContext, OperationalHistoryStore,
    STEERING_PROMPT_MERGE_KEY_PREFIX, SessionHistoryEntry, SessionHistoryEntryKind,
};
use crate::local::{
    GetSessionHistoryBlobContentRequest, GetSessionHistoryOutlineRequest, LocalDaemonResponse,
    SessionHistoryOutlineAgent, SessionHistoryOutlineBlob, SessionHistoryOutlineCursor,
    SessionHistoryOutlineTurn, SessionHistoryOutlineTurnLifecycle,
};
use crate::provider::ExternalProviderImportMetadata;
use crate::session::PromptOrigin;
use crate::session_history_page::SessionHistoryPageEntry;

const DEFAULT_LATEST_PROMPT_COUNT: usize = 4;
const MAX_LATEST_PROMPT_COUNT: usize = 20;
const BLOB_ID_PREFIX: &str = "history";

pub(crate) async fn execute_session_history_outline_request(
    operational_history: OperationalHistoryStore,
    request: GetSessionHistoryOutlineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    execute_scoped_session_history_outline_request(operational_history, request, BTreeMap::new())
        .await
}

pub(crate) async fn execute_scoped_session_history_outline_request(
    operational_history: OperationalHistoryStore,
    request: GetSessionHistoryOutlineRequest,
    agent_imports: BTreeMap<String, ExternalProviderImportMetadata>,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let agent_ids = outline_agent_ids(&operational_history, &request)?;
        let latest_prompt_count = request
            .latest_prompt_count
            .unwrap_or(DEFAULT_LATEST_PROMPT_COUNT)
            .clamp(1, MAX_LATEST_PROMPT_COUNT);
        let mut agents = Vec::new();
        for agent_id in agent_ids {
            agents.push(load_scoped_agent_outline(
                &operational_history,
                &request.session_id,
                &agent_id,
                latest_prompt_count,
                request.cursor.as_ref().map(|cursor| cursor.before_sequence),
                agent_imports.get(&agent_id),
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
    execute_scoped_session_history_blob_content_request(operational_history, request, None).await
}

pub(crate) async fn execute_scoped_session_history_blob_content_request(
    operational_history: OperationalHistoryStore,
    request: GetSessionHistoryBlobContentRequest,
    agent_import: Option<ExternalProviderImportMetadata>,
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
            .filter(|event| event_belongs_to_external_import(agent_import.as_ref(), event))
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
    load_scoped_agent_outline(
        operational_history,
        session_id,
        agent_id,
        latest_prompt_count,
        before_sequence,
        None,
    )
}

fn load_scoped_agent_outline(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    latest_prompt_count: usize,
    before_sequence: Option<u64>,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Result<SessionHistoryOutlineAgent, DaemonError> {
    let mut prompts = load_latest_scoped_user_prompt_events(
        operational_history,
        session_id,
        agent_id,
        before_sequence,
        latest_prompt_count.saturating_add(1),
        agent_import,
    )?;
    let has_more = prompts.len() > latest_prompt_count;
    if has_more {
        prompts.remove(0);
    }
    if prompts.is_empty() {
        return load_promptless_agent_outline(
            operational_history,
            session_id,
            agent_id,
            latest_prompt_count,
            before_sequence,
            agent_import,
        );
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
        let events = scoped_history_events(events, agent_import);
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
    latest_prompt_count: usize,
    before_sequence: Option<u64>,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Result<SessionHistoryOutlineAgent, DaemonError> {
    let events = operational_history
        .load_session_events(session_id, Some(agent_id))?
        .into_iter()
        .filter(|event| before_sequence.is_none_or(|sequence| event.sequence < sequence))
        .filter(|event| event_belongs_to_external_import(agent_import, event))
        .collect::<Vec<_>>();
    let mut groups = promptless_turn_groups(events);
    let has_more = groups.len() > latest_prompt_count;
    if has_more {
        groups = groups.split_off(groups.len().saturating_sub(latest_prompt_count));
    }
    if groups.is_empty() {
        return Ok(SessionHistoryOutlineAgent {
            agent_id: agent_id.to_string(),
            turns: Vec::new(),
            next_cursor: None,
        });
    }
    let mut turns = Vec::new();
    let mut seen_turn_ids = BTreeSet::new();
    for (latest_key, turn_events) in &groups {
        if let Some(mut turn) =
            promptless_outline_turn(session_id, agent_id, latest_key, turn_events)
        {
            ensure_unique_outline_turn_id(&mut turn, &mut seen_turn_ids);
            turns.push(turn);
        }
    }
    let next_cursor = has_more
        .then(|| groups.first())
        .flatten()
        .and_then(|(_, events)| events.first())
        .map(|event| SessionHistoryOutlineCursor {
            before_sequence: event.sequence,
        });
    Ok(SessionHistoryOutlineAgent {
        agent_id: agent_id.to_string(),
        turns,
        next_cursor,
    })
}

fn load_latest_scoped_user_prompt_events(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    before_sequence: Option<u64>,
    limit: usize,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    if agent_import.is_none() {
        return operational_history.load_latest_user_prompt_events(
            session_id,
            agent_id,
            before_sequence,
            limit,
        );
    }
    let mut selected_newest_first = Vec::new();
    let batch_size = limit.max(1).saturating_mul(4).max(32);
    let mut next_before_sequence = before_sequence;
    loop {
        let candidates = operational_history.load_latest_user_prompt_events(
            session_id,
            agent_id,
            next_before_sequence,
            batch_size,
        )?;
        if candidates.is_empty() {
            break;
        }
        let oldest_sequence = candidates.first().map(|event| event.sequence);
        for event in candidates.iter().rev() {
            if event_belongs_to_external_import(agent_import, event) {
                selected_newest_first.push(event.clone());
                if selected_newest_first.len() >= limit {
                    break;
                }
            }
        }
        if selected_newest_first.len() >= limit || candidates.len() < batch_size {
            break;
        }
        next_before_sequence = oldest_sequence;
    }
    selected_newest_first.reverse();
    Ok(selected_newest_first)
}

fn scoped_history_events(
    events: Vec<HistoryEvent>,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Vec<HistoryEvent> {
    events
        .into_iter()
        .filter(|event| event_belongs_to_external_import(agent_import, event))
        .collect()
}

fn event_belongs_to_external_import(
    agent_import: Option<&ExternalProviderImportMetadata>,
    event: &HistoryEvent,
) -> bool {
    let Some(entry) = event.to_session_history_entry() else {
        return true;
    };
    if !entry.is_external_provider_observed() {
        return true;
    }
    let entry_provider = normalize_external_provider(entry.external_provider.as_deref());
    let entry_session_id = non_blank_trimmed(entry.external_provider_session_id.as_deref());
    let Some(agent_import) = agent_import else {
        return true;
    };
    let import_provider = normalize_external_provider(Some(&agent_import.external_provider));
    if entry_provider.is_some()
        && import_provider.is_some()
        && entry_provider.as_deref() != import_provider.as_deref()
    {
        return false;
    }
    let Some(entry_session_id) = entry_session_id else {
        return false;
    };
    let import_session_id = non_blank_trimmed(Some(&agent_import.external_provider_session_id));
    let import_provider_session_id =
        non_blank_trimmed(Some(&agent_import.external_provider_session_provider_id));
    Some(entry_session_id.as_str()) == import_session_id.as_deref()
        || Some(entry_session_id.as_str()) == import_provider_session_id.as_deref()
}

fn normalize_external_provider(value: Option<&str>) -> Option<String> {
    non_blank_trimmed(value).map(|value| value.to_ascii_lowercase())
}

fn non_blank_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn promptless_turn_groups(events: Vec<HistoryEvent>) -> Vec<(String, Vec<HistoryEvent>)> {
    let mut groups = Vec::<(String, Vec<HistoryEvent>)>::new();
    for event in events {
        let key = promptless_turn_group_key(&event);
        if let Some((_, events)) = groups.iter_mut().find(|(candidate, _)| candidate == &key) {
            events.push(event);
        } else {
            groups.push((key, vec![event]));
        }
    }
    groups
}

fn promptless_outline_turn(
    session_id: &str,
    agent_id: &str,
    latest_key: &str,
    turn_events: &[HistoryEvent],
) -> Option<SessionHistoryOutlineTurn> {
    let first_event = turn_events.first()?;
    let synthetic_prompt_entry =
        promptless_synthetic_prompt_entry(session_id, agent_id, turn_events);
    let synthetic_prompt = HistoryEvent::transcript(
        first_event.sequence.saturating_sub(1),
        &synthetic_prompt_entry,
        HistoryEventTurnContext {
            session_id: Some(session_id.to_string()),
            agent_id: Some(agent_id.to_string()),
            turn_id: Some(latest_key.to_string()),
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
    events_with_prompt.extend(turn_events.iter().cloned());
    outline_turn_from_events(&synthetic_prompt, events_with_prompt, false)
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
    let lifecycle = outline_turn_lifecycle(completed_at_ms);
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
        lifecycle,
        completed_at_ms,
        user_prompt,
        entries,
        summary,
        blobs,
    })
}

fn outline_turn_lifecycle(completed_at_ms: Option<u64>) -> SessionHistoryOutlineTurnLifecycle {
    if completed_at_ms.is_some() {
        SessionHistoryOutlineTurnLifecycle::Completed
    } else {
        SessionHistoryOutlineTurnLifecycle::Open
    }
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
    if let Some(entry) = prompt.to_session_history_entry() {
        if let Some(prompt_origin) = entry.prompt_origin {
            return prompt_origin;
        }
        if entry.is_external_provider_observed() {
            return PromptOrigin::External;
        }
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

fn page_entry_from_event(event: HistoryEvent) -> Option<SessionHistoryPageEntry> {
    let mut entry = event.to_session_history_entry()?;
    if entry.is_external_provider_observed_state_signal() {
        return None;
    }
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
mod tests;
