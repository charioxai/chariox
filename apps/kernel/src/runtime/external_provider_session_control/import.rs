use super::*;

pub(super) fn import_external_provider_session(
    app: &mut DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderSessionRequest,
    caller_user_id: &str,
    account_owner_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external =
        external_session_or_refresh(app, runtime_state, store, &request.external_session_id)?;
    ensure_external_session_is_attachable(&external)?;
    ensure_external_session_owner(&external, account_owner_user_id)?;
    let provider = request
        .provider
        .unwrap_or_else(|| external.provider.clone());
    let model = external_provider_import_model(&provider, request.model);
    let mut defaults = SessionAgentDefaults::new(provider.clone())
        .with_account_profile(external.account_profile.clone())
        .with_model(model.clone());
    if let Some(effort) = request.effort.clone() {
        defaults = defaults.with_effort(effort);
    }
    let session_alias = external_provider_import_session_alias(&external, request.alias.as_deref());
    let agent_alias = request
        .alias
        .clone()
        .or_else(|| external.title.clone())
        .unwrap_or_else(|| external.provider_session_id.clone());
    let worktree_id = request
        .worktree_id
        .clone()
        .or_else(|| external.worktree_path.clone())
        .unwrap_or_else(|| format!("external-{}", external.provider_session_id));
    let workspace_id = external
        .worktree_path
        .clone()
        .unwrap_or_else(|| worktree_id.clone());
    let (session, mut agent) = crate::app::KernelSessionService::new(app).create_session(
        CreateSessionRequest::new(workspace_id, worktree_id)
            .with_alias(session_alias.clone())
            .with_agent_defaults(defaults)
            .with_owner_user_id(caller_user_id.to_string()),
    )?;
    agent = app
        .agents
        .alias_agent(agent.id(), Some(agent_alias))
        .unwrap_or(agent);
    app.durable_state_store().append_event(
        "agent.updated",
        Some(agent.id().to_string()),
        serde_json::json!({
            "agent": &agent,
        }),
    )?;
    let provider_run = launch_imported_external_provider(
        app,
        &session,
        &agent,
        &external,
        &provider,
        &model,
        request.effort,
    )?;
    let import = ExternalProviderImportMetadata::observed_history_for_profile(
        external.provider.clone(),
        &external.account_profile,
        external.provider_session_id.clone(),
    );
    let agent = persist_external_import_metadata(app, session.id(), agent.id(), import.clone())?;
    append_observed_external_history(
        app,
        runtime_state,
        &session,
        &agent,
        Some(&provider_run),
        &external,
    );
    store.mark_attached(&external.external_session_id, session.id(), agent.id());
    Ok(LocalDaemonResponse::ExternalProviderSessionImported {
        session: crate::app::KernelSessionReadService::new(app).session_snapshot(session.id())?,
        agent,
        provider_run: Some(provider_run),
    })
}

pub(super) fn import_external_provider_agent(
    app: &mut DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderAgentRequest,
    caller_user_id: &str,
    account_owner_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external =
        external_session_or_refresh(app, runtime_state, store, &request.external_session_id)?;
    ensure_external_session_is_attachable(&external)?;
    ensure_external_session_owner(&external, account_owner_user_id)?;
    let session = app.session_state_store().get_session(&request.session_id)?;
    let provider = request
        .provider
        .unwrap_or_else(|| external.provider.clone());
    let model = external_provider_import_model(&provider, request.model);
    let alias = request
        .alias
        .clone()
        .or_else(|| external.title.clone())
        .unwrap_or_else(|| external.provider_session_id.clone());
    let mut create_request = CreateAgentRequest::new(session.id(), provider.clone())
        .with_alias(alias)
        .with_account_profile(external.account_profile.clone())
        .with_model(model.clone())
        .with_owner_user_id(caller_user_id.to_string());
    if let Some(effort) = request.effort.clone() {
        create_request = create_request.with_effort(effort);
    }
    if let Some(worktree_path) = external.worktree_path.as_deref() {
        create_request = create_request.with_worktree(worktree_path.to_string());
    }
    let agent = crate::app::KernelSessionService::new(app).spawn_agent(create_request)?;
    if request.focus.unwrap_or(true) {
        crate::app::KernelSessionService::new(app).focus_agent(session.id(), agent.id())?;
    }
    let provider_run = launch_imported_external_provider(
        app,
        &session,
        &agent,
        &external,
        &provider,
        &model,
        request.effort,
    )?;
    let import = ExternalProviderImportMetadata::observed_history_for_profile(
        external.provider.clone(),
        &external.account_profile,
        external.provider_session_id.clone(),
    );
    let agent = persist_external_import_metadata(app, session.id(), agent.id(), import.clone())?;
    append_observed_external_history(
        app,
        runtime_state,
        &session,
        &agent,
        Some(&provider_run),
        &external,
    );
    store.mark_attached(&external.external_session_id, session.id(), agent.id());
    Ok(LocalDaemonResponse::ExternalProviderAgentImported {
        session: crate::app::KernelSessionReadService::new(app).session_snapshot(session.id())?,
        agent,
        provider_run: Some(provider_run),
    })
}

pub(super) fn external_session_or_refresh(
    app: &DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    external_session_id: &str,
) -> Result<ExternalProviderSessionRecord, DaemonError> {
    mark_attached_external_provider_sessions(app, runtime_state, store);
    if let Some(session) = store.get(external_session_id) {
        return Ok(session);
    }
    let provider = external_session_id
        .split_once(':')
        .map(|(provider, _)| provider);
    let discovered = runtime_state.map_or_else(
        || crate::app::discover_external_provider_sessions(provider),
        |runtime_state| {
            crate::app::discover_external_provider_sessions_for_profiles(
                &registered_external_provider_profile_roots(runtime_state, None),
                provider,
            )
        },
    );
    for session in discovered {
        store.upsert(session);
    }
    mark_attached_external_provider_sessions(app, runtime_state, store);
    store
        .get(external_session_id)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "import external provider session",
            message: format!("external provider session `{external_session_id}` was not found"),
        })
}

fn ensure_external_session_owner(
    external: &ExternalProviderSessionRecord,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    if external.owner_user_id == caller_user_id {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "import external provider session",
        message: "external provider session was not found".to_string(),
    })
}

pub(super) fn ensure_external_session_is_attachable(
    external: &ExternalProviderSessionRecord,
) -> Result<(), DaemonError> {
    if external.is_attachable_to_chariox() {
        return Ok(());
    }
    let session_label = external.first_attached_session_id().unwrap_or("unknown");
    let agent_label = external.first_attached_agent_id().unwrap_or("unknown");
    Err(DaemonError::LocalTransport {
        operation: "import external provider session",
        message: format!(
            "external provider session `{}` is already attached to Chariox session `{}` agent `{}`",
            external.external_session_id, session_label, agent_label
        ),
    })
}

pub(super) fn launch_imported_external_provider(
    app: &mut DaemonApp,
    session: &RuntimeSession,
    agent: &AgentInstance,
    external: &ExternalProviderSessionRecord,
    provider: &str,
    model: &str,
    effort: Option<String>,
) -> Result<RuntimeProviderRun, DaemonError> {
    let mut request = LaunchProviderRequest::new(
        session.id(),
        provider,
        provider,
        &external.account_profile,
        model,
    )
    .with_agent_id(agent.id())
    .with_owner_user_id(agent.owner_user_id().to_string())
    .with_resume_state(ProviderResumeState::from_external_provider_session(
        provider,
        &external.provider_session_id,
    ))
    .with_external_provider_import(
        ExternalProviderImportMetadata::observed_history_for_profile(
            external.provider.clone(),
            &external.account_profile,
            external.provider_session_id.clone(),
        ),
    );
    if let Some(effort) = effort {
        request = request.with_variant(Some(effort));
    }
    if let Some(worktree_path) = agent.worktree_id().or(external.worktree_path.as_deref()) {
        request = request.with_working_directory(std::path::PathBuf::from(worktree_path));
    }
    app.launch_provider(request)
}
