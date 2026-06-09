use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Mutex};

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use crate::local::{
    ExternalProviderSessionRecord, ImportExternalProviderAgentRequest,
    ImportExternalProviderSessionRequest, ListExternalProviderSessionsRequest, LocalDaemonRequest,
    LocalDaemonResponse, WatchExternalProviderSessionStatusRequest,
};
use crate::provider::ExternalProviderImportMetadata;
use crate::provider::{LaunchProviderRequest, ProviderResumeState, RuntimeProviderRun};
use crate::session::{CreateSessionRequest, RuntimeSession, SessionAgentDefaults};

const EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
const EXTERNAL_PROVIDER_IMPORTED_ACTIVE_INTERVAL: Duration = Duration::from_secs(1);
const EXTERNAL_PROVIDER_IMPORTED_IDLE_INTERVAL: Duration = Duration::from_secs(20);
const EXTERNAL_PROVIDER_IMPORTED_ACTIVE_WINDOW: Duration = Duration::from_secs(120);

pub(crate) async fn run_external_provider_session_discovery_poller(
    app: Arc<Mutex<DaemonApp>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    refresh_external_provider_session_index(&app).await;
    let mut interval = tokio::time::interval(EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = interval.tick() => {
                refresh_external_provider_session_index(&app).await;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ImportedExternalObserverTarget {
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
    import: ExternalProviderImportMetadata,
}

#[derive(Debug, Clone)]
struct ImportedExternalObserverRead {
    target: ImportedExternalObserverTarget,
    turns: Vec<crate::app::ObservedExternalProviderTurn>,
}

#[derive(Debug, Clone)]
struct ImportedExternalObserverSchedule {
    next_due_at: tokio::time::Instant,
    active_until: Option<tokio::time::Instant>,
    consecutive_errors: u32,
}

impl ImportedExternalObserverSchedule {
    fn due_now(now: tokio::time::Instant) -> Self {
        Self {
            next_due_at: now,
            active_until: Some(now + EXTERNAL_PROVIDER_IMPORTED_ACTIVE_WINDOW),
            consecutive_errors: 0,
        }
    }
}

pub(crate) async fn run_imported_external_provider_transcript_observer(
    app: Arc<Mutex<DaemonApp>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut schedule: BTreeMap<String, ImportedExternalObserverSchedule> = BTreeMap::new();
    let mut interval = tokio::time::interval(EXTERNAL_PROVIDER_IMPORTED_ACTIVE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = interval.tick() => {
                poll_imported_external_provider_transcripts(&app, &mut schedule).await;
            }
        }
    }
}

async fn poll_imported_external_provider_transcripts(
    app: &Arc<Mutex<DaemonApp>>,
    schedule: &mut BTreeMap<String, ImportedExternalObserverSchedule>,
) {
    let now = tokio::time::Instant::now();
    let targets = {
        let app = app.lock().await;
        imported_external_observer_targets(&app)
    };
    let target_keys = targets
        .iter()
        .map(imported_observer_target_key)
        .collect::<BTreeSet<_>>();
    schedule.retain(|key, _| target_keys.contains(key));
    let due = targets
        .into_iter()
        .filter(|target| {
            let key = imported_observer_target_key(target);
            let state = schedule
                .entry(key)
                .or_insert_with(|| ImportedExternalObserverSchedule::due_now(now));
            state.next_due_at <= now
        })
        .collect::<Vec<_>>();
    if due.is_empty() {
        return;
    }
    for target in due {
        let key = imported_observer_target_key(&target);
        let provider = target.import.external_provider.clone();
        let provider_session_id = target.import.external_provider_session_provider_id.clone();
        let read = match tokio::task::spawn_blocking(move || {
            crate::app::read_external_provider_observed_turns(&provider, &provider_session_id)
        })
        .await
        {
            Ok(turns) => Ok(ImportedExternalObserverRead { target, turns }),
            Err(error) => Err(error.to_string()),
        };
        match read {
            Ok(read) => {
                let appended = {
                    let mut app = app.lock().await;
                    append_observed_external_turns_for_import(&mut app, read).unwrap_or_default()
                };
                let state = schedule
                    .entry(key)
                    .or_insert_with(|| ImportedExternalObserverSchedule::due_now(now));
                state.consecutive_errors = 0;
                if appended > 0 {
                    state.active_until = Some(now + EXTERNAL_PROVIDER_IMPORTED_ACTIVE_WINDOW);
                }
                let active = state
                    .active_until
                    .is_some_and(|active_until| active_until > now);
                state.next_due_at = now
                    + if active {
                        EXTERNAL_PROVIDER_IMPORTED_ACTIVE_INTERVAL
                    } else {
                        EXTERNAL_PROVIDER_IMPORTED_IDLE_INTERVAL
                    };
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.external_provider_sessions",
                    "external provider transcript observer read failed",
                    serde_json::json!({
                        "target": key,
                        "error": error,
                    }),
                );
                let state = schedule
                    .entry(key)
                    .or_insert_with(|| ImportedExternalObserverSchedule::due_now(now));
                state.consecutive_errors = state.consecutive_errors.saturating_add(1);
                let backoff_secs = 2_u64.pow(state.consecutive_errors.min(5));
                state.next_due_at = now + Duration::from_secs(backoff_secs);
            }
        }
    }
}

async fn refresh_external_provider_session_index(app: &Arc<Mutex<DaemonApp>>) {
    let discovered =
        match tokio::task::spawn_blocking(|| crate::app::discover_external_provider_sessions(None))
            .await
        {
            Ok(discovered) => discovered,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.external_provider_sessions",
                    "external provider session discovery task failed",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
    let store = {
        let app = app.lock().await;
        app.external_provider_session_index_store()
    };
    for provider in ["codex", "claude", "opencode"] {
        let provider_sessions = discovered
            .iter()
            .filter(|session| session.provider == provider)
            .cloned()
            .collect::<Vec<_>>();
        store.replace_provider_sessions(provider, provider_sessions);
    }
}

pub(crate) async fn execute_external_provider_session_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: LocalDaemonRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let store = {
        let app = app.lock().await;
        app.external_provider_session_index_store()
    };
    match request {
        LocalDaemonRequest::ListExternalProviderSessions(request) => {
            Ok(LocalDaemonResponse::ExternalProviderSessionsListed {
                page: store.list(&request),
            })
        }
        LocalDaemonRequest::RefreshExternalProviderSessions(request) => {
            let provider = request.provider.clone();
            let discovered = crate::app::discover_external_provider_sessions(provider.as_deref());
            if let Some(provider) = provider.as_deref() {
                store.replace_provider_sessions(provider, discovered);
            } else {
                for provider in ["codex", "claude", "opencode"] {
                    let provider_sessions = discovered
                        .iter()
                        .filter(|session| session.provider == provider)
                        .cloned()
                        .collect::<Vec<_>>();
                    store.replace_provider_sessions(provider, provider_sessions);
                }
            }
            let list_request = ListExternalProviderSessionsRequest {
                provider: request.provider,
                cursor: None,
                limit: None,
            };
            Ok(LocalDaemonResponse::ExternalProviderSessionsRefreshed {
                page: store.list(&list_request),
            })
        }
        LocalDaemonRequest::WatchExternalProviderSessionStatus(request) => {
            Ok(watch_status_response(&store, request))
        }
        LocalDaemonRequest::ImportExternalProviderSession(request) => {
            let mut app = app.lock().await;
            import_external_provider_session(&mut app, &store, request, caller_user_id)
        }
        LocalDaemonRequest::ImportExternalProviderAgent(request) => {
            let mut app = app.lock().await;
            import_external_provider_agent(&mut app, &store, request, caller_user_id)
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "external provider session request",
            message: "unsupported external provider session request".to_string(),
        }),
    }
}

fn import_external_provider_session(
    app: &mut DaemonApp,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderSessionRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external = external_session_or_refresh(app, store, &request.external_session_id)?;
    let provider = request
        .provider
        .unwrap_or_else(|| external.provider.clone());
    let model = request
        .model
        .unwrap_or_else(|| default_external_provider_model(&provider).to_string());
    let mut defaults = SessionAgentDefaults::new(provider.clone()).with_model(model.clone());
    if let Some(effort) = request.effort.clone() {
        defaults = defaults.with_effort(effort);
    }
    let session_alias = request
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
        .alias_agent(agent.id(), Some(session_alias))
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
    let import = ExternalProviderImportMetadata::observed_history(
        external.external_session_id.clone(),
        external.provider.clone(),
        external.provider_session_id.clone(),
    );
    let agent = persist_external_import_metadata(app, session.id(), agent.id(), import.clone())?;
    append_observed_external_history(app, &session, &agent, Some(&provider_run), &external);
    store.mark_imported(&external.external_session_id, session.id(), agent.id());
    Ok(LocalDaemonResponse::ExternalProviderSessionImported {
        session: crate::app::KernelSessionReadService::new(app).session_snapshot(session.id())?,
        agent,
        provider_run: Some(provider_run),
    })
}

fn import_external_provider_agent(
    app: &mut DaemonApp,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderAgentRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external = external_session_or_refresh(app, store, &request.external_session_id)?;
    let session = app.sessions().get_session(&request.session_id)?;
    let provider = request
        .provider
        .unwrap_or_else(|| external.provider.clone());
    let model = request
        .model
        .unwrap_or_else(|| default_external_provider_model(&provider).to_string());
    let alias = request
        .alias
        .clone()
        .or_else(|| external.title.clone())
        .unwrap_or_else(|| external.provider_session_id.clone());
    let mut create_request = CreateAgentRequest::new(session.id(), provider.clone())
        .with_alias(alias)
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
    let import = ExternalProviderImportMetadata::observed_history(
        external.external_session_id.clone(),
        external.provider.clone(),
        external.provider_session_id.clone(),
    );
    let agent = persist_external_import_metadata(app, session.id(), agent.id(), import.clone())?;
    append_observed_external_history(app, &session, &agent, Some(&provider_run), &external);
    store.mark_imported(&external.external_session_id, session.id(), agent.id());
    Ok(LocalDaemonResponse::ExternalProviderAgentImported {
        session: crate::app::KernelSessionReadService::new(app).session_snapshot(session.id())?,
        agent,
        provider_run: Some(provider_run),
    })
}

fn external_session_or_refresh(
    _app: &DaemonApp,
    store: &crate::app::ExternalProviderSessionIndexStore,
    external_session_id: &str,
) -> Result<ExternalProviderSessionRecord, DaemonError> {
    if let Some(session) = store.get(external_session_id) {
        return Ok(session);
    }
    let provider = external_session_id
        .split_once(':')
        .map(|(provider, _)| provider);
    for session in crate::app::discover_external_provider_sessions(provider) {
        store.upsert(session);
    }
    store
        .get(external_session_id)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "import external provider session",
            message: format!("external provider session `{external_session_id}` was not found"),
        })
}

fn launch_imported_external_provider(
    app: &mut DaemonApp,
    session: &RuntimeSession,
    agent: &AgentInstance,
    external: &ExternalProviderSessionRecord,
    provider: &str,
    model: &str,
    effort: Option<String>,
) -> Result<RuntimeProviderRun, DaemonError> {
    let mut request =
        LaunchProviderRequest::new(session.id(), provider, provider, "default", model)
            .with_agent_id(agent.id())
            .with_owner_user_id(agent.owner_user_id().to_string())
            .with_resume_state(resume_state_for_external_session(
                provider,
                &external.provider_session_id,
            ))
            .with_external_provider_import(ExternalProviderImportMetadata::observed_history(
                external.external_session_id.clone(),
                external.provider.clone(),
                external.provider_session_id.clone(),
            ));
    if let Some(effort) = effort {
        request = request.with_variant(Some(effort));
    }
    if let Some(worktree_path) = agent.worktree_id().or(external.worktree_path.as_deref()) {
        request = request.with_working_directory(std::path::PathBuf::from(worktree_path));
    }
    app.launch_provider(request)
}

fn append_observed_external_history(
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
    let target = ImportedExternalObserverTarget {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider_run_id,
        import: agent
            .external_provider_import()
            .cloned()
            .unwrap_or_else(|| {
                ExternalProviderImportMetadata::observed_history(
                    external.external_session_id.clone(),
                    external.provider.clone(),
                    external.provider_session_id.clone(),
                )
            }),
    };
    let _ = append_observed_external_turns_for_import(
        app,
        ImportedExternalObserverRead { target, turns },
    );
}

fn append_observed_external_turns_for_import(
    app: &mut DaemonApp,
    read: ImportedExternalObserverRead,
) -> Result<usize, DaemonError> {
    if read.turns.is_empty() {
        return Ok(0);
    }
    let session = app.sessions().get_session(&read.target.session_id)?;
    let agent = app.agents.get_agent(&read.target.agent_id)?;
    let provider_run_id = read.target.provider_run_id.clone().or_else(|| {
        app.providers()
            .get_latest_run_for_agent(session.id(), agent.id())
            .map(|run| run.id().to_string())
    });
    let existing_merge_keys = app
        .load_session_history_entries(&session, Some(agent.id()))?
        .into_iter()
        .filter_map(|entry| entry.merge_key)
        .collect::<BTreeSet<_>>();
    let mut appended = 0usize;
    let mut last_cursor = read.target.import.observed_cursor.clone();
    let provider = read.target.import.external_provider.clone();
    let provider_session_id = read
        .target
        .import
        .external_provider_session_provider_id
        .clone();
    let mut seen_merge_keys = existing_merge_keys;
    for (index, turn) in read.turns.into_iter().enumerate() {
        let kind = match turn.role {
            crate::app::ObservedExternalProviderTurnRole::User => {
                SessionHistoryEntryKind::UserPrompt
            }
            crate::app::ObservedExternalProviderTurnRole::Assistant => {
                SessionHistoryEntryKind::ProviderOutput
            }
        };
        let provider_turn_id = turn
            .provider_turn_id
            .clone()
            .or_else(|| Some(format!("observed-{index}")));
        let merge_key = provider_turn_id
            .as_ref()
            .map(|turn_id| format!("external:{provider}:{provider_session_id}:{turn_id}"));
        if merge_key
            .as_ref()
            .is_some_and(|merge_key| seen_merge_keys.contains(merge_key))
        {
            continue;
        }
        let entry = SessionHistoryEntry::external_provider_observed(
            &read.target.session_id,
            provider_run_id.as_deref(),
            &read.target.agent_id,
            kind,
            turn.text,
            &provider,
            &provider_session_id,
            provider_turn_id.clone(),
            turn.observed_at_ms,
        );
        app.append_history_entry(&read.target.session_id, entry.clone());
        emit_observed_external_history_signal(
            app,
            &read.target,
            provider_run_id.as_deref(),
            &entry,
        );
        if let Some(merge_key) = merge_key {
            seen_merge_keys.insert(merge_key.clone());
            last_cursor.last_observed_merge_key = Some(merge_key);
        }
        last_cursor.last_observed_turn_id = provider_turn_id;
        last_cursor.last_observed_at_ms = turn.observed_at_ms.or(last_cursor.last_observed_at_ms);
        appended += 1;
    }
    if appended > 0 {
        let next_import = read.target.import.clone().with_cursor(last_cursor);
        persist_external_import_metadata(
            app,
            &read.target.session_id,
            &read.target.agent_id,
            next_import,
        )?;
        let _ = crate::app::KernelSessionReadService::new(app)
            .session_snapshot(&read.target.session_id);
    }
    Ok(appended)
}

fn emit_observed_external_history_signal(
    app: &DaemonApp,
    target: &ImportedExternalObserverTarget,
    provider_run_id: Option<&str>,
    entry: &SessionHistoryEntry,
) {
    let Some(provider_run_id) = provider_run_id else {
        return;
    };
    let Ok(agent) = app.agents().get_agent(&target.agent_id) else {
        return;
    };
    let recipient_attachment_ids = app
        .attachments
        .list_session_attachment_ids_for_user(&target.session_id, agent.owner_user_id());
    if recipient_attachment_ids.is_empty() {
        return;
    }
    app.terminal_stream_store().fan_out_output(
        &target.session_id,
        provider_run_id,
        Some(&target.agent_id),
        crate::terminal::TerminalOutputKind::ProviderStatus,
        entry.merge_key.clone(),
        recipient_attachment_ids,
        b"external_provider_history_updated",
    );
}

fn imported_external_observer_targets(app: &DaemonApp) -> Vec<ImportedExternalObserverTarget> {
    app.agents()
        .list_agents()
        .into_iter()
        .filter_map(|agent| {
            let import = agent.external_provider_import()?.clone();
            if !import.import_mode.is_observed_history() {
                return None;
            }
            let session = app.sessions().get_session(agent.session_id()).ok()?;
            let provider_run_id = app
                .providers()
                .get_latest_run_for_agent(session.id(), agent.id())
                .map(|run| run.id().to_string());
            Some(ImportedExternalObserverTarget {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                provider_run_id,
                import,
            })
        })
        .collect()
}

fn imported_observer_target_key(target: &ImportedExternalObserverTarget) -> String {
    format!(
        "{}:{}:{}",
        target.session_id, target.agent_id, target.import.external_provider_session_id
    )
}

fn persist_external_import_metadata(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    import: ExternalProviderImportMetadata,
) -> Result<AgentInstance, DaemonError> {
    let mut session = app
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
    let agents = app.agents().get_session_agents(session_id);
    session.set_agents(agents);
    app.update_session_projection(session);
    Ok(agent)
}

fn resume_state_for_external_session(
    provider: &str,
    provider_session_id: &str,
) -> ProviderResumeState {
    match provider {
        "codex" => ProviderResumeState::from_codex_thread_id(provider_session_id),
        "opencode" => ProviderResumeState::from_opencode_session_id(provider_session_id),
        "claude" => ProviderResumeState::from_claude_session_id(provider_session_id),
        _ => ProviderResumeState::default(),
    }
}

fn default_external_provider_model(provider: &str) -> &'static str {
    match provider {
        "codex" => "gpt-5.5",
        "claude" => "claude-sonnet-4-6",
        _ => "default",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::local::{
        ExternalProviderSessionCapabilities, ExternalProviderSessionMode,
        ImportExternalProviderAgentRequest, ImportExternalProviderSessionRequest,
    };
    use std::sync::Arc;

    #[test]
    fn import_external_provider_session_creates_session_agent_and_run() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let store = {
                let app = app.lock().await;
                app.external_provider_session_index_store()
            };
            store.upsert(record("dev-stub", "external-1", "/tmp/external-one"));

            let response = execute_external_provider_session_request(
                &app,
                LocalDaemonRequest::ImportExternalProviderSession(
                    ImportExternalProviderSessionRequest {
                        external_session_id: "dev-stub:external-1".to_string(),
                        alias: Some("Imported external one".to_string()),
                        provider: Some("dev-stub".to_string()),
                        model: Some("default".to_string()),
                        effort: None,
                        worktree_id: None,
                    },
                ),
                "external-import-user",
            )
            .await
            .expect("import should succeed");

            let LocalDaemonResponse::ExternalProviderSessionImported {
                session,
                agent,
                provider_run,
            } = response
            else {
                panic!("unexpected response")
            };
            assert_eq!(session.alias(), Some("imported_external_one"));
            assert_eq!(session.worktree_id(), "/tmp/external-one");
            assert_eq!(session.owner_user_id(), "external-import-user");
            assert_eq!(agent.provider(), "dev-stub");
            assert_eq!(agent.alias(), Some("Imported external one"));
            assert_eq!(agent.owner_user_id(), "external-import-user");
            let provider_run = provider_run.expect("provider run should launch");
            assert_eq!(provider_run.session_id(), session.id());
            assert_eq!(provider_run.agent_instance_id(), Some(agent.id()));
            assert_eq!(provider_run.adapter_key(), "dev-stub");
            assert_eq!(
                session.external_provider_imports()[0].external_provider_session_id,
                "dev-stub:external-1"
            );
            assert_eq!(
                agent
                    .external_provider_import()
                    .expect("agent import metadata should persist")
                    .external_provider_session_provider_id,
                "external-1"
            );
            assert_eq!(
                provider_run
                    .external_provider_import()
                    .expect("provider run import metadata should persist")
                    .external_provider,
                "dev-stub"
            );
            assert!(
                store
                    .get("dev-stub:external-1")
                    .expect("record should remain indexed")
                    .already_imported
            );
        });
    }

    #[test]
    fn import_external_provider_agent_adds_agent_to_existing_session() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let (session_id, store) = {
                let mut app = app.lock().await;
                let (session, _) = crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new("workspace", "worktree"))
                    .expect("session should create");
                let store = app.external_provider_session_index_store();
                (session.id().to_string(), store)
            };
            store.upsert(record("dev-stub", "external-2", "/tmp/external-two"));

            let response = execute_external_provider_session_request(
                &app,
                LocalDaemonRequest::ImportExternalProviderAgent(
                    ImportExternalProviderAgentRequest {
                        session_id: session_id.clone(),
                        external_session_id: "dev-stub:external-2".to_string(),
                        alias: Some("Imported agent".to_string()),
                        provider: Some("dev-stub".to_string()),
                        model: Some("default".to_string()),
                        effort: None,
                        focus: Some(true),
                    },
                ),
                "external-agent-user",
            )
            .await
            .expect("import should succeed");

            let LocalDaemonResponse::ExternalProviderAgentImported {
                session,
                agent,
                provider_run,
            } = response
            else {
                panic!("unexpected response")
            };
            assert_eq!(session.id(), session_id);
            assert_eq!(session.focused_agent_id(), Some(agent.id()));
            assert_eq!(agent.provider(), "dev-stub");
            assert_eq!(agent.alias(), Some("Imported agent"));
            assert_eq!(agent.owner_user_id(), "external-agent-user");
            assert_eq!(agent.worktree_id(), Some("/tmp/external-two"));
            assert_eq!(
                provider_run
                    .expect("provider run should launch")
                    .agent_instance_id(),
                Some(agent.id())
            );
            assert_eq!(
                session.external_provider_imports()[0].external_provider_session_id,
                "dev-stub:external-2"
            );
            assert_eq!(
                agent
                    .external_provider_import()
                    .expect("agent import metadata should persist")
                    .external_provider_session_provider_id,
                "external-2"
            );
        });
    }

    #[test]
    fn append_observed_external_turns_persists_cursor_without_provider_run() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "codex:thread-observed".to_string(),
            "codex".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let appended = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: ImportedExternalObserverTarget {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    provider_run_id: None,
                    import,
                },
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("item-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "observed reply".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("observed turn should append");

        assert_eq!(appended, 1);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(
            entries[0].source,
            Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved)
        );
        assert_eq!(entries[0].provider_run_id, None);
        assert_eq!(entries[0].external_provider.as_deref(), Some("codex"));
        let persisted = app
            .agents()
            .get_agent(agent.id())
            .expect("agent should exist");
        let cursor = &persisted
            .external_provider_import()
            .expect("metadata should persist")
            .observed_cursor;
        assert_eq!(cursor.last_observed_turn_id.as_deref(), Some("item-1"));
        assert_eq!(cursor.last_observed_at_ms, Some(42));
    }

    #[test]
    fn append_observed_external_turns_signals_attached_terminals_to_refresh_history() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "default",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        let import = ExternalProviderImportMetadata::observed_history(
            "opencode:thread-observed".to_string(),
            "opencode".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let appended = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: ImportedExternalObserverTarget {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    provider_run_id: Some(run.id().to_string()),
                    import,
                },
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("item-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "observed reply".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("observed turn should append");

        assert_eq!(appended, 1);
        let records = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider_run_id, run.id());
        assert_eq!(records[0].agent_id.as_deref(), Some(agent.id()));
        assert_eq!(
            records[0].kind,
            crate::terminal::TerminalOutputKind::ProviderStatus
        );
        assert_eq!(
            records[0].merge_key.as_deref(),
            Some("external:opencode:thread-observed:item-1")
        );
    }

    #[test]
    fn resume_state_maps_known_external_providers() {
        assert_eq!(
            resume_state_for_external_session("codex", "thread-1").codex_thread_id(),
            Some("thread-1")
        );
        assert_eq!(
            resume_state_for_external_session("opencode", "session-1").opencode_session_id(),
            Some("session-1")
        );
        assert_eq!(
            resume_state_for_external_session("claude", "session-2").claude_session_id(),
            Some("session-2")
        );
        assert!(resume_state_for_external_session("dev-stub", "session-3").is_empty());
    }

    fn record(
        provider: &str,
        provider_session_id: &str,
        worktree_path: &str,
    ) -> ExternalProviderSessionRecord {
        ExternalProviderSessionRecord {
            external_session_id: format!("{provider}:{provider_session_id}"),
            provider: provider.to_string(),
            provider_session_id: provider_session_id.to_string(),
            title: Some(provider_session_id.to_string()),
            title_source: Some("test".to_string()),
            first_prompt_preview: Some("test prompt".to_string()),
            created_at_ms: None,
            last_modified_at_ms: 10,
            worktree_path: Some(worktree_path.to_string()),
            account_profile: None,
            running_state: None,
            capabilities: ExternalProviderSessionCapabilities {
                can_resume: true,
                can_read_history: true,
                ..ExternalProviderSessionCapabilities::default()
            },
            mode: ExternalProviderSessionMode::Observed,
            already_imported: false,
            imported_session_ids: Vec::new(),
            imported_agent_ids: Vec::new(),
        }
    }
}

fn watch_status_response(
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: WatchExternalProviderSessionStatusRequest,
) -> LocalDaemonResponse {
    let status = store
        .get(&request.external_session_id)
        .map(|session| {
            if session.already_imported {
                "imported".to_string()
            } else {
                session
                    .running_state
                    .unwrap_or_else(|| "available".to_string())
            }
        })
        .unwrap_or_else(|| "unavailable".to_string());
    LocalDaemonResponse::ExternalProviderSessionWatchStatus {
        external_session_id: request.external_session_id,
        status,
    }
}
