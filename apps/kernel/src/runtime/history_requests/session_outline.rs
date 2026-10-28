//! Hierarchical transcript history outline loading.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::DaemonError;
use crate::history::{HistoryEvent, HistoryEventKind, OperationalHistoryStore};
use crate::local::{
    GetSessionHistoryBlobContentRequest, GetSessionHistoryBlobContentsRequest,
    GetSessionHistoryOutlineRequest, LocalDaemonResponse, SessionHistoryBlobContentPage,
    SessionHistoryOutlineAgent, SessionHistoryOutlineCursor, SessionHistoryOutlineTurn,
    SessionHistoryOutlineTurnLifecycle,
};
use crate::provider::normalized_observed_prompt_text;
use crate::provider::ExternalProviderImportMetadata;
use crate::session::PromptOrigin;

mod prefetch;
mod projection;

#[cfg(test)]
#[allow(unused_imports)]
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use projection::MAX_OUTLINE_INLINE_CHARS;
#[cfg(test)]
use projection::{blob_id, MAX_OUTLINE_EVENTS_PER_BLOB};
use projection::{
    event_needs_outline_blob, event_projects_as_outline_entry, has_content,
    outline_blob_content_events, outline_blobs_from_events, outline_page_entry_from_event,
    outline_page_entry_from_event_group, page_entry_from_event, parse_blob_id,
    MAX_OUTLINE_INLINE_ENTRIES_PER_TURN,
};

const DEFAULT_LATEST_PROMPT_COUNT: usize = 4;
const MAX_LATEST_PROMPT_COUNT: usize = 20;
const EXTERNAL_OPEN_OBSERVATION_GRACE_MS: u64 = 5 * 60 * 1_000;
const MAX_BLOB_CONTENTS_PER_REQUEST: usize = 64;

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
        if request.include_prefetched_content {
            prefetch::prefetch_outline_blobs(
                &operational_history,
                &request.session_id,
                &mut agents,
                &agent_imports,
            )?;
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
        let page = load_blob_content_page(
            &operational_history,
            &request.session_id,
            &request.agent_id,
            request.blob_id,
            agent_import.as_ref(),
        )?;
        Ok(LocalDaemonResponse::SessionHistoryBlobContent {
            blob_id: page.blob_id,
            entries: page.entries,
        })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history blob content",
        message: error.to_string(),
    })?
}

pub(crate) async fn execute_scoped_session_history_blob_contents_request(
    operational_history: OperationalHistoryStore,
    request: GetSessionHistoryBlobContentsRequest,
    agent_import: Option<ExternalProviderImportMetadata>,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.blob_ids.len() > MAX_BLOB_CONTENTS_PER_REQUEST {
        return Err(DaemonError::LocalTransport {
            operation: "load session history blob contents",
            message: format!(
                "at most {MAX_BLOB_CONTENTS_PER_REQUEST} blobs can be loaded per request"
            ),
        });
    }
    tokio::task::spawn_blocking(move || {
        let blobs = request
            .blob_ids
            .into_iter()
            .map(|blob_id| {
                load_blob_content_page(
                    &operational_history,
                    &request.session_id,
                    &request.agent_id,
                    blob_id,
                    agent_import.as_ref(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LocalDaemonResponse::SessionHistoryBlobContents { blobs })
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "load session history blob contents",
        message: error.to_string(),
    })?
}

fn load_blob_content_page(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    blob_id: String,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Result<SessionHistoryBlobContentPage, DaemonError> {
    let (sequence_start, sequence_end) = parse_blob_id(&blob_id)?;
    let events = operational_history.load_session_events_for_agent_sequence_range(
        session_id,
        agent_id,
        sequence_start,
        sequence_end,
    )?;
    let events = events
        .into_iter()
        .filter(|event| {
            !event.is_completion_check_turn()
                && !event.is_internal_prompt_echo()
                && !event.is_internal_dispatch_diagnostic()
        })
        .collect();
    let entries = outline_blob_content_events(events)
        .into_iter()
        .filter(|event| event_belongs_to_external_import(agent_import, event))
        .filter_map(page_entry_from_event)
        .collect::<Vec<_>>();
    Ok(SessionHistoryBlobContentPage { blob_id, entries })
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

#[cfg(test)]
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
    let mut prompts = load_visible_outline_prompt_events(
        operational_history,
        session_id,
        agent_id,
        latest_prompt_count,
        before_sequence,
        agent_import,
    )?;
    let has_more = prompts.len() > latest_prompt_count;
    if has_more {
        let remove_count = prompts.len().saturating_sub(latest_prompt_count);
        prompts.drain(0..remove_count);
    }
    if prompts.is_empty() {
        return Ok(SessionHistoryOutlineAgent {
            agent_id: agent_id.to_string(),
            prefetched_blobs: None,
            turns: Vec::new(),
            next_cursor: None,
        });
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
        // A hidden check turn's reply sits between two visible prompts.
        events.retain(|event| {
            !event.is_completion_check_turn()
                && !event.is_internal_prompt_echo()
                && !event.is_internal_dispatch_diagnostic()
        });
        let lifecycle_events = scoped_history_events(events, agent_import);
        let events = if outline_turn_prompt_origin(prompt) == PromptOrigin::Chariox {
            suppress_external_observed_events_from_chariox_turn(lifecycle_events.clone())
        } else {
            lifecycle_events.clone()
        };
        if let Some(mut turn) = outline_turn_from_events_with_lifecycle(
            prompt,
            events,
            &lifecycle_events,
            has_newer_prompt,
        ) {
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
        prefetched_blobs: None,
        turns,
        next_cursor,
    })
}

fn load_visible_outline_prompt_events(
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
    latest_prompt_count: usize,
    before_sequence: Option<u64>,
    agent_import: Option<&ExternalProviderImportMetadata>,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    let limit = outline_prompt_candidate_limit(latest_prompt_count, agent_import);
    let mut next_before_sequence = before_sequence;
    let mut prompts = Vec::new();
    loop {
        let mut candidates = load_latest_scoped_user_prompt_events(
            operational_history,
            session_id,
            agent_id,
            next_before_sequence,
            limit,
            agent_import,
        )?;
        let exhausted = candidates.len() < limit;
        let oldest_sequence = candidates.first().map(|event| event.sequence);
        candidates.retain(|prompt| {
            !prompt.is_completion_check_turn()
                && (!prompt.is_internal_prompt_echo()
                    || prompt
                        .prompt_id
                        .as_deref()
                        .is_some_and(crate::session::is_agent_inbox_prompt_id))
                && !external_observed_tool_call_prompt(prompt)
                && !external_recovery_envelope_prompt(prompt)
        });
        suppress_chariox_owned_external_prompt_echoes(
            &mut candidates,
            operational_history,
            session_id,
            agent_id,
        )?;
        // Hidden prompts consume SQL candidates, never the visible page or
        // its lookahead. Keep scanning until an older visible anchor exists.
        candidates.append(&mut prompts);
        prompts = candidates;
        if prompts.len() > latest_prompt_count || exhausted {
            return Ok(prompts);
        }
        next_before_sequence = oldest_sequence;
    }
}

fn outline_prompt_candidate_limit(
    latest_prompt_count: usize,
    _agent_import: Option<&ExternalProviderImportMetadata>,
) -> usize {
    let minimum = latest_prompt_count.saturating_add(1);
    minimum.saturating_mul(8).clamp(32, 128)
}

fn suppress_external_observed_events_from_chariox_turn(
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

fn suppress_chariox_owned_external_prompt_echoes(
    prompts: &mut Vec<HistoryEvent>,
    operational_history: &OperationalHistoryStore,
    session_id: &str,
    agent_id: &str,
) -> Result<(), DaemonError> {
    let chariox_owned_prompts =
        operational_history.load_chariox_owned_prompt_texts(session_id, agent_id)?;
    let chariox_owned_prompt_texts = chariox_owned_prompts
        .iter()
        .filter_map(|text| normalized_observed_prompt_text(text))
        .collect::<BTreeSet<_>>();
    let chariox_owned_workflow_delivery_tokens = chariox_owned_prompts
        .iter()
        .filter_map(|text| workflow_delivery_token(text))
        .collect::<BTreeSet<_>>();
    let chariox_owned_workflow_handoff_payloads = chariox_owned_prompts
        .iter()
        .filter_map(|text| workflow_handoff_payload(text))
        .collect::<Vec<_>>();
    let chariox_owned_workflow_endpoint_prompts = chariox_owned_prompts
        .iter()
        .filter_map(|text| workflow_endpoint_prompt(text))
        .collect::<BTreeSet<_>>();
    if chariox_owned_prompt_texts.is_empty() {
        return Ok(());
    }
    prompts.retain(|prompt| {
        !external_prompt_matches_chariox_owned_text(
            prompt,
            &chariox_owned_prompt_texts,
            &chariox_owned_workflow_delivery_tokens,
            &chariox_owned_workflow_handoff_payloads,
            &chariox_owned_workflow_endpoint_prompts,
        )
    });
    Ok(())
}

fn external_prompt_matches_chariox_owned_text(
    prompt: &HistoryEvent,
    chariox_owned_prompt_texts: &BTreeSet<String>,
    chariox_owned_workflow_delivery_tokens: &BTreeSet<String>,
    chariox_owned_workflow_handoff_payloads: &[serde_json::Value],
    chariox_owned_workflow_endpoint_prompts: &BTreeSet<String>,
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
    chariox_owned_prompt_texts.contains(&text)
        || workflow_delivery_token(&entry.text)
            .is_some_and(|token| chariox_owned_workflow_delivery_tokens.contains(&token))
        || workflow_handoff_payload(&entry.text)
            .is_some_and(|payload| chariox_owned_workflow_handoff_payloads.contains(&payload))
        || workflow_endpoint_prompt(&entry.text)
            .is_some_and(|endpoint| chariox_owned_workflow_endpoint_prompts.contains(&endpoint))
}

fn workflow_endpoint_prompt(text: &str) -> Option<String> {
    const OPEN: &str = "<endpoint-prompt>";
    const CLOSE: &str = "</endpoint-prompt>";
    let start = text.find(OPEN)?.saturating_add(OPEN.len());
    let end = text[start..].find(CLOSE)?.saturating_add(start);
    normalized_observed_prompt_text(&text[start..end])
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

fn external_recovery_envelope_prompt(prompt: &HistoryEvent) -> bool {
    const RECOVERY_OPERATION_PREFIX: &str = "[Chariox recovery operation chariox-recovery:";
    const CODEX_TURN_ABORTED_PREFIX: &str = "<turn_aborted>";

    prompt.kind == HistoryEventKind::UserPrompt
        && prompt.to_session_history_entry().is_some_and(|entry| {
            if !entry.is_external_provider_observed() {
                return false;
            }
            let text = entry.text.trim_start();
            text.starts_with(RECOVERY_OPERATION_PREFIX)
                || text.starts_with(CODEX_TURN_ABORTED_PREFIX)
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

/// Consecutive fragments of one streamed provider message (same run and merge
/// key, whitespace-only ones included) project as one inline entry, so a
/// token-streamed answer neither floods the inline budget nor loses a lone
/// space between words. A message too long to inline stays per fragment.
fn outline_inline_entry_groups<'a>(
    events: &'a [HistoryEvent],
    prompt_sequence: u64,
    summary_sequences: &BTreeSet<u64>,
) -> Vec<Vec<&'a HistoryEvent>> {
    let continues_message = |last: &HistoryEvent, event: &HistoryEvent| {
        last.kind == HistoryEventKind::ProviderOutput
            && event.kind == HistoryEventKind::ProviderOutput
            && last.provider_run_id == event.provider_run_id
            && history_event_merge_key(event).is_some()
            && history_event_merge_key(last) == history_event_merge_key(event)
    };
    let mut groups = Vec::<Vec<&HistoryEvent>>::new();
    let mut open = false;
    for event in events {
        if event.sequence == prompt_sequence || summary_sequences.contains(&event.sequence) {
            continue;
        }
        if !event_projects_as_outline_entry(event) || event_needs_outline_blob(event) {
            open = false;
            continue;
        }
        let continues = open
            && groups
                .last()
                .and_then(|group| group.last())
                .is_some_and(|last| continues_message(last, event));
        if continues
            && event
                .content
                .as_deref()
                .is_some_and(|content| !content.is_empty())
        {
            if let Some(group) = groups.last_mut() {
                group.push(event);
            }
        } else if has_content(event) {
            groups.push(vec![event]);
            open = true;
        }
    }
    groups
        .into_iter()
        .flat_map(|group| {
            let chars = group
                .iter()
                .map(|event| {
                    event
                        .content
                        .as_deref()
                        .map_or(0, |content| content.chars().count())
                })
                .sum::<usize>();
            if chars > MAX_OUTLINE_INLINE_CHARS {
                group
                    .into_iter()
                    .map(|event| vec![event])
                    .collect::<Vec<_>>()
            } else {
                vec![group]
            }
        })
        .collect()
}

#[cfg(test)]
fn outline_turn_from_events(
    prompt: &HistoryEvent,
    events: Vec<HistoryEvent>,
    has_newer_prompt: bool,
) -> Option<SessionHistoryOutlineTurn> {
    let lifecycle_events = events.clone();
    outline_turn_from_events_with_lifecycle(prompt, events, &lifecycle_events, has_newer_prompt)
}

fn outline_turn_from_events_with_lifecycle(
    prompt: &HistoryEvent,
    events: Vec<HistoryEvent>,
    lifecycle_events: &[HistoryEvent],
    has_newer_prompt: bool,
) -> Option<SessionHistoryOutlineTurn> {
    let events = suppress_sparse_legacy_transcript_duplicates(events);
    let external_identity = outline_turn_external_identity(&events);
    let prompt_origin = outline_turn_prompt_origin(prompt);
    let mut user_prompt = outline_page_entry_from_event(prompt.clone())?;
    if prompt
        .prompt_id
        .as_deref()
        .is_some_and(crate::session::is_internal_prompt_echo_id)
    {
        // Retain the turn and reply attribution without presenting kernel
        // instructions as user input after a reconnect.
        user_prompt.entry.text.clear();
        user_prompt.fragment_start = 0;
        user_prompt.fragment_end = 0;
        user_prompt.total_chars = 0;
    }
    project_workflow_user_prompt(&mut user_prompt, prompt_origin);
    if prompt.is_internal_prompt_echo() {
        // Preserve the agent's reply and turn identity without a user instruction row.
        user_prompt.entry.text.clear();
        user_prompt.fragment_start = 0;
        user_prompt.fragment_end = 0;
        user_prompt.total_chars = 0;
    }
    let completed_at_ms =
        outline_turn_completed_at_ms(prompt, lifecycle_events, prompt_origin, has_newer_prompt);
    let lifecycle = outline_turn_lifecycle(completed_at_ms, lifecycle_events);
    let outcome = (lifecycle != SessionHistoryOutlineTurnLifecycle::Open)
        .then(|| outline_turn_outcome(lifecycle_events))
        .flatten();
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
    let inline_entry_groups =
        outline_inline_entry_groups(&events, prompt.sequence, &summary_sequences);
    let overflow_entry_count = inline_entry_groups
        .len()
        .saturating_sub(MAX_OUTLINE_INLINE_ENTRIES_PER_TURN);
    let mut forced_blob_sequences = inline_entry_groups
        .iter()
        .take(overflow_entry_count)
        .flatten()
        .map(|event| event.sequence)
        .collect::<BTreeSet<_>>();
    if summary
        .as_ref()
        .is_some_and(|entry| entry.fragment_end < entry.total_chars)
    {
        forced_blob_sequences.extend(summary_sequences.iter().copied());
    }
    let entries = inline_entry_groups
        .iter()
        .skip(overflow_entry_count)
        .filter_map(|group| outline_page_entry_from_event_group(group))
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
        outcome,
        completed_at_ms,
        user_prompt,
        entries,
        summary,
        blobs,
    })
}

fn project_workflow_user_prompt(
    user_prompt: &mut crate::session_history_page::SessionHistoryPageEntry,
    prompt_origin: PromptOrigin,
) {
    if prompt_origin != PromptOrigin::Chariox {
        return;
    }
    let Some(endpoint_prompt) = workflow_endpoint_prompt(&user_prompt.entry.text) else {
        return;
    };
    user_prompt.entry.text = endpoint_prompt;
    user_prompt.fragment_start = 0;
    user_prompt.total_chars = user_prompt.entry.text.chars().count();
    user_prompt.fragment_end = user_prompt.total_chars;
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LegacyTranscriptDuplicateKey {
    session_id: Option<String>,
    agent_id: Option<String>,
    provider_run_id: Option<String>,
    kind: String,
    timestamp_ms: u64,
    merge_key: String,
    content: String,
}

fn suppress_sparse_legacy_transcript_duplicates(events: Vec<HistoryEvent>) -> Vec<HistoryEvent> {
    let attributed_keys = events
        .iter()
        .filter(|event| event.prompt_id.is_some() || event.turn_id.is_some())
        .filter_map(legacy_transcript_duplicate_key)
        .collect::<BTreeSet<_>>();
    events
        .into_iter()
        .filter(|event| {
            event.prompt_id.is_some()
                || event.turn_id.is_some()
                || legacy_transcript_duplicate_key(event)
                    .is_none_or(|key| !attributed_keys.contains(&key))
        })
        .collect()
}

fn legacy_transcript_duplicate_key(event: &HistoryEvent) -> Option<LegacyTranscriptDuplicateKey> {
    let merge_key = history_event_merge_key(event)?;
    Some(LegacyTranscriptDuplicateKey {
        session_id: event.session_id.clone(),
        agent_id: event.agent_id.clone(),
        provider_run_id: event.provider_run_id.clone(),
        kind: format!("{:?}", event.kind),
        timestamp_ms: event.timestamp_ms,
        merge_key: merge_key.to_string(),
        content: event.content.clone()?,
    })
}

fn outline_turn_lifecycle(
    completed_at_ms: Option<u64>,
    events: &[HistoryEvent],
) -> SessionHistoryOutlineTurnLifecycle {
    if completed_at_ms.is_none() {
        return SessionHistoryOutlineTurnLifecycle::Open;
    }
    if events.iter().any(|event| {
        event
            .metadata
            .get(crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
            == Some("failed")
    }) {
        return SessionHistoryOutlineTurnLifecycle::Failed;
    }
    if events.iter().any(|event| {
        event
            .metadata
            .get(crate::history::PROMPT_SETTLEMENT_STATUS_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
            == Some("cancelled")
    }) {
        return SessionHistoryOutlineTurnLifecycle::Cancelled;
    }
    SessionHistoryOutlineTurnLifecycle::Completed
}

fn outline_turn_outcome(events: &[HistoryEvent]) -> Option<crate::provider::TurnOutcome> {
    events.iter().find_map(|event| {
        if let Some(outcome) = event
            .metadata
            .get(crate::history::TURN_OUTCOME_METADATA_KEY)
        {
            return serde_json::from_value(outcome.clone()).ok();
        }
        #[derive(serde::Deserialize)]
        struct PreOutcomeFailure {
            cause: crate::provider::TurnOutcomeKind,
            retryable: bool,
            message: String,
        }
        let failure: PreOutcomeFailure = serde_json::from_value(
            event
                .metadata
                .get(crate::history::PRE_OUTCOME_FAILURE_METADATA_KEY)?
                .clone(),
        )
        .ok()?;
        Some(crate::provider::TurnOutcome::new(
            failure.cause,
            failure.retryable,
            &failure.message,
        ))
    })
}

fn outline_turn_completed_at_ms(
    prompt: &HistoryEvent,
    events: &[HistoryEvent],
    prompt_origin: PromptOrigin,
    has_newer_prompt: bool,
) -> Option<u64> {
    if let Some(settled_at_ms) = outline_turn_settlement_observed_at_ms(prompt, events) {
        let latest_content_at_ms = events
            .iter()
            .filter(|event| outline_turn_completion_content_is_visible(event))
            .map(|event| event.timestamp_ms)
            .max()
            .unwrap_or(prompt.timestamp_ms);
        return Some(settled_at_ms.max(latest_content_at_ms));
    }
    if prompt_origin == PromptOrigin::Chariox && !has_newer_prompt {
        return None;
    }
    if prompt_origin == PromptOrigin::External
        && !has_newer_prompt
        && !external_turn_observation_is_stale(prompt, events)
    {
        return None;
    }
    Some(
        events
            .iter()
            .filter(|event| outline_turn_completion_content_is_visible(event))
            .map(|event| event.timestamp_ms)
            .max()
            .unwrap_or(prompt.timestamp_ms),
    )
}

fn outline_turn_completion_content_is_visible(event: &HistoryEvent) -> bool {
    if !matches!(
        event.kind,
        HistoryEventKind::ProviderOutput
            | HistoryEventKind::ProviderReasoning
            | HistoryEventKind::ProviderTool
            | HistoryEventKind::ProviderError
            | HistoryEventKind::ProviderStatus
    ) || !has_content(event)
    {
        return false;
    }
    let Some(entry) = event.to_session_history_entry() else {
        return false;
    };
    if entry.is_external_provider_observed_state_signal()
        || entry
            .external_observation
            .as_ref()
            .is_some_and(|observation| observation.passive_telemetry)
    {
        return false;
    }
    event.kind != HistoryEventKind::ProviderStatus || event_projects_as_outline_entry(event)
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

fn outline_turn_settlement_observed_at_ms(
    prompt: &HistoryEvent,
    events: &[HistoryEvent],
) -> Option<u64> {
    // An observer can import provider history from before this prompt (for
    // example after a kernel restart). A settlement observed before the turn
    // began belongs to an earlier provider turn and cannot settle this one.
    let turn_started_at_ms = external_observed_at_ms(prompt).unwrap_or(prompt.timestamp_ms);
    events
        .iter()
        .filter_map(|event| {
            if let Some(settled_at_ms) = persisted_prompt_settlement_at_ms(event) {
                return Some(settled_at_ms);
            }
            let entry = event.to_session_history_entry()?;
            let observation = entry.external_observation.as_ref()?;
            let observed_at_ms = entry.observed_at_ms.unwrap_or(event.timestamp_ms);
            (observation.settles_active_prompt && observed_at_ms >= turn_started_at_ms)
                .then_some(observed_at_ms)
        })
        .max()
}

fn persisted_prompt_settlement_at_ms(event: &HistoryEvent) -> Option<u64> {
    event
        .metadata
        .get(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
        .and_then(serde_json::Value::as_u64)
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
    PromptOrigin::Chariox
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
