use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Mutex};

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::{external_session_id_for_provider_session, DaemonApp};
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use crate::local::{
    ExternalProviderSessionRecord, ImportExternalProviderAgentRequest,
    ImportExternalProviderSessionRequest, ListExternalProviderSessionsRequest, LocalDaemonRequest,
    LocalDaemonResponse,
};
use crate::provider::{
    ExternalProviderImportMetadata, LaunchProviderRequest, ProviderResumeState, RuntimeProviderRun,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{
    CreateSessionRequest, PromptOrigin, PromptQueueItem, PromptStatus, RuntimeSession,
    SessionAgentDefaults,
};

const EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
const EXTERNAL_PROVIDER_IMPORTED_ACTIVE_INTERVAL: Duration = Duration::from_secs(1);
const EXTERNAL_PROVIDER_IMPORTED_IDLE_INTERVAL: Duration = Duration::from_secs(20);
const EXTERNAL_PROVIDER_IMPORTED_ACTIVE_WINDOW: Duration = Duration::from_secs(120);
const EXTERNAL_PROVIDER_IMPORTED_SETTLE_GRACE: Duration = Duration::from_secs(4);
const EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN: usize = 64;

#[derive(Debug, Default)]
struct ExternalProviderSessionDiscoveryCache {
    signature: Option<crate::app::ExternalProviderSessionDiscoverySignature>,
}

pub(crate) async fn run_external_provider_session_discovery_poller(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: KernelRuntimeState,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut cache = ExternalProviderSessionDiscoveryCache::default();
    refresh_external_provider_session_index(&app, Some(&runtime_state), Some(&mut cache), false)
        .await;
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
                refresh_external_provider_session_index(&app, Some(&runtime_state), Some(&mut cache), false).await;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImportedExternalObserverAppendOptions {
    allow_external_active_prompt_settlement: bool,
}

impl Default for ImportedExternalObserverAppendOptions {
    fn default() -> Self {
        Self {
            allow_external_active_prompt_settlement: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ImportedExternalObserverAppendOutcome {
    changed_count: usize,
    active_relevant_changed_count: usize,
    external_active_prompt_settled: bool,
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ImportedExternalObserverSchedule {
    next_due_at: tokio::time::Instant,
    active_until: Option<tokio::time::Instant>,
    last_changed_at: Option<tokio::time::Instant>,
    consecutive_errors: u32,
}

impl ImportedExternalObserverSchedule {
    fn due_now(now: tokio::time::Instant) -> Self {
        Self {
            next_due_at: now,
            active_until: Some(now + EXTERNAL_PROVIDER_IMPORTED_ACTIVE_WINDOW),
            last_changed_at: None,
            consecutive_errors: 0,
        }
    }
}

pub(crate) async fn run_imported_external_provider_transcript_observer(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: crate::runtime::state::KernelRuntimeState,
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
                poll_imported_external_provider_transcripts(&app, &runtime_state, &mut schedule).await;
            }
        }
    }
}

async fn poll_imported_external_provider_transcripts(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: &crate::runtime::state::KernelRuntimeState,
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
        let allow_external_active_prompt_settlement = schedule
            .get(&key)
            .and_then(|state| state.last_changed_at)
            .is_some_and(|last_changed_at| {
                now.duration_since(last_changed_at) >= EXTERNAL_PROVIDER_IMPORTED_SETTLE_GRACE
            });
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
                let outcome = {
                    let mut app = app.lock().await;
                    append_observed_external_turns_for_import_with_options(
                        &mut app,
                        read,
                        ImportedExternalObserverAppendOptions {
                            allow_external_active_prompt_settlement,
                        },
                    )
                    .unwrap_or_default()
                };
                if outcome.external_active_prompt_settled {
                    if let Some(provider_run_id) = outcome.provider_run_id.as_deref() {
                        if let Err(error) = runtime_state
                            .dispatch_next_queued_prompt_after_external_settlement(
                                &outcome.session_id,
                                &outcome.agent_id,
                                provider_run_id,
                            )
                            .await
                        {
                            crate::logging::warn_with_fields(
                                "daemon.external_provider_sessions",
                                "failed to dispatch queued prompt after external provider turn settled",
                                serde_json::json!({
                                    "session_id": outcome.session_id,
                                    "agent_id": outcome.agent_id,
                                    "provider_run_id": provider_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                }
                let state = schedule
                    .entry(key)
                    .or_insert_with(|| ImportedExternalObserverSchedule::due_now(now));
                state.consecutive_errors = 0;
                if outcome.active_relevant_changed_count > 0 {
                    state.active_until = Some(now + EXTERNAL_PROVIDER_IMPORTED_ACTIVE_WINDOW);
                    state.last_changed_at = Some(now);
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

async fn refresh_external_provider_session_index(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    mut cache: Option<&mut ExternalProviderSessionDiscoveryCache>,
    force: bool,
) {
    let signature = match tokio::task::spawn_blocking(|| {
        crate::app::external_provider_session_discovery_signature(None)
    })
    .await
    {
        Ok(signature) => signature,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.external_provider_sessions",
                "external provider session signature task failed",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            return;
        }
    };
    if !force && cache.as_ref().and_then(|cache| cache.signature.as_ref()) == Some(&signature) {
        return;
    }
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
    if let Some(cache) = cache.as_mut() {
        cache.signature = Some(signature);
    }
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
    let app = app.lock().await;
    mark_attached_external_provider_sessions(&app, runtime_state, &store);
}

pub(crate) async fn execute_external_provider_session_request(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    request: LocalDaemonRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let store = {
        let app = app.lock().await;
        app.external_provider_session_index_store()
    };
    match request {
        LocalDaemonRequest::ListExternalProviderSessions(request) => {
            {
                let app = app.lock().await;
                mark_attached_external_provider_sessions(&app, runtime_state, &store);
            }
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
            {
                let app = app.lock().await;
                mark_attached_external_provider_sessions(&app, runtime_state, &store);
            }
            refresh_imported_external_provider_histories(
                app,
                runtime_state,
                request.provider.as_deref(),
            )
            .await;
            let list_request = ListExternalProviderSessionsRequest {
                provider: request.provider,
                cursor: None,
                limit: None,
            };
            Ok(LocalDaemonResponse::ExternalProviderSessionsRefreshed {
                page: store.list(&list_request),
            })
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

async fn refresh_imported_external_provider_histories(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    provider_filter: Option<&str>,
) {
    let targets = {
        let app = app.lock().await;
        imported_external_observer_targets(&app)
            .into_iter()
            .filter(|target| {
                provider_filter
                    .map(|provider| target.import.external_provider == provider)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>()
    };
    for target in targets {
        let provider = target.import.external_provider.clone();
        let provider_session_id = target.import.external_provider_session_provider_id.clone();
        let read = match tokio::task::spawn_blocking(move || {
            crate::app::read_external_provider_observed_turns(&provider, &provider_session_id)
        })
        .await
        {
            Ok(turns) => ImportedExternalObserverRead { target, turns },
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.external_provider_sessions",
                    "external provider session refresh failed to read imported history",
                    serde_json::json!({ "error": error.to_string() }),
                );
                continue;
            }
        };
        let outcome = {
            let mut app = app.lock().await;
            append_observed_external_turns_for_import(&mut app, read).unwrap_or_default()
        };
        if outcome.external_active_prompt_settled {
            if let (Some(runtime_state), Some(provider_run_id)) =
                (runtime_state, outcome.provider_run_id.as_deref())
            {
                if let Err(error) = runtime_state
                    .dispatch_next_queued_prompt_after_external_settlement(
                        &outcome.session_id,
                        &outcome.agent_id,
                        provider_run_id,
                    )
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.external_provider_sessions",
                        "external provider session refresh failed to dispatch queued prompt after external settlement",
                        serde_json::json!({
                            "session_id": outcome.session_id,
                            "agent_id": outcome.agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }
}

fn import_external_provider_session(
    app: &mut DaemonApp,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderSessionRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external = external_session_or_refresh(app, store, &request.external_session_id)?;
    ensure_external_session_is_attachable(&external)?;
    let provider = request
        .provider
        .unwrap_or_else(|| external.provider.clone());
    let model = external_provider_import_model(&provider, request.model);
    let mut defaults = SessionAgentDefaults::new(provider.clone()).with_model(model.clone());
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
    let import = ExternalProviderImportMetadata::observed_history(
        external.external_session_id.clone(),
        external.provider.clone(),
        external.provider_session_id.clone(),
    );
    let agent = persist_external_import_metadata(app, session.id(), agent.id(), import.clone())?;
    append_observed_external_history(app, &session, &agent, Some(&provider_run), &external);
    store.mark_attached(&external.external_session_id, session.id(), agent.id());
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
    ensure_external_session_is_attachable(&external)?;
    let session = app.sessions().get_session(&request.session_id)?;
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
    store.mark_attached(&external.external_session_id, session.id(), agent.id());
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

fn ensure_external_session_is_attachable(
    external: &ExternalProviderSessionRecord,
) -> Result<(), DaemonError> {
    if !external.attached_to_arroba {
        return Ok(());
    }
    let session_label = external
        .attached_session_ids
        .first()
        .map(String::as_str)
        .unwrap_or("unknown");
    let agent_label = external
        .attached_agent_ids
        .first()
        .map(String::as_str)
        .unwrap_or("unknown");
    Err(DaemonError::LocalTransport {
        operation: "import external provider session",
        message: format!(
            "external provider session `{}` is already attached to Arroba session `{}` agent `{}`",
            external.external_session_id, session_label, agent_label
        ),
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
) -> Result<ImportedExternalObserverAppendOutcome, DaemonError> {
    append_observed_external_turns_for_import_with_options(
        app,
        read,
        ImportedExternalObserverAppendOptions::default(),
    )
}

fn append_observed_external_turns_for_import_with_options(
    app: &mut DaemonApp,
    read: ImportedExternalObserverRead,
    options: ImportedExternalObserverAppendOptions,
) -> Result<ImportedExternalObserverAppendOutcome, DaemonError> {
    let mut outcome = ImportedExternalObserverAppendOutcome {
        session_id: read.target.session_id.clone(),
        agent_id: read.target.agent_id.clone(),
        provider_run_id: read.target.provider_run_id.clone(),
        ..ImportedExternalObserverAppendOutcome::default()
    };
    if read.turns.is_empty() {
        return Ok(outcome);
    }
    let session = app.sessions().get_session(&read.target.session_id)?;
    let agent = app.agents.get_agent(&read.target.agent_id)?;
    let provider_run_id = read.target.provider_run_id.clone().or_else(|| {
        app.providers()
            .get_latest_run_for_agent(session.id(), agent.id())
            .map(|run| run.id().to_string())
    });
    outcome.provider_run_id = provider_run_id.clone();
    let provider = read.target.import.external_provider.clone();
    let provider_session_id = read
        .target
        .import
        .external_provider_session_provider_id
        .clone();
    let external_merge_key_prefix = format!("external:{provider}:{provider_session_id}:");
    let cursor_merge_key = read
        .target
        .import
        .observed_cursor
        .last_observed_merge_key
        .as_deref();
    let has_cursor = cursor_merge_key.is_some();
    let mut cursor_seen = !has_cursor;
    let mut candidate_turns = Vec::new();
    for turn in &read.turns {
        let merge_key = observed_external_turn_merge_key(&external_merge_key_prefix, turn);
        if !cursor_seen {
            if merge_key.as_deref() == cursor_merge_key {
                cursor_seen = true;
            }
            continue;
        }
        candidate_turns.push(turn);
    }
    let (arroba_owned_prompt_history, existing_merge_keys) = if has_cursor && cursor_seen {
        (
            app.operational_history_store()
                .load_arroba_owned_prompt_texts(&read.target.session_id, &read.target.agent_id)?,
            BTreeSet::new(),
        )
    } else {
        candidate_turns = read.turns.iter().collect();
        app.operational_history_store().load_external_import_index(
            &read.target.session_id,
            &read.target.agent_id,
            &external_merge_key_prefix,
        )?
    };
    let mut arroba_owned_prompt_texts = arroba_owned_prompt_history
        .iter()
        .filter_map(|text| normalized_observed_prompt_text(text))
        .collect::<BTreeSet<_>>();
    if let Some(prompt_state) = session.prompt_states().get(agent.id()) {
        if let Some(active_prompt) = prompt_state.active_prompt() {
            if active_prompt.prompt_origin() != PromptOrigin::External {
                if let Some(text) = normalized_observed_prompt_text(active_prompt.prompt()) {
                    arroba_owned_prompt_texts.insert(text);
                }
            }
        }
        for queued_prompt in prompt_state.queued_prompts() {
            if queued_prompt.prompt_origin() != PromptOrigin::External {
                if let Some(text) = normalized_observed_prompt_text(queued_prompt.prompt()) {
                    arroba_owned_prompt_texts.insert(text);
                }
            }
        }
    }
    let mut appended = 0usize;
    let mut active_relevant_appended = 0usize;
    let mut last_cursor = read.target.import.observed_cursor.clone();
    let mut visible_provider_turn_id = latest_observed_user_turn_id(&read.turns);
    let mut seen_merge_keys = existing_merge_keys;
    let mut current_observed_turn_is_arroba_owned = false;
    for turn in candidate_turns {
        let kind = match turn.role {
            crate::app::ObservedExternalProviderTurnRole::User => {
                SessionHistoryEntryKind::UserPrompt
            }
            crate::app::ObservedExternalProviderTurnRole::Assistant => {
                SessionHistoryEntryKind::ProviderOutput
            }
            crate::app::ObservedExternalProviderTurnRole::Reasoning => {
                SessionHistoryEntryKind::ProviderReasoning
            }
            crate::app::ObservedExternalProviderTurnRole::Tool => {
                SessionHistoryEntryKind::ProviderTool
            }
            crate::app::ObservedExternalProviderTurnRole::Status => {
                SessionHistoryEntryKind::ProviderStatus
            }
        };
        let merge_turn_id = turn
            .provider_turn_id
            .clone()
            .or_else(|| Some(turn.stable_fallback_id()));
        if turn.role == crate::app::ObservedExternalProviderTurnRole::User {
            visible_provider_turn_id = merge_turn_id.clone();
        }
        let provider_turn_id = visible_provider_turn_id
            .clone()
            .or_else(|| merge_turn_id.clone());
        let merge_key = merge_turn_id
            .as_ref()
            .map(|turn_id| format!("external:{provider}:{provider_session_id}:{turn_id}"));
        if turn.role == crate::app::ObservedExternalProviderTurnRole::User {
            current_observed_turn_is_arroba_owned = normalized_observed_prompt_text(&turn.text)
                .is_some_and(|text| arroba_owned_prompt_texts.contains(&text));
        }
        if current_observed_turn_is_arroba_owned {
            if let Some(merge_key) = merge_key {
                last_cursor.last_observed_merge_key = Some(merge_key);
            }
            last_cursor.last_observed_turn_id = merge_turn_id;
            last_cursor.last_observed_at_ms =
                turn.observed_at_ms.or(last_cursor.last_observed_at_ms);
            continue;
        }
        let entry = SessionHistoryEntry::external_provider_observed_with_merge_key(
            &read.target.session_id,
            provider_run_id.as_deref(),
            &read.target.agent_id,
            kind,
            turn.text.clone(),
            &provider,
            &provider_session_id,
            merge_key.clone(),
            provider_turn_id.clone(),
            turn.observed_at_ms,
        );
        let is_duplicate = merge_key
            .as_ref()
            .is_some_and(|merge_key| seen_merge_keys.contains(merge_key));
        if !is_duplicate {
            app.append_history_entry(&read.target.session_id, entry.clone());
            emit_observed_external_history_signal(
                app,
                &read.target,
                provider_run_id.as_deref(),
                &entry,
            );
            appended += 1;
            if !external_observed_turn_is_passive_telemetry(&provider, turn) {
                active_relevant_appended += 1;
            }
        }
        if let Some(merge_key) = merge_key {
            seen_merge_keys.insert(merge_key.clone());
            last_cursor.last_observed_merge_key = Some(merge_key);
        }
        last_cursor.last_observed_turn_id = merge_turn_id;
        last_cursor.last_observed_at_ms = turn.observed_at_ms.or(last_cursor.last_observed_at_ms);
    }
    let changed = appended;
    outcome.changed_count = changed;
    outcome.active_relevant_changed_count = active_relevant_appended;
    let latest_active_prompt = external_active_prompt_from_turns(
        &read.target,
        &read.turns,
        active_relevant_appended > 0,
        &arroba_owned_prompt_texts,
    );
    let latest_observation_settles = latest_effective_observed_turn_settles(&provider, &read.turns);
    let should_sync_active_prompt = latest_active_prompt.is_some()
        || latest_observation_settles
        || (changed == 0 && options.allow_external_active_prompt_settlement);
    let active_prompt_changed = if should_sync_active_prompt {
        app.prompt_owner_sync_external_active_prompt(
            &read.target.session_id,
            &read.target.agent_id,
            latest_active_prompt.clone(),
        )?
    } else {
        false
    };
    outcome.external_active_prompt_settled =
        active_prompt_changed && latest_active_prompt.is_none();
    let cursor_changed = last_cursor != read.target.import.observed_cursor;
    let state_signal_merge_key = last_cursor.last_observed_merge_key.clone();
    if changed > 0 || cursor_changed {
        let next_import = read.target.import.clone().with_cursor(last_cursor);
        persist_external_import_metadata(
            app,
            &read.target.session_id,
            &read.target.agent_id,
            next_import,
        )?;
        let _ = crate::app::KernelSessionReadService::new(app)
            .session_snapshot(&read.target.session_id);
    } else if active_prompt_changed {
        let _ = crate::app::KernelSessionReadService::new(app)
            .session_snapshot(&read.target.session_id);
    }
    if active_prompt_changed {
        emit_observed_external_state_signal(
            app,
            &read.target,
            provider_run_id.as_deref(),
            state_signal_merge_key.as_deref(),
            if latest_active_prompt.is_some() {
                "active_prompt_started"
            } else {
                "active_prompt_settled"
            },
        );
    }
    Ok(outcome)
}

fn observed_external_turn_merge_key(
    external_merge_key_prefix: &str,
    turn: &crate::app::ObservedExternalProviderTurn,
) -> Option<String> {
    turn.provider_turn_id
        .clone()
        .or_else(|| Some(turn.stable_fallback_id()))
        .map(|turn_id| format!("{external_merge_key_prefix}{turn_id}"))
}

fn normalized_observed_prompt_text(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn external_active_prompt_from_turns(
    target: &ImportedExternalObserverTarget,
    turns: &[crate::app::ObservedExternalProviderTurn],
    has_new_observations: bool,
    arroba_owned_prompt_texts: &BTreeSet<String>,
) -> Option<PromptQueueItem> {
    if latest_effective_observed_turn_settles(&target.import.external_provider, turns) {
        return None;
    }
    let latest = if has_new_observations {
        turns
            .iter()
            .rev()
            .find(|turn| turn.role == crate::app::ObservedExternalProviderTurnRole::User)?
    } else {
        let latest = turns
            .iter()
            .rev()
            .find(|turn| {
                !external_status_turn_is_passive_telemetry(
                    &target.import.external_provider,
                    &turn.text,
                )
            })
            .or_else(|| turns.last())?;
        match latest.role {
            crate::app::ObservedExternalProviderTurnRole::Assistant
                if !external_provider_uses_explicit_completion(
                    &target.import.external_provider,
                ) =>
            {
                return None;
            }
            crate::app::ObservedExternalProviderTurnRole::Status
                if external_status_turn_settles(&latest.text) =>
            {
                return None;
            }
            crate::app::ObservedExternalProviderTurnRole::User => latest,
            crate::app::ObservedExternalProviderTurnRole::Assistant
            | crate::app::ObservedExternalProviderTurnRole::Reasoning
            | crate::app::ObservedExternalProviderTurnRole::Tool
            | crate::app::ObservedExternalProviderTurnRole::Status => turns
                .iter()
                .rev()
                .find(|turn| turn.role == crate::app::ObservedExternalProviderTurnRole::User)?,
        }
    };
    if normalized_observed_prompt_text(&latest.text)
        .is_some_and(|text| arroba_owned_prompt_texts.contains(&text))
    {
        return None;
    }
    let provider_turn_id = latest
        .provider_turn_id
        .clone()
        .unwrap_or_else(|| latest.stable_fallback_id());
    Some(
        PromptQueueItem::new(
            format!(
                "external:{}:{}:{}",
                target.import.external_provider,
                target.import.external_provider_session_provider_id,
                provider_turn_id
            ),
            format!("external:{}", target.import.external_provider),
            target.agent_id.clone(),
            latest.text.clone(),
            PromptStatus::Running,
        )
        .with_prompt_origin(PromptOrigin::External),
    )
}

fn latest_effective_observed_turn_settles(
    provider: &str,
    turns: &[crate::app::ObservedExternalProviderTurn],
) -> bool {
    let Some(latest) = turns
        .iter()
        .rev()
        .find(|turn| !external_observed_turn_is_passive_telemetry(provider, turn))
        .or_else(|| turns.last())
    else {
        return false;
    };
    match latest.role {
        crate::app::ObservedExternalProviderTurnRole::Status => {
            external_status_turn_settles(&latest.text)
        }
        crate::app::ObservedExternalProviderTurnRole::Assistant
        | crate::app::ObservedExternalProviderTurnRole::User
        | crate::app::ObservedExternalProviderTurnRole::Reasoning
        | crate::app::ObservedExternalProviderTurnRole::Tool => false,
    }
}

fn external_status_turn_settles(text: &str) -> bool {
    text.starts_with("codex task_complete")
        || text.starts_with("claude message completed")
        || text.starts_with("opencode message completed")
}

fn external_status_turn_is_passive_telemetry(provider: &str, text: &str) -> bool {
    provider == "claude"
        && (text.starts_with("claude last-prompt") || text.starts_with("claude ai-title"))
}

fn external_observed_turn_is_passive_telemetry(
    provider: &str,
    turn: &crate::app::ObservedExternalProviderTurn,
) -> bool {
    turn.role == crate::app::ObservedExternalProviderTurnRole::Status
        && external_status_turn_is_passive_telemetry(provider, &turn.text)
}

fn external_provider_uses_explicit_completion(provider: &str) -> bool {
    matches!(provider, "codex" | "opencode")
}

fn latest_observed_user_turn_id(
    turns: &[crate::app::ObservedExternalProviderTurn],
) -> Option<String> {
    turns
        .iter()
        .rev()
        .find(|turn| turn.role == crate::app::ObservedExternalProviderTurnRole::User)
        .map(|turn| {
            turn.provider_turn_id
                .clone()
                .unwrap_or_else(|| turn.stable_fallback_id())
        })
}

fn emit_observed_external_history_signal(
    app: &DaemonApp,
    target: &ImportedExternalObserverTarget,
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
    app.terminal_stream_store().fan_out_output(
        &target.session_id,
        &provider_run_id,
        Some(&target.agent_id),
        crate::terminal::TerminalOutputKind::ProviderStatus,
        entry.merge_key.clone(),
        recipient_attachment_ids,
        b"external_provider_history_updated",
    );
}

fn emit_observed_external_state_signal(
    app: &DaemonApp,
    target: &ImportedExternalObserverTarget,
    provider_run_id: Option<&str>,
    latest_merge_key: Option<&str>,
    reason: &str,
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
    let provider = &target.import.external_provider;
    let provider_session_id = &target.import.external_provider_session_provider_id;
    let latest_merge_key = latest_merge_key.unwrap_or("none");
    app.terminal_stream_store().fan_out_output(
        &target.session_id,
        &provider_run_id,
        Some(&target.agent_id),
        crate::terminal::TerminalOutputKind::ProviderStatus,
        Some(format!(
            "external:{provider}:{provider_session_id}:state:{reason}:{latest_merge_key}"
        )),
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

fn mark_attached_external_provider_sessions(
    app: &DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
) {
    for attachment in attached_external_provider_session_refs(app, runtime_state) {
        store.mark_attached(
            &attachment.external_session_id,
            &attachment.session_id,
            &attachment.agent_id,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AttachedExternalProviderSessionRef {
    external_session_id: String,
    session_id: String,
    agent_id: String,
}

fn attached_external_provider_session_refs(
    app: &DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
) -> BTreeSet<AttachedExternalProviderSessionRef> {
    let mut attached = BTreeSet::new();
    for agent in app.agents().list_agents() {
        if app.sessions().get_session(agent.session_id()).is_err() {
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
        push_provider_run_attachment(&mut attached, &run);
    }
    if let Some(runtime_state) = runtime_state {
        for run in runtime_state.provider_runs_for_external_session_attachment() {
            push_provider_run_attachment(&mut attached, &run);
        }
    }
    attached
}

fn push_provider_run_attachment(
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

fn push_resume_state_attachments(
    attached: &mut BTreeSet<AttachedExternalProviderSessionRef>,
    resume_state: &ProviderResumeState,
    session_id: &str,
    agent_id: &str,
) {
    for (provider, provider_session_id) in [
        ("codex", resume_state.codex_thread_id()),
        ("opencode", resume_state.opencode_session_id()),
        ("claude", resume_state.claude_session_id()),
    ] {
        let Some(provider_session_id) = provider_session_id else {
            continue;
        };
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

fn external_provider_import_model(provider: &str, requested_model: Option<String>) -> String {
    requested_model.unwrap_or_else(|| match provider {
        "codex" => "default".to_string(),
        provider => default_external_provider_model(provider).to_string(),
    })
}

fn external_provider_import_session_alias(
    external: &ExternalProviderSessionRecord,
    requested_alias: Option<&str>,
) -> String {
    let alias_source = requested_alias
        .or(external.title.as_deref())
        .or(external.first_prompt_preview.as_deref())
        .unwrap_or(&external.provider_session_id);
    let mut base = session_alias_slug(alias_source).unwrap_or_else(|| {
        session_alias_slug(&format!(
            "{}-{}",
            external.provider, external.provider_session_id
        ))
        .unwrap_or_else(|| "unattached_agent".to_string())
    });
    let suffix = session_alias_slug(&external.provider_session_id)
        .map(|slug| short_alias_suffix(&slug))
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or_else(|| "imported".to_string());
    let reserved = suffix.len().saturating_add(1);
    if base.len().saturating_add(reserved) > EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN {
        base.truncate(EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN.saturating_sub(reserved));
        base = base.trim_matches(['-', '_']).to_string();
    }
    if base.is_empty() {
        suffix
    } else if base.ends_with(&format!("-{suffix}")) || base == suffix {
        base
    } else {
        format!("{base}-{suffix}")
    }
}

fn session_alias_slug(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_separator = false;
    for char in input.trim().chars().flat_map(|char| char.to_lowercase()) {
        if char.is_ascii_lowercase() || char.is_ascii_digit() {
            slug.push(char);
            previous_separator = false;
        } else if matches!(char, '-' | '_' | ' ' | '\t' | '\n' | '\r') {
            if !slug.is_empty() && !previous_separator {
                slug.push('-');
                previous_separator = true;
            }
        }
    }
    let slug = slug.trim_matches(['-', '_']).to_string();
    (!slug.is_empty()).then_some(slug)
}

fn short_alias_suffix(slug: &str) -> String {
    const SHORT_SUFFIX_LEN: usize = 12;
    if slug.len() <= SHORT_SUFFIX_LEN {
        slug.to_string()
    } else {
        slug[slug.len() - SHORT_SUFFIX_LEN..].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::local::{
        ExternalProviderSessionCapabilities, ImportExternalProviderAgentRequest,
        ImportExternalProviderSessionRequest,
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
                None,
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
            assert_eq!(session.alias(), Some("imported-external-one-external-1"));
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
                    .attached_to_arroba
            );
        });
    }

    #[test]
    fn import_codex_session_without_model_uses_persisted_thread_model() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let store = {
                let app = app.lock().await;
                app.external_provider_session_index_store()
            };
            store.upsert(record("codex", "thread-1", "/tmp/codex-thread"));

            let response = execute_external_provider_session_request(
                &app,
                None,
                LocalDaemonRequest::ImportExternalProviderSession(
                    ImportExternalProviderSessionRequest {
                        external_session_id: "codex:thread-1".to_string(),
                        alias: None,
                        provider: None,
                        model: None,
                        effort: None,
                        worktree_id: None,
                    },
                ),
                "external-import-user",
            )
            .await
            .expect("import should succeed");

            let LocalDaemonResponse::ExternalProviderSessionImported {
                agent,
                provider_run,
                ..
            } = response
            else {
                panic!("unexpected response")
            };
            let provider_run = provider_run.expect("provider run should launch");
            assert_eq!(agent.provider(), "codex");
            assert_eq!(agent.model(), Some("default"));
            assert_eq!(provider_run.model(), "default");
            assert_eq!(
                provider_run.resume_state().codex_thread_id(),
                Some("thread-1")
            );
        });
    }

    #[test]
    fn import_external_provider_session_rejects_already_attached_thread() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let store = {
                let app = app.lock().await;
                app.external_provider_session_index_store()
            };
            store.upsert(record(
                "dev-stub",
                "external-attached",
                "/tmp/external-attached",
            ));
            store.mark_attached(
                "dev-stub:external-attached",
                "session-existing",
                "agent-existing",
            );

            let error = execute_external_provider_session_request(
                &app,
                None,
                LocalDaemonRequest::ImportExternalProviderSession(
                    ImportExternalProviderSessionRequest {
                        external_session_id: "dev-stub:external-attached".to_string(),
                        alias: None,
                        provider: Some("dev-stub".to_string()),
                        model: Some("default".to_string()),
                        effort: None,
                        worktree_id: None,
                    },
                ),
                "external-import-user",
            )
            .await
            .expect_err("already attached external session should be rejected");

            assert!(error.to_string().contains(
                "already attached to Arroba session `session-existing` agent `agent-existing`"
            ));
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
                None,
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
    fn attached_arroba_agent_resume_state_removes_external_session_from_attachable_list() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        app.agents()
            .set_agent_runtime_profile(
                agent.id(),
                "codex",
                Some("gpt-test".to_string()),
                None,
                ProviderResumeState::from_codex_thread_id("thread-owned-by-arroba"),
            )
            .expect("agent runtime profile should update");
        let store = app.external_provider_session_index_store();
        store.upsert(record(
            "codex",
            "thread-owned-by-arroba",
            "/tmp/owned-by-arroba",
        ));

        mark_attached_external_provider_sessions(&app, None, &store);

        let page = store.list(&ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        });
        assert!(page.sessions.is_empty());
        let attached = store
            .get("codex:thread-owned-by-arroba")
            .expect("record should remain indexed");
        assert!(attached.attached_to_arroba);
        assert_eq!(attached.attached_session_ids, vec![session.id()]);
        assert_eq!(attached.attached_agent_ids, vec![agent.id()]);
    }

    #[test]
    fn live_provider_run_provider_session_id_counts_as_attached_to_arroba() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-test")
                .with_agent_id("agent-1");
        let launch = crate::provider::ProviderLaunchResult {
            process_label: "codex:test".to_string(),
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let mut run = RuntimeProviderRun::new("run-1", &request, launch);
        run.set_provider_session_id(Some("thread-live-run".to_string()));
        let mut attached = BTreeSet::new();

        push_provider_run_attachment(&mut attached, &run);

        assert!(attached.contains(&AttachedExternalProviderSessionRef {
            external_session_id: "codex:thread-live-run".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
        }));
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
        let outcome = append_observed_external_turns_for_import(
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

        assert_eq!(outcome.changed_count, 1);
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
    fn append_observed_external_user_turn_creates_external_active_prompt() {
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

        let outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: ImportedExternalObserverTarget {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    provider_run_id: None,
                    import,
                },
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::User,
                    text: "external prompt".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("observed user turn should append");

        assert_eq!(outcome.changed_count, 1);
        assert!(!outcome.external_active_prompt_settled);
        let active_prompt = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .expect("external user turn should mark active prompt");
        assert_eq!(active_prompt.prompt_origin(), PromptOrigin::External);
        assert_eq!(active_prompt.status(), PromptStatus::Running);
        assert_eq!(active_prompt.prompt(), "external prompt");
        assert_eq!(active_prompt.id(), "external:codex:thread-observed:user-1");
        let mirrored_session = app
            .sessions()
            .get_session(session.id())
            .expect("session mirror should load");
        assert_eq!(
            mirrored_session
                .active_prompt_for_agent(agent.id())
                .map(|prompt| prompt.prompt_origin()),
            Some(PromptOrigin::External)
        );
    }

    #[test]
    fn append_observed_external_assistant_turn_clears_external_active_prompt_after_stable_poll() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "claude:thread-observed".to_string(),
            "claude".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import: import.clone(),
        };
        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::User,
                    text: "external prompt".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("observed user turn should append");
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "external user turn should mark active before assistant output"
        );

        let first_assistant_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "external reply".to_string(),
                    observed_at_ms: Some(84),
                }],
            },
        )
        .expect("observed assistant turn should append");

        assert_eq!(first_assistant_outcome.changed_count, 1);
        assert!(!first_assistant_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "new assistant output should not settle the external active marker until it is stable"
        );

        let stable_assistant_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "external reply".to_string(),
                    observed_at_ms: Some(84),
                }],
            },
        )
        .expect("stable observed assistant turn should settle");

        assert_eq!(stable_assistant_outcome.changed_count, 0);
        assert!(stable_assistant_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "assistant output should settle the external active marker"
        );
        let mirrored_session = app
            .sessions()
            .get_session(session.id())
            .expect("session mirror should load");
        assert!(mirrored_session
            .active_prompt_for_agent(agent.id())
            .is_none());
    }

    #[test]
    fn append_observed_external_codex_assistant_waits_for_task_complete_to_settle() {
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
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "intermediate commentary".to_string(),
            observed_at_ms: Some(84),
        };
        let task_complete = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("task-complete-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
            observed_at_ms: Some(126),
        };

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant.clone()],
            },
        )
        .expect("observed Codex assistant turn should append");

        let stable_assistant_outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("stable Codex assistant turn should stay active");

        assert_eq!(stable_assistant_outcome.changed_count, 0);
        assert!(!stable_assistant_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "Codex assistant commentary can be followed by tools and must not settle the turn"
        );

        let complete_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, task_complete],
            },
        )
        .expect("Codex task_complete should settle");

        assert_eq!(complete_outcome.changed_count, 1);
        assert!(complete_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "Codex task_complete should clear the external active prompt"
        );
    }

    #[test]
    fn append_observed_external_assistant_turn_waits_for_settlement_permission() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "claude:thread-observed".to_string(),
            "claude".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "external reply".to_string(),
            observed_at_ms: Some(84),
        };

        let first_assistant_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::User,
                    text: "external prompt".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("observed user turn should append");
        assert_eq!(first_assistant_outcome.changed_count, 1);

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![assistant.clone()],
            },
        )
        .expect("observed assistant turn should append");

        let early_stable_outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![assistant.clone()],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: false,
            },
        )
        .expect("early stable assistant turn should not settle");

        assert_eq!(early_stable_outcome.changed_count, 0);
        assert!(!early_stable_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "stable assistant output should stay running until settlement is permitted"
        );

        let late_stable_outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![assistant],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("late stable assistant turn should settle");

        assert_eq!(late_stable_outcome.changed_count, 0);
        assert!(late_stable_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "external active prompt should clear once settlement is permitted"
        );
    }

    #[test]
    fn append_observed_external_claude_telemetry_after_assistant_still_settles() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "claude:thread-observed".to_string(),
            "claude".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "final external reply".to_string(),
            observed_at_ms: Some(84),
        };
        let telemetry = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("last-prompt-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "claude last-prompt {\"lastPrompt\":\"external prompt\"}".to_string(),
            observed_at_ms: Some(126),
        };

        let initial_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant.clone(), telemetry.clone()],
            },
        )
        .expect("observed Claude turn should append");
        assert_eq!(initial_outcome.changed_count, 3);
        assert_eq!(initial_outcome.active_relevant_changed_count, 2);

        let outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, assistant, telemetry],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("stable Claude telemetry should settle");

        assert_eq!(outcome.changed_count, 0);
        assert_eq!(outcome.active_relevant_changed_count, 0);
        assert!(outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "Claude passive telemetry after assistant output must not keep the external turn working"
        );
    }

    #[test]
    fn append_observed_external_claude_completion_after_tool_settles() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "claude:thread-observed".to_string(),
            "claude".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let tool = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("tool-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Tool,
            text: "TOOL_STEP_20: complete".to_string(),
            observed_at_ms: Some(84),
        };
        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "FINAL_EXTERNAL_PARITY_SUMMARY".to_string(),
            observed_at_ms: Some(126),
        };
        let completion = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1:completed".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "claude message completed\n{\"stop_reason\":\"end_turn\"}".to_string(),
            observed_at_ms: Some(126),
        };
        let telemetry = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("last-prompt-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "claude last-prompt {\"lastPrompt\":\"external prompt\"}".to_string(),
            observed_at_ms: Some(168),
        };

        let initial_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![
                    prompt.clone(),
                    tool.clone(),
                    assistant.clone(),
                    completion.clone(),
                    telemetry.clone(),
                ],
            },
        )
        .expect("observed Claude turn should append");
        assert_eq!(initial_outcome.changed_count, 5);
        assert_eq!(initial_outcome.active_relevant_changed_count, 4);
        assert!(!initial_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "completed Claude imports should not create a running external prompt"
        );

        let outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, tool, assistant, completion, telemetry],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("stable Claude completion should settle");

        assert_eq!(outcome.changed_count, 0);
        assert_eq!(outcome.active_relevant_changed_count, 0);
        assert!(!outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "completed Claude imports should remain idle on stable reread"
        );
    }

    #[test]
    fn append_observed_external_claude_completion_with_new_passive_telemetry_settles() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "claude:thread-observed".to_string(),
            "claude".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let tool = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("tool-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Tool,
            text: "TOOL_STEP_20: complete".to_string(),
            observed_at_ms: Some(84),
        };
        let running_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), tool.clone()],
            },
        )
        .expect("observed Claude tool should append");
        assert_eq!(running_outcome.changed_count, 2);
        assert!(!running_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "external tool output should keep the prompt running"
        );

        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "FINAL_EXTERNAL_PARITY_SUMMARY".to_string(),
            observed_at_ms: Some(126),
        };
        let completion = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1:completed".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "claude message completed\n{\"stop_reason\":\"end_turn\"}".to_string(),
            observed_at_ms: Some(126),
        };
        let telemetry = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("last-prompt-leaf-assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "claude last-prompt {\"leafUuid\":\"assistant-1\"}".to_string(),
            observed_at_ms: None,
        };

        let completed_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, tool, assistant, completion, telemetry],
            },
        )
        .expect("observed Claude completion should append and settle");

        assert_eq!(completed_outcome.changed_count, 3);
        assert_eq!(completed_outcome.active_relevant_changed_count, 2);
        assert!(completed_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "Claude completion followed by passive telemetry must clear WORKING in the same poll"
        );
    }

    #[test]
    fn append_observed_external_tool_turn_keeps_external_active_prompt_running() {
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
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let tool = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("tool-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Tool,
            text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
            observed_at_ms: Some(84),
        };

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), tool.clone()],
            },
        )
        .expect("observed prompt and tool should append");

        let stable_tool_outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, tool],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("stable tool should not settle");

        assert_eq!(stable_tool_outcome.changed_count, 0);
        assert!(!stable_tool_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "tool output alone should not settle the external turn"
        );
    }

    #[test]
    fn append_observed_external_codex_token_count_keeps_active_prompt_running() {
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
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let token_count = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("token-count-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
                .to_string(),
            observed_at_ms: Some(84),
        };

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed prompt should append");

        let token_count_outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, token_count],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("Codex token count should append without settling");

        assert_eq!(token_count_outcome.changed_count, 1);
        assert!(!token_count_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "Codex token_count is telemetry and must not settle the external turn"
        );
    }

    #[test]
    fn append_observed_external_opencode_completion_status_settles_active_prompt() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "opencode:thread-observed".to_string(),
            "opencode".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let status = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("message-status-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "opencode message completed\n{\"finish\":\"stop\"}".to_string(),
            observed_at_ms: Some(84),
        };

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed prompt should append");
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "OpenCode prompt should mark the external turn running"
        );

        let first_status_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), status.clone()],
            },
        )
        .expect("observed prompt and status should append");
        assert!(first_status_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "OpenCode completion metadata should settle the external turn immediately"
        );

        let stable_status_outcome = append_observed_external_turns_for_import_with_options(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, status],
            },
            ImportedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("stable completion status should stay settled");

        assert_eq!(stable_status_outcome.changed_count, 0);
        assert!(!stable_status_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "OpenCode completion metadata should keep the external turn settled"
        );
    }

    #[test]
    fn append_observed_external_turns_groups_codex_prompt_without_item_id_with_reply() {
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
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: None,
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt without provider item id".to_string(),
            observed_at_ms: Some(42),
        };
        let prompt_turn_id = prompt.stable_fallback_id();
        let reasoning = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("reasoning-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Reasoning,
            text: "external reasoning".to_string(),
            observed_at_ms: Some(63),
        };
        let tool = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("tool-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Tool,
            text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
            observed_at_ms: Some(72),
        };
        let reply_one = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("msg-reply-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "external reply one".to_string(),
            observed_at_ms: Some(84),
        };
        let reply_two = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("msg-reply-2".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "external reply two".to_string(),
            observed_at_ms: Some(126),
        };

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed user turn should append");

        let changed_reply_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![
                    prompt.clone(),
                    reasoning.clone(),
                    tool.clone(),
                    reply_one.clone(),
                    reply_two.clone(),
                ],
            },
        )
        .expect("observed assistant turn should append");

        assert_eq!(changed_reply_outcome.changed_count, 4);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 5);
        assert_eq!(
            entries[0].external_provider_turn_id.as_deref(),
            Some(prompt_turn_id.as_str())
        );
        assert_eq!(
            entries[1].external_provider_turn_id.as_deref(),
            Some(prompt_turn_id.as_str())
        );
        assert_eq!(
            entries[2].external_provider_turn_id.as_deref(),
            Some(prompt_turn_id.as_str())
        );
        assert_eq!(
            entries[3].external_provider_turn_id.as_deref(),
            Some(prompt_turn_id.as_str())
        );
        assert_eq!(
            entries[4].external_provider_turn_id.as_deref(),
            Some(prompt_turn_id.as_str())
        );
        assert_eq!(entries[1].kind, SessionHistoryEntryKind::ProviderReasoning);
        assert_eq!(entries[2].kind, SessionHistoryEntryKind::ProviderTool);
        let expected_prompt_merge_key = format!("external:codex:thread-observed:{prompt_turn_id}");
        assert_eq!(
            entries[0].merge_key.as_deref(),
            Some(expected_prompt_merge_key.as_str())
        );
        assert_eq!(
            entries[1].merge_key.as_deref(),
            Some("external:codex:thread-observed:reasoning-1")
        );
        assert_eq!(
            entries[2].merge_key.as_deref(),
            Some("external:codex:thread-observed:tool-1")
        );
        assert_eq!(
            entries[3].merge_key.as_deref(),
            Some("external:codex:thread-observed:msg-reply-1")
        );
        assert_eq!(
            entries[4].merge_key.as_deref(),
            Some("external:codex:thread-observed:msg-reply-2")
        );
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "changed external assistant output should keep the external prompt marked running"
        );

        let stable_reply_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![
                    prompt.clone(),
                    reasoning.clone(),
                    tool.clone(),
                    reply_one,
                    reply_two,
                ],
            },
        )
        .expect("stable observed assistant turn should stay active for Codex");

        assert_eq!(stable_reply_outcome.changed_count, 0);
        assert!(!stable_reply_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "Codex stable assistant output should stay active until task_complete"
        );

        let complete_outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![
                    prompt,
                    reasoning,
                    tool,
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("task-complete-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Status,
                        text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                        observed_at_ms: Some(168),
                    },
                ],
            },
        )
        .expect("Codex task_complete should settle");

        assert_eq!(complete_outcome.changed_count, 1);
        assert!(complete_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "Codex task_complete should clear the external active marker"
        );
    }

    #[test]
    fn append_observed_external_turns_skips_arroba_owned_prompt_echoes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        app.append_history_entry(
            session.id(),
            SessionHistoryEntry::user_prompt(
                session.id(),
                "attachment-1",
                agent.id(),
                "arroba owned prompt",
            ),
        );
        let import = ExternalProviderImportMetadata::observed_history(
            "codex:thread-observed".to_string(),
            "codex".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };

        let outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-owned".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "arroba owned prompt".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-owned".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "provider reply to arroba owned prompt".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("observed arroba-owned provider turn should be skipped");

        assert_eq!(outcome.changed_count, 0);
        assert!(!outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "arroba-owned prompt echoes should not create an external active prompt"
        );
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "arroba owned prompt");
        let persisted = app
            .agents()
            .get_agent(agent.id())
            .expect("agent should exist");
        let cursor = &persisted
            .external_provider_import()
            .expect("metadata should persist")
            .observed_cursor;
        assert_eq!(
            cursor.last_observed_turn_id.as_deref(),
            Some("assistant-owned")
        );
    }

    #[test]
    fn append_observed_external_turns_skips_active_arroba_prompt_echoes() {
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
        app.prompt_owner_activate_prompt(
            session.id(),
            PromptQueueItem::new(
                "arroba-active-prompt",
                "attachment-1",
                agent.id(),
                "arroba-owned active prompt",
                PromptStatus::Running,
            ),
        )
        .expect("active Arroba prompt should mirror");

        let outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: ImportedExternalObserverTarget {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    provider_run_id: None,
                    import,
                },
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-owned-active".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "arroba-owned active prompt".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("tool-owned-active".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Tool,
                        text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
                        observed_at_ms: Some(84),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-owned-active".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "provider reply to arroba-owned active prompt".to_string(),
                        observed_at_ms: Some(126),
                    },
                ],
            },
        )
        .expect("observed Arroba-owned active provider turn should be skipped");

        assert_eq!(outcome.changed_count, 0);
        assert!(!outcome.external_active_prompt_settled);
        assert!(app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load")
            .is_empty());
        let active_prompt = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .expect("Arroba active prompt should remain active");
        assert_eq!(active_prompt.prompt_origin(), PromptOrigin::Arroba);
        assert_eq!(active_prompt.prompt(), "arroba-owned active prompt");
    }

    #[test]
    fn append_observed_external_turns_skips_changed_duplicate_merge_key() {
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
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import: import.clone(),
        };
        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "partial external reply".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("initial observed assistant turn should append");

        let outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "complete external reply".to_string(),
                    observed_at_ms: Some(84),
                }],
            },
        )
        .expect("changed observed assistant duplicate should be skipped");

        assert_eq!(outcome.changed_count, 0);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "partial external reply");
        assert_eq!(entries[0].observed_at_ms, Some(42));
        let legacy_entries = app
            .history_store()
            .load(&session)
            .expect("legacy history should load");
        assert_eq!(legacy_entries.len(), 1);
        assert_eq!(legacy_entries[0].text, "partial external reply");
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
        let outcome = append_observed_external_turns_for_import(
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

        assert_eq!(outcome.changed_count, 1);
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
    fn append_observed_external_turns_signals_history_refresh_without_provider_run() {
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
        let import = ExternalProviderImportMetadata::observed_history(
            "codex:thread-observed".to_string(),
            "codex".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let outcome = append_observed_external_turns_for_import(
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

        assert_eq!(outcome.changed_count, 1);
        let records = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].provider_run_id,
            format!("external-observer:{}", agent.id())
        );
        assert_eq!(records[0].agent_id.as_deref(), Some(agent.id()));
        assert_eq!(
            records[0].kind,
            crate::terminal::TerminalOutputKind::ProviderStatus
        );
        assert_eq!(records[0].bytes, b"external_provider_history_updated");
    }

    #[test]
    fn append_observed_external_completion_signals_settled_state_refresh() {
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
        let import = ExternalProviderImportMetadata::observed_history(
            "codex:thread-observed".to_string(),
            "codex".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = ImportedExternalObserverTarget {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_run_id: None,
            import,
        };
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let completion = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("task-complete-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
            observed_at_ms: Some(84),
        };

        append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("prompt should append");
        let _ = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());

        let outcome = append_observed_external_turns_for_import(
            &mut app,
            ImportedExternalObserverRead {
                target,
                turns: vec![prompt, completion],
            },
        )
        .expect("completion should append and settle");

        assert!(outcome.external_active_prompt_settled);
        let records = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(
            records.len(),
            2,
            "completion must signal both the new history row and the settled state projection"
        );
        assert_eq!(records[0].bytes, b"external_provider_history_updated");
        assert_eq!(records[1].bytes, b"external_provider_history_updated");
        assert_eq!(
            records[1].merge_key.as_deref(),
            Some(
                "external:codex:thread-observed:state:active_prompt_settled:external:codex:thread-observed:task-complete-1"
            )
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

    #[test]
    fn external_provider_import_session_alias_slugifies_prompt_titles() {
        let record = ExternalProviderSessionRecord {
            title: Some("There is supposed to be a service in the kernel".to_string()),
            provider_session_id: "019eea18-f755-7680-ab3f-31be1f79d4d0".to_string(),
            ..record(
                "codex",
                "019eea18-f755-7680-ab3f-31be1f79d4d0",
                "/tmp/external-one",
            )
        };

        assert_eq!(
            external_provider_import_session_alias(&record, None),
            "there-is-supposed-to-be-a-service-in-the-kernel-31be1f79d4d0"
        );
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
            capabilities: ExternalProviderSessionCapabilities {
                can_read_history: true,
                ..ExternalProviderSessionCapabilities::default()
            },
            attached_to_arroba: false,
            attached_session_ids: Vec::new(),
            attached_agent_ids: Vec::new(),
        }
    }
}
