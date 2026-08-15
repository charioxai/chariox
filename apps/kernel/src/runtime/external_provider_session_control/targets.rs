use super::*;

pub(super) fn attached_external_observer_targets(
    app: &DaemonApp,
    responsive_targets_only: bool,
) -> Vec<AttachedExternalObserverTarget> {
    let inputs = ExternalObserverRuntimeInputs::capture(app, responsive_targets_only);
    let session_store = app.session_state_store();
    attached_external_observer_targets_from_inputs(&inputs, &session_store)
}

#[derive(Clone)]
pub(super) struct ExternalObserverRuntimeInputs {
    cursor_store: AttachedProviderTranscriptCursorStore,
    agents: Vec<AgentInstance>,
    runs: Vec<RuntimeProviderRun>,
    live_attachment_sessions: BTreeSet<String>,
    responsive_targets_only: bool,
}

impl ExternalObserverRuntimeInputs {
    pub(super) fn capture(app: &DaemonApp, responsive_targets_only: bool) -> Self {
        let agents = app.agents().list_agents();
        let runs = app.providers().list_runs();
        let mut candidate_session_ids = agents
            .iter()
            .map(|agent| agent.session_id().to_string())
            .collect::<BTreeSet<_>>();
        candidate_session_ids.extend(runs.iter().map(|run| run.session_id().to_string()));
        let live_attachment_sessions = candidate_session_ids
            .into_iter()
            .filter(|session_id| {
                !app.attachments
                    .list_session_attachment_ids(session_id)
                    .is_empty()
            })
            .collect();
        Self {
            cursor_store: app.attached_provider_transcript_cursor_store(),
            agents,
            runs,
            live_attachment_sessions,
            responsive_targets_only,
        }
    }

    pub(super) fn known_agent_keys(&self) -> BTreeSet<(String, String)> {
        self.agents
            .iter()
            .map(|agent| (agent.session_id().to_string(), agent.id().to_string()))
            .collect()
    }
}

pub(super) fn attached_external_observer_targets_from_inputs(
    inputs: &ExternalObserverRuntimeInputs,
    session_store: &crate::session::SessionStateStore,
) -> Vec<AttachedExternalObserverTarget> {
    let cursor_store = &inputs.cursor_store;
    let mut targets = BTreeMap::<String, AttachedExternalObserverTarget>::new();
    for agent in &inputs.agents {
        if inputs.responsive_targets_only && agent.external_provider_import().is_none() {
            continue;
        }
        let session_id = agent.session_id();
        if !session_store.has_session(session_id) || agent.remote_execution().is_some() {
            continue;
        }
        let latest_run = inputs
            .runs
            .iter()
            .filter(|run| {
                run.session_id() == session_id && run.agent_instance_id() == Some(agent.id())
            })
            .max_by_key(|run| (run.last_activity_at_ms(), run.started_at_ms()))
            .cloned();
        if !inputs.live_attachment_sessions.contains(session_id)
            && !latest_run.as_ref().is_some_and(provider_run_is_running)
        {
            continue;
        }
        let provider_run_is_active = latest_run.as_ref().is_some_and(provider_run_is_running);
        let provider_run_id = latest_run.as_ref().map(|run| run.id().to_string());
        if let Some(import) = agent.external_provider_import().cloned() {
            let target = attached_external_observer_target_from_import(
                session_id,
                agent.id(),
                provider_run_id.clone(),
                import,
            );
            targets.insert(attached_observer_target_key(&target), target);
        }
        for target in attached_external_observer_targets_from_resume_state(
            cursor_store,
            session_id,
            agent.id(),
            provider_run_id.clone(),
            agent.provider_resume_state(),
            provider_run_is_active,
        ) {
            targets
                .entry(attached_observer_target_key(&target))
                .or_insert(target);
        }
    }
    for run in &inputs.runs {
        let Some(agent_id) = run.agent_instance_id() else {
            continue;
        };
        let Some(agent) = inputs.agents.iter().find(|agent| agent.id() == agent_id) else {
            continue;
        };
        if !session_store.has_session(run.session_id()) || agent.remote_execution().is_some() {
            continue;
        }
        if !inputs.live_attachment_sessions.contains(run.session_id())
            && !provider_run_is_running(run)
        {
            continue;
        }
        for target in attached_external_observer_targets_from_provider_run(cursor_store, run) {
            targets
                .entry(attached_observer_target_key(&target))
                .or_insert(target);
        }
    }
    targets.into_values().collect()
}

pub(super) fn session_has_live_attachment(app: &DaemonApp, session_id: &str) -> bool {
    !app.attachments
        .list_session_attachment_ids(session_id)
        .is_empty()
}

pub(super) fn provider_run_is_running(run: &RuntimeProviderRun) -> bool {
    matches!(
        run.state(),
        ProviderRunState::Starting | ProviderRunState::Running
    )
}

pub(super) fn mark_attached_external_provider_sessions(
    app: &DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
) {
    let attached_refs = attached_external_provider_session_refs(app, runtime_state);
    prune_stale_external_provider_session_refs(app, &attached_refs, store);
    for attachment in attached_refs {
        store.mark_attached(
            &attachment.external_session_id,
            &attachment.session_id,
            &attachment.agent_id,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct AttachedExternalProviderSessionRef {
    pub(super) external_session_id: String,
    pub(super) session_id: String,
    pub(super) agent_id: String,
}

pub(super) fn prune_stale_external_provider_session_refs(
    app: &DaemonApp,
    attached_refs: &BTreeSet<AttachedExternalProviderSessionRef>,
    store: &crate::app::ExternalProviderSessionIndexStore,
) {
    let known_agents = app
        .agents()
        .list_agents()
        .into_iter()
        .map(|agent| (agent.session_id().to_string(), agent.id().to_string()))
        .collect::<BTreeSet<_>>();
    prune_stale_external_provider_session_refs_with_known_agents(
        &known_agents,
        attached_refs,
        store,
    );
}

pub(super) fn prune_stale_external_provider_session_refs_with_known_agents(
    known_agents: &BTreeSet<(String, String)>,
    attached_refs: &BTreeSet<AttachedExternalProviderSessionRef>,
    store: &crate::app::ExternalProviderSessionIndexStore,
) {
    let desired_refs = attached_refs
        .iter()
        .map(|attachment| ExternalProviderSessionAttachmentRef {
            external_session_id: attachment.external_session_id.clone(),
            session_id: attachment.session_id.clone(),
            agent_id: attachment.agent_id.clone(),
        })
        .collect::<BTreeSet<_>>();
    for attachment in store.attachment_refs() {
        if desired_refs.contains(&attachment) {
            continue;
        }
        if !known_agents.contains(&(attachment.session_id.clone(), attachment.agent_id.clone())) {
            continue;
        }
        store.detach_attachment(
            &attachment.external_session_id,
            &attachment.session_id,
            &attachment.agent_id,
        );
    }
}

pub(super) fn attached_external_provider_session_refs(
    app: &DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
) -> BTreeSet<AttachedExternalProviderSessionRef> {
    let session_store = app.session_state_store();
    let mut attached = BTreeSet::new();
    for agent in app.agents().list_agents() {
        let session_id = agent.session_id();
        if !session_store.has_session(session_id) || agent.remote_execution().is_some() {
            continue;
        }
        let latest_run = app
            .providers()
            .get_latest_run_for_agent(session_id, agent.id());
        if !session_has_live_attachment(app, session_id)
            && !latest_run.as_ref().is_some_and(provider_run_is_running)
        {
            continue;
        }
        if let Some(import) = agent.external_provider_import() {
            attached.insert(AttachedExternalProviderSessionRef {
                external_session_id: import.external_provider_session_id.clone(),
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
            });
        }
        push_resume_state_attachments(
            &mut attached,
            agent.provider_resume_state(),
            agent.session_id(),
            agent.id(),
        );
    }
    for run in app.providers().list_runs() {
        if provider_run_targets_remote_agent(app, &run) {
            continue;
        }
        if !session_has_live_attachment(app, run.session_id()) && !provider_run_is_running(&run) {
            continue;
        }
        push_provider_run_attachment(&mut attached, &run);
    }
    if let Some(runtime_state) = runtime_state {
        for run in runtime_state.provider_runs_for_external_session_attachment() {
            if provider_run_targets_remote_agent(app, &run) {
                continue;
            }
            if provider_run_is_running(&run) {
                push_provider_run_attachment(&mut attached, &run);
            }
        }
    }
    attached
}

pub(super) fn attached_external_provider_session_refs_from_inputs(
    inputs: &ExternalObserverRuntimeInputs,
    session_store: &crate::session::SessionStateStore,
    runtime_provider_runs: impl IntoIterator<Item = RuntimeProviderRun>,
) -> BTreeSet<AttachedExternalProviderSessionRef> {
    let mut attached = BTreeSet::new();
    for agent in &inputs.agents {
        let session_id = agent.session_id();
        if !session_store.has_session(session_id) || agent.remote_execution().is_some() {
            continue;
        }
        let latest_run = inputs
            .runs
            .iter()
            .filter(|run| {
                run.session_id() == session_id && run.agent_instance_id() == Some(agent.id())
            })
            .max_by_key(|run| (run.last_activity_at_ms(), run.started_at_ms()));
        if !inputs.live_attachment_sessions.contains(session_id)
            && !latest_run.is_some_and(provider_run_is_running)
        {
            continue;
        }
        if let Some(import) = agent.external_provider_import() {
            attached.insert(AttachedExternalProviderSessionRef {
                external_session_id: import.external_provider_session_id.clone(),
                session_id: session_id.to_string(),
                agent_id: agent.id().to_string(),
            });
        }
        push_resume_state_attachments(
            &mut attached,
            agent.provider_resume_state(),
            session_id,
            agent.id(),
        );
    }
    let all_runs = inputs
        .runs
        .iter()
        .cloned()
        .chain(runtime_provider_runs)
        .collect::<Vec<_>>();
    for run in &all_runs {
        let Some(agent_id) = run.agent_instance_id() else {
            continue;
        };
        if inputs
            .agents
            .iter()
            .find(|agent| agent.id() == agent_id)
            .is_some_and(|agent| agent.remote_execution().is_some())
        {
            continue;
        }
        if !inputs.live_attachment_sessions.contains(run.session_id())
            && !provider_run_is_running(run)
        {
            continue;
        }
        push_provider_run_attachment(&mut attached, run);
    }
    attached
}

pub(super) fn provider_run_targets_remote_agent(app: &DaemonApp, run: &RuntimeProviderRun) -> bool {
    run.agent_instance_id()
        .and_then(|agent_id| app.agents().get_agent(agent_id).ok())
        .is_some_and(|agent| agent.remote_execution().is_some())
}

pub(super) fn push_provider_run_attachment(
    attached: &mut BTreeSet<AttachedExternalProviderSessionRef>,
    run: &RuntimeProviderRun,
) {
    let Some(agent_id) = run.agent_instance_id() else {
        return;
    };
    if let Some(provider_session_id) = run.provider_session_id() {
        if let Some(external_session_id) =
            external_session_id_for_provider_session(run.adapter_key(), provider_session_id)
        {
            attached.insert(AttachedExternalProviderSessionRef {
                external_session_id,
                session_id: run.session_id().to_string(),
                agent_id: agent_id.to_string(),
            });
        }
    }
    push_resume_state_attachments(attached, run.resume_state(), run.session_id(), agent_id);
}

pub(super) fn push_resume_state_attachments(
    attached: &mut BTreeSet<AttachedExternalProviderSessionRef>,
    resume_state: &ProviderResumeState,
    session_id: &str,
    agent_id: &str,
) {
    for (provider, provider_session_id) in resume_state.external_provider_sessions() {
        if let Some(external_session_id) =
            external_session_id_for_provider_session(provider, provider_session_id)
        {
            attached.insert(AttachedExternalProviderSessionRef {
                external_session_id,
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
    }
}

pub(super) fn attached_external_observer_target_from_import(
    session_id: impl Into<String>,
    agent_id: impl Into<String>,
    provider_run_id: Option<String>,
    import: ExternalProviderImportMetadata,
) -> AttachedExternalObserverTarget {
    let session_id = session_id.into();
    let agent_id = agent_id.into();
    AttachedExternalObserverTarget {
        session_id,
        agent_id,
        provider_run_id,
        external_session_id: import.external_provider_session_id.clone(),
        provider: import.external_provider.clone(),
        provider_session_id: import.external_provider_session_provider_id.clone(),
        observed_cursor: import.observed_cursor.clone(),
        cursor_source: AttachedExternalObserverCursorSource::Imported(import),
        needs_responsive_refresh: true,
    }
}

pub(super) fn attached_external_observer_targets_from_provider_run(
    cursor_store: &crate::app::AttachedProviderTranscriptCursorStore,
    run: &RuntimeProviderRun,
) -> Vec<AttachedExternalObserverTarget> {
    let Some(agent_id) = run.agent_instance_id() else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    if let Some(provider_session_id) = run.provider_session_id() {
        if let Some(target) = attached_external_observer_target_from_provider_session(
            cursor_store,
            run.session_id(),
            agent_id,
            Some(run.id().to_string()),
            run.adapter_key(),
            provider_session_id,
            provider_run_is_running(run),
        ) {
            targets.push(target);
        }
    }
    targets.extend(attached_external_observer_targets_from_resume_state(
        cursor_store,
        run.session_id(),
        agent_id,
        Some(run.id().to_string()),
        run.resume_state(),
        provider_run_is_running(run),
    ));
    targets
}

pub(super) fn attached_external_observer_targets_from_resume_state(
    cursor_store: &crate::app::AttachedProviderTranscriptCursorStore,
    session_id: &str,
    agent_id: &str,
    provider_run_id: Option<String>,
    resume_state: &ProviderResumeState,
    needs_responsive_refresh: bool,
) -> Vec<AttachedExternalObserverTarget> {
    resume_state
        .external_provider_sessions()
        .into_iter()
        .filter_map(|(provider, provider_session_id)| {
            attached_external_observer_target_from_provider_session(
                cursor_store,
                session_id,
                agent_id,
                provider_run_id.clone(),
                provider,
                provider_session_id,
                needs_responsive_refresh,
            )
        })
        .collect()
}

pub(super) fn attached_external_observer_target_from_provider_session(
    cursor_store: &crate::app::AttachedProviderTranscriptCursorStore,
    session_id: &str,
    agent_id: &str,
    provider_run_id: Option<String>,
    provider: &str,
    provider_session_id: &str,
    needs_responsive_refresh: bool,
) -> Option<AttachedExternalObserverTarget> {
    let external_session_id =
        external_session_id_for_provider_session(provider, provider_session_id)?;
    let cursor_key = AttachedProviderTranscriptCursorKey::new(
        session_id,
        agent_id,
        provider,
        provider_session_id,
    );
    Some(AttachedExternalObserverTarget {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        provider_run_id,
        external_session_id,
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        observed_cursor: cursor_store.get(&cursor_key),
        cursor_source: AttachedExternalObserverCursorSource::CharioxOwned(cursor_key),
        needs_responsive_refresh,
    })
}

pub(super) fn attached_observer_target_key(target: &AttachedExternalObserverTarget) -> String {
    format!(
        "{}:{}:{}",
        target.session_id, target.agent_id, target.external_session_id
    )
}

pub(super) fn persist_attached_external_observer_cursor(
    app: &mut DaemonApp,
    target: &AttachedExternalObserverTarget,
    cursor: ExternalProviderObservedCursor,
) -> Result<(), DaemonError> {
    match &target.cursor_source {
        AttachedExternalObserverCursorSource::Imported(import) => {
            let next_import = import.clone().with_cursor(cursor);
            persist_external_import_metadata(
                app,
                &target.session_id,
                &target.agent_id,
                next_import,
            )?;
        }
        AttachedExternalObserverCursorSource::CharioxOwned(key) => {
            app.attached_provider_transcript_cursor_store()
                .set(key.clone(), cursor);
        }
    }
    Ok(())
}

pub(super) fn persist_external_import_metadata(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    import: ExternalProviderImportMetadata,
) -> Result<AgentInstance, DaemonError> {
    let session = app
        .sessions_mut()
        .upsert_external_provider_import(session_id, import.clone())?;
    let agent = app
        .agents()
        .set_external_provider_import(agent_id, Some(import.clone()))?;
    app.durable_state_store().append_event(
        "session.updated",
        Some(session.id().to_string()),
        serde_json::json!({ "session": &session }),
    )?;
    app.durable_state_store().append_event(
        "agent.updated",
        Some(agent.id().to_string()),
        serde_json::json!({ "agent": &agent }),
    )?;
    let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session.id())?;
    Ok(agent)
}
