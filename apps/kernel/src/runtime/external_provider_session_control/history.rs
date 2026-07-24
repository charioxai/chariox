use super::*;

pub(super) fn append_observed_external_history(
    app: &mut DaemonApp,
    session: &RuntimeSession,
    agent: &AgentInstance,
    provider_run: Option<&RuntimeProviderRun>,
    external: &ExternalProviderSessionRecord,
) {
    let turns = crate::app::read_external_provider_observed_turns(
        &external.provider,
        &external.provider_session_id,
    );
    let provider_run_id = provider_run.map(|run| run.id().to_string()).or_else(|| {
        app.providers()
            .get_latest_run_for_agent(session.id(), agent.id())
            .map(|run| run.id().to_string())
    });
    let import = agent
        .external_provider_import()
        .cloned()
        .unwrap_or_else(|| {
            ExternalProviderImportMetadata::observed_history(
                external.external_session_id.clone(),
                external.provider.clone(),
                external.provider_session_id.clone(),
            )
        });
    let target = AttachedExternalObserverTarget {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider_run_id,
        external_session_id: import.external_provider_session_id.clone(),
        provider: import.external_provider.clone(),
        provider_session_id: import.external_provider_session_provider_id.clone(),
        observed_cursor: import.observed_cursor.clone(),
        cursor_source: AttachedExternalObserverCursorSource::Imported(import),
        needs_responsive_refresh: true,
    };
    let _ = append_observed_external_turns_for_attached_target(
        app,
        AttachedExternalObserverRead { target, turns },
    );
}

pub(super) fn append_observed_external_turns_for_attached_target(
    app: &mut DaemonApp,
    read: AttachedExternalObserverRead,
) -> Result<AttachedExternalObserverAppendOutcome, DaemonError> {
    let mut outcome = AttachedExternalObserverAppendOutcome {
        session_id: read.target.session_id.clone(),
        agent_id: read.target.agent_id.clone(),
        provider_run_id: read.target.provider_run_id.clone(),
        ..AttachedExternalObserverAppendOutcome::default()
    };
    if read.turns.is_empty() {
        return Ok(outcome);
    }
    let session = app
        .session_state_store()
        .get_session(&read.target.session_id)?;
    let agent = app.agents.get_agent(&read.target.agent_id)?;
    let (active_prompt, queued_prompts) =
        app.prompt_state_owner().state_parts(&session, agent.id());
    let provider_run_id = read.target.provider_run_id.clone().or_else(|| {
        app.providers()
            .get_latest_run_for_agent(session.id(), agent.id())
            .map(|run| run.id().to_string())
    });
    outcome.provider_run_id = provider_run_id.clone();
    let provider = read.target.provider.clone();
    let provider_session_id = read.target.provider_session_id.clone();
    let external_merge_key_prefix = crate::history::external_provider_observed_merge_key_prefix(
        &provider,
        &provider_session_id,
    );
    let history_index = app
        .operational_history_store()
        .load_external_import_history_index(
            &read.target.session_id,
            &read.target.agent_id,
            &external_merge_key_prefix,
        )?;
    let mut existing_entries_by_merge_key = history_index.external_entries_by_merge_key;
    let mut appended = 0usize;
    let mut active_relevant_appended = 0usize;
    let mut last_changed_entry = None;
    let candidate_turns =
        latest_observed_external_turns_by_merge_key(&read.turns, &provider, &provider_session_id);
    let mut import_state = ObservedExternalTurnImportState::new(
        read.target.observed_cursor.clone(),
        &candidate_turns,
        arroba_owned_prompt_text_counts(
            &history_index.arroba_owned_prompts,
            active_prompt.as_ref(),
            &queued_prompts,
        ),
    );
    for turn in &candidate_turns {
        let observed = import_state.record_turn(turn, &provider, &provider_session_id);
        if observed.is_arroba_owned {
            continue;
        }
        let observed_at_ms = turn.observed_at_ms.or_else(|| {
            existing_entries_by_merge_key
                .get(&observed.merge_key)
                .and_then(|existing| existing.observed_at_ms)
        });
        let mut entry = SessionHistoryEntry::external_provider_observed_with_merge_key(
            &read.target.session_id,
            provider_run_id.as_deref(),
            &read.target.agent_id,
            observed.kind,
            turn.text.clone(),
            &provider,
            &provider_session_id,
            Some(observed.merge_key.clone()),
            Some(observed.provider_turn_id.clone()),
            observed_at_ms,
        );
        entry.external_observation =
            ExternalProviderObservationPolicy::for_provider(&provider).observation_for_turn(turn);
        let has_observable_change = existing_entries_by_merge_key
            .get(&observed.merge_key)
            .is_none_or(|existing| !external_observed_history_entry_matches(existing, &entry));
        if has_observable_change {
            app.replace_history_entry_by_merge_key_or_append(
                &read.target.session_id,
                &observed.merge_key,
                entry.clone(),
            );
            existing_entries_by_merge_key.insert(
                observed.merge_key.clone(),
                ExternalImportHistoryEntry {
                    kind: entry.kind,
                    text: entry.text.clone(),
                    external_provider: entry.external_provider.clone(),
                    external_provider_session_id: entry.external_provider_session_id.clone(),
                    external_provider_turn_id: entry.external_provider_turn_id.clone(),
                    observed_at_ms: entry.observed_at_ms,
                    external_observation: entry.external_observation.clone(),
                },
            );
            last_changed_entry = Some(entry.clone());
            appended += 1;
            if !ExternalProviderObservationPolicy::for_provider(&provider)
                .turn_is_passive_telemetry(turn)
            {
                active_relevant_appended += 1;
            }
        }
        update_provider_run_usage_from_external_observation(
            app,
            provider_run_id.as_deref(),
            &provider,
            turn,
        );
    }
    let changed = appended;
    outcome.changed_count = changed;
    outcome.active_relevant_changed_count = active_relevant_appended;
    let last_cursor = import_state.cursor().clone();
    let cursor_changed = last_cursor != read.target.observed_cursor;
    if changed > 0 || cursor_changed {
        persist_attached_external_observer_cursor(app, &read.target, last_cursor)?;
        let _ = crate::app::KernelSessionReadService::new(app)
            .session_snapshot(&read.target.session_id);
    }
    if let Some(entry) = last_changed_entry.as_ref() {
        emit_observed_external_history_signal(app, &read.target, provider_run_id.as_deref(), entry);
    }
    Ok(outcome)
}

struct ObservedExternalTurnImportState {
    cursor: ExternalProviderObservedCursor,
    visible_provider_turn_id: Option<String>,
    current_observed_turn_is_arroba_owned: bool,
    arroba_owned_provider_turn_ids: BTreeSet<String>,
    candidate_user_turn_ids: BTreeSet<String>,
    arroba_owned_prompt_text_counts: BTreeMap<String, usize>,
}

struct ObservedExternalTurnImportDecision {
    kind: crate::history::SessionHistoryEntryKind,
    provider_turn_id: String,
    merge_key: String,
    is_arroba_owned: bool,
}

impl ObservedExternalTurnImportState {
    fn new(
        cursor: ExternalProviderObservedCursor,
        candidate_turns: &[ObservedExternalProviderTurn],
        arroba_owned_prompt_text_counts: BTreeMap<String, usize>,
    ) -> Self {
        let candidate_user_turn_ids = candidate_turns
            .iter()
            .filter(|turn| turn.role == ObservedExternalProviderTurnRole::User)
            .map(ObservedExternalProviderTurn::provider_turn_id_or_fallback)
            .collect::<BTreeSet<_>>();
        Self {
            arroba_owned_provider_turn_ids: cursor.arroba_owned_observed_prompt_turn_ids.clone(),
            cursor,
            visible_provider_turn_id: None,
            current_observed_turn_is_arroba_owned: false,
            candidate_user_turn_ids,
            arroba_owned_prompt_text_counts,
        }
    }

    fn record_turn(
        &mut self,
        turn: &ObservedExternalProviderTurn,
        provider: &str,
        provider_session_id: &str,
    ) -> ObservedExternalTurnImportDecision {
        let kind = turn.role.session_history_kind();
        let merge_turn_id = turn.provider_turn_id_or_fallback();
        if turn.role == ObservedExternalProviderTurnRole::User {
            self.visible_provider_turn_id = Some(merge_turn_id.clone());
            self.current_observed_turn_is_arroba_owned =
                self.arroba_owned_provider_turn_ids.contains(&merge_turn_id)
                    || consume_arroba_owned_prompt_text_match(
                        &mut self.arroba_owned_prompt_text_counts,
                        &turn.text,
                    );
            if self.current_observed_turn_is_arroba_owned {
                self.arroba_owned_provider_turn_ids
                    .insert(merge_turn_id.clone());
            }
        }
        let provider_turn_id = self
            .visible_provider_turn_id
            .clone()
            .unwrap_or_else(|| merge_turn_id.clone());
        let merge_key = turn.external_merge_key(provider, provider_session_id);
        self.cursor.last_observed_merge_key = Some(merge_key.clone());
        self.cursor.last_observed_turn_id = Some(merge_turn_id);
        self.cursor.last_observed_at_ms = turn.observed_at_ms.or(self.cursor.last_observed_at_ms);
        self.cursor.arroba_owned_observed_prompt_turn_ids =
            observed_arroba_owned_user_turn_ids_in_window(
                &self.arroba_owned_provider_turn_ids,
                &self.candidate_user_turn_ids,
            );
        ObservedExternalTurnImportDecision {
            kind,
            provider_turn_id,
            merge_key,
            is_arroba_owned: self.current_observed_turn_is_arroba_owned,
        }
    }

    fn cursor(&self) -> &ExternalProviderObservedCursor {
        &self.cursor
    }
}

fn arroba_owned_prompt_text_counts(
    history_prompts: &[String],
    active_prompt: Option<&PromptQueueItem>,
    queued_prompts: &std::collections::VecDeque<PromptQueueItem>,
) -> BTreeMap<String, usize> {
    let mut counts = history_prompts
        .iter()
        .filter_map(|text| normalized_observed_prompt_text(text))
        .fold(BTreeMap::<String, usize>::new(), |mut counts, text| {
            *counts.entry(text).or_default() += 1;
            counts
        });
    if let Some(active_prompt) = active_prompt {
        add_arroba_owned_prompt_text_count(&mut counts, active_prompt);
    }
    for queued_prompt in queued_prompts {
        add_arroba_owned_prompt_text_count(&mut counts, queued_prompt);
    }
    counts
}

fn add_arroba_owned_prompt_text_count(
    counts: &mut BTreeMap<String, usize>,
    prompt: &PromptQueueItem,
) {
    if !prompt.is_arroba_owned() {
        return;
    }
    if let Some(text) = normalized_observed_prompt_text(prompt.prompt()) {
        *counts.entry(text).or_default() += 1;
    }
}

pub(super) fn update_provider_run_usage_from_external_observation(
    app: &mut DaemonApp,
    provider_run_id: Option<&str>,
    provider: &str,
    turn: &ObservedExternalProviderTurn,
) {
    if turn.role != ObservedExternalProviderTurnRole::Status {
        return;
    }
    let Some(provider_run_id) = provider_run_id else {
        return;
    };
    let Some(usage) =
        ExternalProviderObservationPolicy::for_provider(provider).status_usage(&turn.text)
    else {
        return;
    };
    match app.providers.record_observed_usage(provider_run_id, usage) {
        Ok(run) => app.update_provider_run_projection(run),
        Err(error) => crate::logging::debug_with_fields(
            "daemon.external_provider",
            "ignored external provider usage update for missing provider run",
            serde_json::json!({
                "provider_run_id": provider_run_id,
                "provider": provider,
                "error": error.to_string(),
            }),
        ),
    }
}

pub(super) fn latest_observed_external_turns_by_merge_key(
    turns: &[ObservedExternalProviderTurn],
    provider: &str,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let mut latest_indices_by_merge_key = BTreeMap::new();
    for (index, turn) in turns.iter().enumerate() {
        latest_indices_by_merge_key.insert(
            turn.external_merge_key(provider, provider_session_id),
            index,
        );
    }
    let latest_indices = latest_indices_by_merge_key
        .into_values()
        .collect::<BTreeSet<_>>();
    turns
        .iter()
        .enumerate()
        .filter_map(|(index, turn)| latest_indices.contains(&index).then_some(turn.clone()))
        .collect()
}

pub(super) fn external_observed_history_entry_matches(
    existing: &ExternalImportHistoryEntry,
    next: &SessionHistoryEntry,
) -> bool {
    existing.kind == next.kind
        && existing.text == next.text
        && existing.external_provider == next.external_provider
        && existing.external_provider_session_id == next.external_provider_session_id
        && existing.external_provider_turn_id == next.external_provider_turn_id
        && existing.observed_at_ms == next.observed_at_ms
        && existing.external_observation == next.external_observation
}

pub(super) fn observed_arroba_owned_user_turn_ids_in_window(
    arroba_owned_provider_turn_ids: &BTreeSet<String>,
    candidate_user_turn_ids: &BTreeSet<String>,
) -> BTreeSet<String> {
    arroba_owned_provider_turn_ids
        .intersection(candidate_user_turn_ids)
        .cloned()
        .collect()
}

pub(super) fn consume_arroba_owned_prompt_text_match(
    counts: &mut BTreeMap<String, usize>,
    observed_text: &str,
) -> bool {
    let Some(text) = normalized_observed_prompt_text(observed_text) else {
        return false;
    };
    let owned_text = if counts.contains_key(&text) {
        text
    } else if text.ends_with("<metaagent-event/>") && counts.contains_key("<metaagent-event/>") {
        "<metaagent-event/>".to_string()
    } else {
        return false;
    };
    let Some(count) = counts.get_mut(&owned_text) else {
        return false;
    };
    if *count == 0 {
        return false;
    }
    *count -= 1;
    if *count == 0 {
        counts.remove(&owned_text);
    }
    true
}

pub(super) fn emit_observed_external_history_signal(
    app: &DaemonApp,
    target: &AttachedExternalObserverTarget,
    provider_run_id: Option<&str>,
    entry: &SessionHistoryEntry,
) {
    let Ok(agent) = app.agents().get_agent(&target.agent_id) else {
        return;
    };
    let recipient_attachment_ids = app
        .attachments
        .list_session_attachment_ids_for_user(&target.session_id, agent.owner_user_id());
    if recipient_attachment_ids.is_empty() {
        return;
    }
    let provider_run_id = provider_run_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("external-observer:{}", target.agent_id));
    let Some(external_observation_metadata) =
        crate::terminal::TerminalOutputExternalObservationMetadata::from_session_history_entry(
            entry,
        )
    else {
        return;
    };
    app.terminal_stream_store()
        .fan_out_external_observed_output(
            &target.session_id,
            &provider_run_id,
            Some(&target.agent_id),
            crate::terminal::TerminalOutputKind::ProviderStatus,
            entry.merge_key.clone(),
            recipient_attachment_ids,
            EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes(),
            external_observation_metadata,
            entry.source_attachment_id.clone(),
        );
}
