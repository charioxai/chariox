//! Hierarchical transcript history outline loading.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::DaemonError;
use crate::history::{
    HistoryEvent, HistoryEventKind, HistoryEventTurnContext, OperationalHistoryStore,
    SessionHistoryEntry, SessionHistoryEntryKind,
};
use crate::local::{
    GetSessionHistoryBlobContentRequest, GetSessionHistoryOutlineRequest, LocalDaemonResponse,
    SessionHistoryOutlineAgent, SessionHistoryOutlineCursor, SessionHistoryOutlineTurn,
    SessionHistoryOutlineTurnLifecycle,
};
use crate::provider::normalized_observed_prompt_text;
use crate::provider::ExternalProviderImportMetadata;
use crate::session::PromptOrigin;

mod projection;

#[cfg(test)]
use projection::{blob_id, MAX_OUTLINE_EVENTS_PER_BLOB, MAX_OUTLINE_INLINE_CHARS};
use projection::{
    event_needs_outline_blob, event_projects_as_outline_entry, has_content,
    outline_blobs_from_events, outline_page_entry_from_event, outline_page_entry_from_event_group,
    page_entry_from_event, parse_blob_id, MAX_OUTLINE_INLINE_ENTRIES_PER_TURN,
};

const DEFAULT_LATEST_PROMPT_COUNT: usize = 4;
const MAX_LATEST_PROMPT_COUNT: usize = 20;
const PROMPTLESS_TEXT: &str = "(no recorded prompt; showing recent agent activity)";
const EXTERNAL_OPEN_OBSERVATION_GRACE_MS: u64 = 5 * 60 * 1_000;

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
        outline_prompt_candidate_limit(latest_prompt_count, agent_import),
        agent_import,
    )?;
    prompts.retain(|prompt| !external_observed_tool_call_prompt(prompt));
    suppress_arroba_owned_external_prompt_echoes(
        &mut prompts,
        operational_history,
        session_id,
        agent_id,
    )?;
    let has_more = prompts.len() > latest_prompt_count;
    if has_more {
        let remove_count = prompts.len().saturating_sub(latest_prompt_count);
        prompts.drain(0..remove_count);
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
        let mut events = operational_history.load_session_events_for_agent_sequence_range(
            session_id,
            agent_id,
            prompt.sequence,
            sequence_end,
        )?;
        if !events
            .iter()
            .any(|event| persisted_prompt_settlement_at_ms(event).is_some())
        {
            if let Some(prompt_id) = prompt.prompt_id.as_deref().or(prompt.turn_id.as_deref()) {
                if let Some(settlement) = operational_history
                    .load_prompt_settlement_event(session_id, agent_id, prompt_id)?
                {
                    events.push(settlement);
                    events.sort_by_key(|event| event.sequence);
                }
            }
        }
        let events = scoped_history_events(events, agent_import);
        let events = if outline_turn_prompt_origin(prompt) == PromptOrigin::Arroba {
            suppress_external_observed_events_from_arroba_turn(events)
        } else {
            events
        };
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

fn outline_prompt_candidate_limit(
    latest_prompt_count: usize,
    _agent_import: Option<&ExternalProviderImportMetadata>,
) -> usize {
    let minimum = latest_prompt_count.saturating_add(1);
    minimum.saturating_mul(8).clamp(32, 128)
}

fn suppress_external_observed_events_from_arroba_turn(
    events: Vec<HistoryEvent>,
) -> Vec<HistoryEvent> {
    events
        .into_iter()
        .filter(|event| !event_is_external_provider_observed(event))
        .collect()
}

fn event_is_external_provider_observed(event: &HistoryEvent) -> bool {
    event
        .to_session_history_entry()
        .is_some_and(|entry| entry.is_external_provider_observed())
}

fn suppress_arroba_owned_external_prompt_echoes(
    prompts: &mut Vec<HistoryEvent>,
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
) -> Result<(), DaemonError> {
    let arroba_owned_prompts =
        operational_history.load_arroba_owned_prompt_texts(session_id, agent_id)?;
    let arroba_owned_prompt_texts = arroba_owned_prompts
        .iter()
        .filter_map(|text| normalized_observed_prompt_text(text))
        .collect::<BTreeSet<_>>();
    let arroba_owned_workflow_delivery_tokens = arroba_owned_prompts
        .iter()
        .filter_map(|text| workflow_delivery_token(text))
        .collect::<BTreeSet<_>>();
    let arroba_owned_workflow_handoff_payloads = arroba_owned_prompts
        .iter()
        .filter_map(|text| workflow_handoff_payload(text))
        .collect::<Vec<_>>();
    if arroba_owned_prompt_texts.is_empty() {
        return Ok(());
    }
    prompts.retain(|prompt| {
        !external_prompt_matches_arroba_owned_text(
            prompt,
            &arroba_owned_prompt_texts,
            &arroba_owned_workflow_delivery_tokens,
            &arroba_owned_workflow_handoff_payloads,
        )
    });
    Ok(())
}

fn external_prompt_matches_arroba_owned_text(
    prompt: &HistoryEvent,
    arroba_owned_prompt_texts: &BTreeSet<String>,
    arroba_owned_workflow_delivery_tokens: &BTreeSet<String>,
    arroba_owned_workflow_handoff_payloads: &[serde_json::Value],
) -> bool {
    if prompt.kind != HistoryEventKind::UserPrompt {
        return false;
    }
    let Some(entry) = prompt.to_session_history_entry() else {
        return false;
    };
    if !entry.is_external_provider_observed() {
        return false;
    }
    let Some(text) = normalized_observed_prompt_text(&entry.text) else {
        return false;
    };
    arroba_owned_prompt_texts.contains(&text)
        || workflow_delivery_token(&entry.text)
            .is_some_and(|token| arroba_owned_workflow_delivery_tokens.contains(&token))
        || workflow_handoff_payload(&entry.text)
            .is_some_and(|payload| arroba_owned_workflow_handoff_payloads.contains(&payload))
}

fn workflow_handoff_payload(text: &str) -> Option<serde_json::Value> {
    const OPEN: &str = "<workflow-handoff-payloads>";
    const CLOSE: &str = "</workflow-handoff-payloads>";
    let start = text.find(OPEN)?.saturating_add(OPEN.len());
    let end = text[start..].find(CLOSE)?.saturating_add(start);
    serde_json::from_str(text[start..end].trim()).ok()
}

fn workflow_delivery_token(text: &str) -> Option<String> {
    const PREFIX: &str = "workflow-ack:";
    let start = text.find(PREFIX)?;
    let token = text[start..]
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':')
        })
        .collect::<String>();
    (token.len() > PREFIX.len()).then_some(token)
}

fn external_observed_tool_call_prompt(prompt: &HistoryEvent) -> bool {
    if prompt.kind != HistoryEventKind::UserPrompt {
        return false;
    }
    let Some(entry) = prompt.to_session_history_entry() else {
        return false;
    };
    if !entry.is_external_provider_observed() {
        return false;
    }
    let text = entry.text.trim_start();
    text.starts_with("Called the ") && text.contains(" tool with the following input:")
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
    let timestamp_ms = events
        .iter()
        .filter_map(|event| external_observed_at_ms(event).or(Some(event.timestamp_ms)))
        .min();
    let mut entry = if let Some(identity) = outline_turn_external_identity(events) {
        SessionHistoryEntry::external_provider_observed(
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
        )
    } else {
        SessionHistoryEntry::user_prompt(session_id, "arroba-history", agent_id, PROMPTLESS_TEXT)
    };
    if let Some(timestamp_ms) = timestamp_ms {
        entry.timestamp_ms = timestamp_ms;
    }
    entry
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
    let user_prompt = outline_page_entry_from_event(prompt.clone())?;
    let external_identity = outline_turn_external_identity(&events);
    let prompt_origin = outline_turn_prompt_origin(prompt);
    let completed_at_ms =
        outline_turn_completed_at_ms(prompt, &events, prompt_origin, has_newer_prompt);
    let lifecycle = outline_turn_lifecycle(completed_at_ms);
    let summary_index = events
        .iter()
        .rposition(|event| event.kind == HistoryEventKind::ProviderOutput && has_content(event));
    let summary_events = summary_index
        .map(|index| trailing_provider_output_group(&events, index))
        .unwrap_or_default();
    let summary_sequences = summary_events
        .iter()
        .map(|event| event.sequence)
        .collect::<BTreeSet<_>>();
    let summary = outline_page_entry_from_event_group(&summary_events);
    let inline_entry_candidates = events
        .iter()
        .filter(|event| event.sequence != prompt.sequence)
        .filter(|event| !summary_sequences.contains(&event.sequence))
        .filter(|event| has_content(event))
        .filter(|event| event_projects_as_outline_entry(event))
        .filter(|event| !event_needs_outline_blob(event))
        .collect::<Vec<_>>();
    let overflow_entry_count = inline_entry_candidates
        .len()
        .saturating_sub(MAX_OUTLINE_INLINE_ENTRIES_PER_TURN);
    let mut forced_blob_sequences = inline_entry_candidates
        .iter()
        .take(overflow_entry_count)
        .map(|event| event.sequence)
        .collect::<BTreeSet<_>>();
    if summary
        .as_ref()
        .is_some_and(|entry| entry.fragment_end < entry.total_chars)
    {
        forced_blob_sequences.extend(summary_sequences.iter().copied());
    }
    let entries = inline_entry_candidates
        .into_iter()
        .skip(overflow_entry_count)
        .cloned()
        .filter_map(outline_page_entry_from_event)
        .collect::<Vec<_>>();
    let blobs = outline_blobs_from_events(
        &events,
        prompt.sequence,
        &summary_sequences,
        &forced_blob_sequences,
    );
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

fn trailing_provider_output_group(
    events: &[HistoryEvent],
    summary_index: usize,
) -> Vec<&HistoryEvent> {
    let summary_event = &events[summary_index];
    let summary_merge_key = history_event_merge_key(summary_event);
    let mut start = summary_index;
    while start > 0 {
        let candidate = &events[start - 1];
        if candidate.kind != HistoryEventKind::ProviderOutput
            || candidate.provider_run_id != summary_event.provider_run_id
            || history_event_merge_key(candidate) != summary_merge_key
        {
            break;
        }
        start -= 1;
    }
    events[start..=summary_index].iter().collect()
}

fn history_event_merge_key(event: &HistoryEvent) -> Option<&str> {
    event
        .metadata
        .get("merge_key")
        .and_then(|value| value.as_str())
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
    if prompt_origin == PromptOrigin::Arroba
        && !has_newer_prompt
        && !is_promptless_synthetic_prompt(prompt)
    {
        return None;
    }
    if prompt_origin == PromptOrigin::External
        && !has_newer_prompt
        && !is_promptless_synthetic_prompt(prompt)
        && !external_turn_observation_is_stale(prompt, events)
    {
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

fn external_turn_observation_is_stale(prompt: &HistoryEvent, events: &[HistoryEvent]) -> bool {
    let latest_observed_at_ms = events
        .iter()
        .filter_map(external_observed_at_ms)
        .max()
        .or_else(|| external_observed_at_ms(prompt));
    latest_observed_at_ms.is_some_and(|observed_at_ms| {
        crate::session::unix_epoch_ms().saturating_sub(observed_at_ms)
            >= EXTERNAL_OPEN_OBSERVATION_GRACE_MS
    })
}

fn external_observed_at_ms(event: &HistoryEvent) -> Option<u64> {
    let entry = event.to_session_history_entry()?;
    entry
        .is_external_provider_observed()
        .then_some(entry.observed_at_ms.unwrap_or(event.timestamp_ms))
}

fn outline_turn_settlement_observed_at_ms(events: &[HistoryEvent]) -> Option<u64> {
    events
        .iter()
        .filter_map(|event| {
            if let Some(settled_at_ms) = persisted_prompt_settlement_at_ms(event) {
                return Some(settled_at_ms);
            }
            let entry = event.to_session_history_entry()?;
            let observation = entry.external_observation.as_ref()?;
            observation
                .settles_active_prompt
                .then_some(entry.observed_at_ms.unwrap_or(event.timestamp_ms))
        })
        .max()
}

fn persisted_prompt_settlement_at_ms(event: &HistoryEvent) -> Option<u64> {
    event
        .metadata
        .get(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
        .and_then(serde_json::Value::as_u64)
}

fn is_promptless_synthetic_prompt(prompt: &HistoryEvent) -> bool {
    prompt.kind == HistoryEventKind::UserPrompt
        && prompt
            .to_session_history_entry()
            .is_some_and(|entry| entry.text == PROMPTLESS_TEXT)
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

#[cfg(test)]
mod tests;
