use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{watch, Mutex};

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::{
    external_session_id_for_provider_session, normalized_observed_prompt_text,
    AttachedProviderTranscriptCursorKey, DaemonApp, ExternalProviderObservationPolicy,
};
use crate::error::DaemonError;
use crate::history::{
    ExternalImportHistoryEntry, SessionHistoryEntry,
    EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON, EXTERNAL_PROVIDER_ACTIVE_PROMPT_STARTED_REASON,
    EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
};
use crate::local::{
    ExternalProviderSessionRecord, ImportExternalProviderAgentRequest,
    ImportExternalProviderSessionRequest, ListExternalProviderSessionsRequest, LocalDaemonRequest,
    LocalDaemonResponse,
};
use crate::provider::{
    external_provider_import_model, external_provider_session_providers,
    ExternalProviderImportMetadata, ExternalProviderObservedCursor, LaunchProviderRequest,
    ProviderResumeState, RuntimeProviderRun,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{CreateSessionRequest, PromptQueueItem, RuntimeSession, SessionAgentDefaults};
#[cfg(test)]
use crate::session::{PromptOrigin, PromptStatus};

const EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
const EXTERNAL_PROVIDER_ATTACHED_ACTIVE_INTERVAL: Duration = Duration::from_secs(1);
const EXTERNAL_PROVIDER_ATTACHED_IDLE_INTERVAL: Duration = Duration::from_secs(20);
const EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW: Duration = Duration::from_secs(120);
const EXTERNAL_PROVIDER_ATTACHED_SETTLE_GRACE: Duration = Duration::from_secs(4);
const EXTERNAL_PROVIDER_ATTACHED_MAX_POLLS_PER_TICK: usize = 16;
const EXTERNAL_PROVIDER_ATTACHED_SLOW_TICK: Duration = Duration::from_millis(250);
const EXTERNAL_PROVIDER_DISCOVERY_SLOW_SIGNATURE: Duration = Duration::from_millis(250);
const EXTERNAL_PROVIDER_DISCOVERY_SLOW_REFRESH: Duration = Duration::from_millis(500);
const EXTERNAL_PROVIDER_DISCOVERY_FULL_SCAN_AFTER_CACHED_CHECKS: u32 = 10;
const EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN: usize = 64;

#[derive(Debug, Default)]
struct ExternalProviderSessionDiscoveryCache {
    signature: Option<crate::app::ExternalProviderSessionDiscoverySignature>,
    candidate_paths: Option<Vec<(String, PathBuf)>>,
    cached_signature_checks: u32,
}

#[derive(Debug)]
struct ExternalProviderSessionDiscoverySignatureRead {
    signature: crate::app::ExternalProviderSessionDiscoverySignature,
    candidate_paths: Vec<(String, PathBuf)>,
    full_scan: bool,
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
struct AttachedExternalObserverTarget {
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
    external_session_id: String,
    provider: String,
    provider_session_id: String,
    observed_cursor: ExternalProviderObservedCursor,
    cursor_source: AttachedExternalObserverCursorSource,
}

#[derive(Debug, Clone)]
enum AttachedExternalObserverCursorSource {
    Imported(ExternalProviderImportMetadata),
    ArrobaOwned(AttachedProviderTranscriptCursorKey),
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverRead {
    target: AttachedExternalObserverTarget,
    turns: Vec<crate::app::ObservedExternalProviderTurn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttachedExternalObserverAppendOptions {
    allow_external_active_prompt_settlement: bool,
}

impl Default for AttachedExternalObserverAppendOptions {
    fn default() -> Self {
        Self {
            allow_external_active_prompt_settlement: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AttachedExternalObserverAppendOutcome {
    changed_count: usize,
    active_relevant_changed_count: usize,
    external_active_prompt_settled: bool,
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverSchedule {
    next_due_at: tokio::time::Instant,
    active_until: Option<tokio::time::Instant>,
    last_changed_at: Option<tokio::time::Instant>,
    consecutive_errors: u32,
}

impl AttachedExternalObserverSchedule {
    fn due_now(now: tokio::time::Instant) -> Self {
        Self {
            next_due_at: now,
            active_until: Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW),
            last_changed_at: None,
            consecutive_errors: 0,
        }
    }
}

pub(crate) async fn run_attached_provider_transcript_observer(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: crate::runtime::state::KernelRuntimeState,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut schedule: BTreeMap<String, AttachedExternalObserverSchedule> = BTreeMap::new();
    let mut interval = tokio::time::interval(EXTERNAL_PROVIDER_ATTACHED_ACTIVE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = interval.tick() => {
                poll_attached_external_provider_transcripts(&app, &runtime_state, &mut schedule).await;
            }
        }
    }
}

async fn poll_attached_external_provider_transcripts(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: &crate::runtime::state::KernelRuntimeState,
    schedule: &mut BTreeMap<String, AttachedExternalObserverSchedule>,
) {
    let tick_started = Instant::now();
    let now = tokio::time::Instant::now();
    let targets = {
        let app = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        attached_external_observer_targets(&app)
    };
    let target_count = targets.len();
    let target_keys = targets
        .iter()
        .map(attached_observer_target_key)
        .collect::<BTreeSet<_>>();
    schedule.retain(|key, _| target_keys.contains(key));
    let due = due_attached_external_observer_targets(
        targets,
        schedule,
        now,
        EXTERNAL_PROVIDER_ATTACHED_MAX_POLLS_PER_TICK,
    );
    if due.is_empty() {
        return;
    }
    let due_count = due.len();
    let limited = due_count >= EXTERNAL_PROVIDER_ATTACHED_MAX_POLLS_PER_TICK;
    let mut read_ms_total = 0u128;
    let mut append_ms_total = 0u128;
    let mut changed_count = 0usize;
    let mut active_relevant_changed_count = 0usize;
    let mut error_count = 0usize;
    for target in due {
        let key = attached_observer_target_key(&target);
        let allow_external_active_prompt_settlement = schedule
            .get(&key)
            .and_then(|state| state.last_changed_at)
            .is_some_and(|last_changed_at| {
                now.duration_since(last_changed_at) >= EXTERNAL_PROVIDER_ATTACHED_SETTLE_GRACE
            });
        let provider = target.provider.clone();
        let provider_session_id = target.provider_session_id.clone();
        let read_started = Instant::now();
        let read = match tokio::task::spawn_blocking(move || {
            crate::app::read_external_provider_observed_turns(&provider, &provider_session_id)
        })
        .await
        {
            Ok(turns) => Ok(AttachedExternalObserverRead { target, turns }),
            Err(error) => Err(error.to_string()),
        };
        read_ms_total += read_started.elapsed().as_millis();
        match read {
            Ok(read) => {
                let append_started = Instant::now();
                let outcome = {
                    let mut app = crate::runtime::app_lock::lock_app_instrumented(
                        &app,
                        "external_provider_session_control",
                    )
                    .await;
                    append_observed_external_turns_for_attached_target_with_options(
                        &mut app,
                        read,
                        AttachedExternalObserverAppendOptions {
                            allow_external_active_prompt_settlement,
                        },
                    )
                    .unwrap_or_default()
                };
                append_ms_total += append_started.elapsed().as_millis();
                changed_count += outcome.changed_count;
                active_relevant_changed_count += outcome.active_relevant_changed_count;
                dispatch_next_queued_prompt_after_external_settlement(
                    Some(runtime_state),
                    &outcome,
                    "failed to dispatch queued prompt after external provider turn settled",
                )
                .await;
                let state = schedule
                    .entry(key)
                    .or_insert_with(|| AttachedExternalObserverSchedule::due_now(now));
                state.consecutive_errors = 0;
                if outcome.active_relevant_changed_count > 0 {
                    state.active_until = Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW);
                    state.last_changed_at = Some(now);
                }
                let active = state
                    .active_until
                    .is_some_and(|active_until| active_until > now);
                state.next_due_at = now
                    + if active {
                        EXTERNAL_PROVIDER_ATTACHED_ACTIVE_INTERVAL
                    } else {
                        EXTERNAL_PROVIDER_ATTACHED_IDLE_INTERVAL
                    };
            }
            Err(error) => {
                error_count += 1;
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
                    .or_insert_with(|| AttachedExternalObserverSchedule::due_now(now));
                state.consecutive_errors = state.consecutive_errors.saturating_add(1);
                let backoff_secs = 2_u64.pow(state.consecutive_errors.min(5));
                state.next_due_at = now + Duration::from_secs(backoff_secs);
            }
        }
    }
    let total_elapsed = tick_started.elapsed();
    if total_elapsed >= EXTERNAL_PROVIDER_ATTACHED_SLOW_TICK
        || changed_count > 0
        || error_count > 0
        || limited
    {
        crate::logging::info_with_fields(
            "daemon.external_provider_sessions",
            "attached provider transcript observer tick",
            serde_json::json!({
                "target_count": target_count,
                "due_count": due_count,
                "max_polls": EXTERNAL_PROVIDER_ATTACHED_MAX_POLLS_PER_TICK,
                "limited": limited,
                "read_ms": read_ms_total,
                "append_ms": append_ms_total,
                "total_ms": total_elapsed.as_millis(),
                "changed_count": changed_count,
                "active_relevant_changed_count": active_relevant_changed_count,
                "error_count": error_count,
            }),
        );
    }
}

fn due_attached_external_observer_targets(
    targets: Vec<AttachedExternalObserverTarget>,
    schedule: &mut BTreeMap<String, AttachedExternalObserverSchedule>,
    now: tokio::time::Instant,
    max_polls: usize,
) -> Vec<AttachedExternalObserverTarget> {
    let mut due = targets
        .into_iter()
        .filter_map(|target| {
            let key = attached_observer_target_key(&target);
            let state = schedule
                .entry(key.clone())
                .or_insert_with(|| AttachedExternalObserverSchedule::due_now(now));
            (state.next_due_at <= now).then_some((state.next_due_at, key, target))
        })
        .collect::<Vec<_>>();
    due.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    due.into_iter()
        .take(max_polls)
        .map(|(_, _, target)| target)
        .collect()
}

async fn refresh_external_provider_session_index(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    mut cache: Option<&mut ExternalProviderSessionDiscoveryCache>,
    force: bool,
) {
    let refresh_started = Instant::now();
    let signature_started = Instant::now();
    let cached_candidate_paths = (!force)
        .then(|| {
            cache.as_ref().and_then(|cache| {
                (cache.cached_signature_checks
                    < EXTERNAL_PROVIDER_DISCOVERY_FULL_SCAN_AFTER_CACHED_CHECKS)
                    .then(|| cache.candidate_paths.clone())
                    .flatten()
            })
        })
        .flatten();
    let mut signature_read =
        match read_external_provider_discovery_signature(cached_candidate_paths).await {
            Some(signature) => signature,
            None => return,
        };
    let mut signature_ms = signature_started.elapsed().as_millis();
    if !signature_read.full_scan
        && cache
            .as_ref()
            .and_then(|cache| cache.signature.as_ref())
            .is_some_and(|cached| cached != &signature_read.signature)
    {
        let full_signature_started = Instant::now();
        signature_read = match read_external_provider_discovery_signature(None).await {
            Some(signature) => signature,
            None => return,
        };
        signature_ms += full_signature_started.elapsed().as_millis();
    }
    let signature = signature_read.signature.clone();
    if !force && cache.as_ref().and_then(|cache| cache.signature.as_ref()) == Some(&signature) {
        if let Some(cache) = cache.as_mut() {
            cache.candidate_paths = Some(signature_read.candidate_paths);
            cache.cached_signature_checks = if signature_read.full_scan {
                0
            } else {
                cache.cached_signature_checks.saturating_add(1)
            };
        }
        let total_elapsed = refresh_started.elapsed();
        if total_elapsed >= EXTERNAL_PROVIDER_DISCOVERY_SLOW_SIGNATURE {
            crate::logging::info_with_fields(
                "daemon.external_provider_sessions",
                "external provider session discovery unchanged",
                serde_json::json!({
                    "signature_ms": signature_ms,
                    "total_ms": total_elapsed.as_millis(),
                    "full_scan": signature_read.full_scan,
                    "cached_signature_checks": cache
                        .as_ref()
                        .map(|cache| cache.cached_signature_checks)
                        .unwrap_or(0),
                }),
            );
        }
        return;
    }
    let discovery_started = Instant::now();
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
    let discovery_ms = discovery_started.elapsed().as_millis();
    let codex_count = count_external_provider_sessions(&discovered, "codex");
    let claude_count = count_external_provider_sessions(&discovered, "claude");
    let opencode_count = count_external_provider_sessions(&discovered, "opencode");
    if let Some(cache) = cache.as_mut() {
        cache.signature = Some(signature);
        cache.candidate_paths = Some(signature_read.candidate_paths);
        cache.cached_signature_checks = 0;
    }
    let store = {
        let app = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        app.external_provider_session_index_store()
    };
    for provider in external_provider_session_providers() {
        let provider_sessions = discovered
            .iter()
            .filter(|session| session.provider == *provider)
            .cloned()
            .collect::<Vec<_>>();
        store.replace_provider_sessions(provider, provider_sessions);
    }
    // Read the attached-session refs under the app lock, then apply them to
    // the index store (which has its own lock) without holding the app lock,
    // so this background poller does not block foreground commands for the
    // duration of the store writes.
    let attached_refs = {
        let app = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        attached_external_provider_session_refs(&app, runtime_state)
    };
    for attachment in attached_refs {
        store.mark_attached(
            &attachment.external_session_id,
            &attachment.session_id,
            &attachment.agent_id,
        );
    }
    let total_elapsed = refresh_started.elapsed();
    if force || total_elapsed >= EXTERNAL_PROVIDER_DISCOVERY_SLOW_REFRESH || !discovered.is_empty()
    {
        crate::logging::info_with_fields(
            "daemon.external_provider_sessions",
            "external provider session discovery refreshed",
            serde_json::json!({
                "force": force,
                "signature_ms": signature_ms,
                "discovery_ms": discovery_ms,
                "total_ms": total_elapsed.as_millis(),
                "signature_full_scan": signature_read.full_scan,
                "session_count": discovered.len(),
                "codex_count": codex_count,
                "claude_count": claude_count,
                "opencode_count": opencode_count,
            }),
        );
    }
}

async fn read_external_provider_discovery_signature(
    cached_candidate_paths: Option<Vec<(String, PathBuf)>>,
) -> Option<ExternalProviderSessionDiscoverySignatureRead> {
    let full_scan = cached_candidate_paths.is_none();
    match tokio::task::spawn_blocking(move || {
        let candidate_paths = cached_candidate_paths.unwrap_or_else(|| {
            crate::app::external_provider_session_discovery_candidate_paths(None)
        });
        let signature = crate::app::external_provider_session_discovery_signature_for_candidates(
            &candidate_paths,
        );
        ExternalProviderSessionDiscoverySignatureRead {
            signature,
            candidate_paths,
            full_scan,
        }
    })
    .await
    {
        Ok(signature) => Some(signature),
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.external_provider_sessions",
                "external provider session signature task failed",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            None
        }
    }
}

fn count_external_provider_sessions(
    sessions: &[ExternalProviderSessionRecord],
    provider: &str,
) -> usize {
    sessions
        .iter()
        .filter(|session| session.provider == provider)
        .count()
}

pub(crate) async fn execute_external_provider_session_request(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    request: LocalDaemonRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let store = {
        let app = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        app.external_provider_session_index_store()
    };
    match request {
        LocalDaemonRequest::ListExternalProviderSessions(request) => {
            {
                let app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
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
                for provider in external_provider_session_providers() {
                    let provider_sessions = discovered
                        .iter()
                        .filter(|session| session.provider == *provider)
                        .cloned()
                        .collect::<Vec<_>>();
                    store.replace_provider_sessions(provider, provider_sessions);
                }
            }
            {
                let app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
                mark_attached_external_provider_sessions(&app, runtime_state, &store);
            }
            refresh_attached_external_provider_histories(
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
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            mark_attached_external_provider_sessions(&app, runtime_state, &store);
            import_external_provider_session(
                &mut app,
                runtime_state,
                &store,
                request,
                caller_user_id,
            )
        }
        LocalDaemonRequest::ImportExternalProviderAgent(request) => {
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            mark_attached_external_provider_sessions(&app, runtime_state, &store);
            import_external_provider_agent(&mut app, runtime_state, &store, request, caller_user_id)
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "external provider session request",
            message: "unsupported external provider session request".to_string(),
        }),
    }
}

async fn refresh_attached_external_provider_histories(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    provider_filter: Option<&str>,
) {
    let targets = {
        let app = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        attached_external_observer_targets(&app)
            .into_iter()
            .filter(|target| {
                provider_filter
                    .map(|provider| target.provider == provider)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>()
    };
    for target in targets {
        let provider = target.provider.clone();
        let provider_session_id = target.provider_session_id.clone();
        let read = match tokio::task::spawn_blocking(move || {
            crate::app::read_external_provider_observed_turns(&provider, &provider_session_id)
        })
        .await
        {
            Ok(turns) => AttachedExternalObserverRead { target, turns },
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
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            append_observed_external_turns_for_attached_target(&mut app, read).unwrap_or_default()
        };
        dispatch_next_queued_prompt_after_external_settlement(
            runtime_state,
            &outcome,
            "external provider session refresh failed to dispatch queued prompt after external settlement",
        )
        .await;
    }
}

async fn dispatch_next_queued_prompt_after_external_settlement(
    runtime_state: Option<&KernelRuntimeState>,
    outcome: &AttachedExternalObserverAppendOutcome,
    warning_message: &'static str,
) {
    let Some(runtime_state) = runtime_state else {
        return;
    };
    if !outcome.external_active_prompt_settled {
        return;
    }
    let Some(provider_run_id) = outcome.provider_run_id.as_deref() else {
        return;
    };
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
            warning_message,
            serde_json::json!({
                "session_id": outcome.session_id,
                "agent_id": outcome.agent_id,
                "provider_run_id": provider_run_id,
                "error": error.to_string(),
            }),
        );
    }
}

fn import_external_provider_session(
    app: &mut DaemonApp,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderSessionRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external =
        external_session_or_refresh(app, runtime_state, store, &request.external_session_id)?;
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
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderAgentRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let external =
        external_session_or_refresh(app, runtime_state, store, &request.external_session_id)?;
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
    for session in crate::app::discover_external_provider_sessions(provider) {
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

fn ensure_external_session_is_attachable(
    external: &ExternalProviderSessionRecord,
) -> Result<(), DaemonError> {
    if external.is_attachable_to_arroba() {
        return Ok(());
    }
    let session_label = external.first_attached_session_id().unwrap_or("unknown");
    let agent_label = external.first_attached_agent_id().unwrap_or("unknown");
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
            .with_resume_state(ProviderResumeState::from_external_provider_session(
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
    };
    let _ = append_observed_external_turns_for_attached_target(
        app,
        AttachedExternalObserverRead { target, turns },
    );
}

fn append_observed_external_turns_for_attached_target(
    app: &mut DaemonApp,
    read: AttachedExternalObserverRead,
) -> Result<AttachedExternalObserverAppendOutcome, DaemonError> {
    append_observed_external_turns_for_attached_target_with_options(
        app,
        read,
        AttachedExternalObserverAppendOptions::default(),
    )
}

fn append_observed_external_turns_for_attached_target_with_options(
    app: &mut DaemonApp,
    read: AttachedExternalObserverRead,
    options: AttachedExternalObserverAppendOptions,
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
    let session = app.sessions().get_session(&read.target.session_id)?;
    let agent = app.agents.get_agent(&read.target.agent_id)?;
    let queued_prompt_waiting = session
        .prompt_states()
        .get(agent.id())
        .is_some_and(|state| !state.queued_prompts().is_empty());
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
    let mut arroba_owned_prompt_text_counts = history_index
        .arroba_owned_prompts
        .iter()
        .filter_map(|text| normalized_observed_prompt_text(text))
        .fold(BTreeMap::<String, usize>::new(), |mut counts, text| {
            *counts.entry(text).or_default() += 1;
            counts
        });
    if let Some(prompt_state) = session.prompt_states().get(agent.id()) {
        if let Some(active_prompt) = prompt_state.active_prompt() {
            if active_prompt.is_arroba_owned() {
                if let Some(text) = normalized_observed_prompt_text(active_prompt.prompt()) {
                    *arroba_owned_prompt_text_counts.entry(text).or_default() += 1;
                }
            }
        }
        for queued_prompt in prompt_state.queued_prompts() {
            if queued_prompt.is_arroba_owned() {
                if let Some(text) = normalized_observed_prompt_text(queued_prompt.prompt()) {
                    *arroba_owned_prompt_text_counts.entry(text).or_default() += 1;
                }
            }
        }
    }
    let mut appended = 0usize;
    let mut active_relevant_appended = 0usize;
    let mut last_cursor = read.target.observed_cursor.clone();
    let mut visible_provider_turn_id = None;
    let mut current_observed_turn_is_arroba_owned = false;
    let mut arroba_owned_provider_turn_ids = BTreeSet::new();
    let candidate_turns =
        latest_observed_external_turns_by_merge_key(&read.turns, &provider, &provider_session_id);
    for turn in &candidate_turns {
        let kind = turn.role.session_history_kind();
        let merge_turn_id = turn.provider_turn_id_or_fallback();
        if turn.role == crate::app::ObservedExternalProviderTurnRole::User {
            visible_provider_turn_id = Some(merge_turn_id.clone());
        }
        let provider_turn_id = visible_provider_turn_id
            .clone()
            .unwrap_or_else(|| merge_turn_id.clone());
        let merge_key = turn.external_merge_key(&provider, &provider_session_id);
        if turn.role == crate::app::ObservedExternalProviderTurnRole::User {
            current_observed_turn_is_arroba_owned = consume_arroba_owned_prompt_text_match(
                &mut arroba_owned_prompt_text_counts,
                &turn.text,
            );
            if current_observed_turn_is_arroba_owned {
                arroba_owned_provider_turn_ids.insert(merge_turn_id.clone());
            }
        }
        if current_observed_turn_is_arroba_owned {
            last_cursor.last_observed_merge_key = Some(merge_key);
            last_cursor.last_observed_turn_id = Some(merge_turn_id);
            last_cursor.last_observed_at_ms =
                turn.observed_at_ms.or(last_cursor.last_observed_at_ms);
            continue;
        }
        let mut entry = SessionHistoryEntry::external_provider_observed_with_merge_key(
            &read.target.session_id,
            provider_run_id.as_deref(),
            &read.target.agent_id,
            kind,
            turn.text.clone(),
            &provider,
            &provider_session_id,
            Some(merge_key.clone()),
            Some(provider_turn_id.clone()),
            turn.observed_at_ms,
        );
        entry.external_observation =
            ExternalProviderObservationPolicy::for_provider(&provider).observation_for_turn(turn);
        let has_observable_change = {
            existing_entries_by_merge_key
                .get(&merge_key)
                .is_none_or(|existing| !external_observed_history_entry_matches(existing, &entry))
        };
        if has_observable_change {
            app.replace_history_entry_by_merge_key_or_append(
                &read.target.session_id,
                &merge_key,
                entry.clone(),
            );
            existing_entries_by_merge_key.insert(
                merge_key.clone(),
                ExternalImportHistoryEntry {
                    kind: entry.kind,
                    text: entry.text.clone(),
                    external_observation: entry.external_observation.clone(),
                },
            );
            emit_observed_external_history_signal(
                app,
                &read.target,
                provider_run_id.as_deref(),
                &entry,
            );
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
        last_cursor.last_observed_merge_key = Some(merge_key);
        last_cursor.last_observed_turn_id = Some(merge_turn_id);
        last_cursor.last_observed_at_ms = turn.observed_at_ms.or(last_cursor.last_observed_at_ms);
    }
    let changed = appended;
    outcome.changed_count = changed;
    outcome.active_relevant_changed_count = active_relevant_appended;
    let observation_policy = ExternalProviderObservationPolicy::for_provider(&provider);
    let active_prompt_sync = observation_policy.active_prompt_sync(
        &candidate_turns,
        changed,
        active_relevant_appended,
        options.allow_external_active_prompt_settlement,
        &arroba_owned_provider_turn_ids,
    );
    let latest_active_prompt = active_prompt_sync
        .active_prompt_turn
        .map(|turn| external_active_prompt_from_turn(&read.target, turn));
    let active_prompt_changed = if active_prompt_sync.should_sync_active_prompt {
        app.prompt_owner_sync_external_active_prompt(
            &read.target.session_id,
            &read.target.agent_id,
            latest_active_prompt.clone(),
        )?
    } else {
        false
    };
    outcome.external_active_prompt_settled = active_prompt_sync.should_sync_active_prompt
        && latest_active_prompt.is_none()
        && (active_prompt_changed || queued_prompt_waiting);
    let cursor_changed = last_cursor != read.target.observed_cursor;
    let state_signal_merge_key = last_cursor.last_observed_merge_key.clone();
    let state_signal_observed_at_ms = last_cursor.last_observed_at_ms;
    let persisted_hidden_settlement_signal =
        outcome.external_active_prompt_settled && !active_prompt_sync.latest_observation_settles;
    if persisted_hidden_settlement_signal {
        persist_observed_external_settlement_history_signal(
            app,
            &read.target,
            provider_run_id.as_deref(),
            state_signal_merge_key.as_deref(),
            visible_provider_turn_id.as_deref(),
            last_cursor.last_observed_at_ms,
        );
    }
    if changed > 0 || cursor_changed {
        persist_attached_external_observer_cursor(app, &read.target, last_cursor)?;
        let _ = crate::app::KernelSessionReadService::new(app)
            .session_snapshot(&read.target.session_id);
    } else if active_prompt_changed {
        let _ = crate::app::KernelSessionReadService::new(app)
            .session_snapshot(&read.target.session_id);
    }
    if active_prompt_changed || persisted_hidden_settlement_signal {
        emit_observed_external_state_signal(
            app,
            &read.target,
            provider_run_id.as_deref(),
            state_signal_merge_key.as_deref(),
            visible_provider_turn_id.as_deref(),
            state_signal_observed_at_ms,
            if latest_active_prompt.is_some() {
                EXTERNAL_PROVIDER_ACTIVE_PROMPT_STARTED_REASON
            } else {
                EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON
            },
        );
    }
    Ok(outcome)
}

fn update_provider_run_usage_from_external_observation(
    app: &mut DaemonApp,
    provider_run_id: Option<&str>,
    provider: &str,
    turn: &crate::app::ObservedExternalProviderTurn,
) {
    if turn.role != crate::app::ObservedExternalProviderTurnRole::Status {
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

fn persist_observed_external_settlement_history_signal(
    app: &DaemonApp,
    target: &AttachedExternalObserverTarget,
    provider_run_id: Option<&str>,
    latest_merge_key: Option<&str>,
    provider_turn_id: Option<&str>,
    observed_at_ms: Option<u64>,
) {
    let Some(latest_merge_key) = latest_merge_key else {
        return;
    };
    let Some(provider_turn_id) = provider_turn_id else {
        return;
    };
    let entry = SessionHistoryEntry::external_provider_observed_state_signal(
        &target.session_id,
        provider_run_id,
        &target.agent_id,
        &target.provider,
        &target.provider_session_id,
        EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
        latest_merge_key,
        provider_turn_id.to_string(),
        observed_at_ms.or_else(|| Some(crate::session::unix_epoch_ms())),
    );
    let Some(merge_key) = entry.merge_key.clone() else {
        return;
    };
    app.replace_history_entry_by_merge_key_or_append(&target.session_id, &merge_key, entry);
}

fn latest_observed_external_turns_by_merge_key(
    turns: &[crate::app::ObservedExternalProviderTurn],
    provider: &str,
    provider_session_id: &str,
) -> Vec<crate::app::ObservedExternalProviderTurn> {
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

fn external_observed_history_entry_matches(
    existing: &ExternalImportHistoryEntry,
    next: &SessionHistoryEntry,
) -> bool {
    existing.kind == next.kind
        && existing.text == next.text
        && existing.external_observation == next.external_observation
}

fn consume_arroba_owned_prompt_text_match(
    counts: &mut BTreeMap<String, usize>,
    observed_text: &str,
) -> bool {
    let Some(text) = normalized_observed_prompt_text(observed_text) else {
        return false;
    };
    let Some(count) = counts.get_mut(&text) else {
        return false;
    };
    if *count == 0 {
        return false;
    }
    *count -= 1;
    if *count == 0 {
        counts.remove(&text);
    }
    true
}

fn external_active_prompt_from_turn(
    target: &AttachedExternalObserverTarget,
    latest: &crate::app::ObservedExternalProviderTurn,
) -> PromptQueueItem {
    let provider_turn_id = latest.provider_turn_id_or_fallback();
    let external_prompt_id = crate::history::external_provider_observed_merge_key(
        &target.provider,
        &target.provider_session_id,
        &provider_turn_id,
    );
    PromptQueueItem::external_observed_running(
        external_prompt_id,
        &target.provider,
        target.agent_id.clone(),
        latest.text.clone(),
    )
}

fn emit_observed_external_history_signal(
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
        );
}

fn emit_observed_external_state_signal(
    app: &DaemonApp,
    target: &AttachedExternalObserverTarget,
    provider_run_id: Option<&str>,
    latest_merge_key: Option<&str>,
    provider_turn_id: Option<&str>,
    observed_at_ms: Option<u64>,
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
    let latest_merge_key = latest_merge_key.unwrap_or("none");
    let state_entry = SessionHistoryEntry::external_provider_observed_state_signal(
        &target.session_id,
        Some(&provider_run_id),
        &target.agent_id,
        &target.provider,
        &target.provider_session_id,
        reason,
        latest_merge_key,
        provider_turn_id.unwrap_or(reason).to_string(),
        observed_at_ms,
    );
    let Some(external_observation_metadata) =
        crate::terminal::TerminalOutputExternalObservationMetadata::from_session_history_entry(
            &state_entry,
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
            state_entry.merge_key,
            recipient_attachment_ids,
            EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes(),
            external_observation_metadata,
        );
}

fn attached_external_observer_targets(app: &DaemonApp) -> Vec<AttachedExternalObserverTarget> {
    let cursor_store = app.attached_provider_transcript_cursor_store();
    let mut targets = BTreeMap::<String, AttachedExternalObserverTarget>::new();
    for agent in app.agents().list_agents() {
        let Ok(session) = app.sessions().get_session(agent.session_id()) else {
            continue;
        };
        let provider_run_id = app
            .providers()
            .get_latest_run_for_agent(session.id(), agent.id())
            .map(|run| run.id().to_string());
        if let Some(import) = agent.external_provider_import().cloned() {
            let target = attached_external_observer_target_from_import(
                session.id(),
                agent.id(),
                provider_run_id.clone(),
                import,
            );
            targets.insert(attached_observer_target_key(&target), target);
        }
        for target in attached_external_observer_targets_from_resume_state(
            &cursor_store,
            session.id(),
            agent.id(),
            provider_run_id.clone(),
            agent.provider_resume_state(),
        ) {
            targets
                .entry(attached_observer_target_key(&target))
                .or_insert(target);
        }
    }
    for run in app.providers().list_runs() {
        let Some(agent_id) = run.agent_instance_id() else {
            continue;
        };
        if app.sessions().get_session(run.session_id()).is_err()
            || app.agents().get_agent(agent_id).is_err()
        {
            continue;
        }
        for target in attached_external_observer_targets_from_provider_run(&cursor_store, &run) {
            targets
                .entry(attached_observer_target_key(&target))
                .or_insert(target);
        }
    }
    targets.into_values().collect()
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

fn attached_external_observer_target_from_import(
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
    }
}

fn attached_external_observer_targets_from_provider_run(
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
    ));
    targets
}

fn attached_external_observer_targets_from_resume_state(
    cursor_store: &crate::app::AttachedProviderTranscriptCursorStore,
    session_id: &str,
    agent_id: &str,
    provider_run_id: Option<String>,
    resume_state: &ProviderResumeState,
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
            )
        })
        .collect()
}

fn attached_external_observer_target_from_provider_session(
    cursor_store: &crate::app::AttachedProviderTranscriptCursorStore,
    session_id: &str,
    agent_id: &str,
    provider_run_id: Option<String>,
    provider: &str,
    provider_session_id: &str,
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
        cursor_source: AttachedExternalObserverCursorSource::ArrobaOwned(cursor_key),
    })
}

fn attached_observer_target_key(target: &AttachedExternalObserverTarget) -> String {
    format!(
        "{}:{}:{}",
        target.session_id, target.agent_id, target.external_session_id
    )
}

fn persist_attached_external_observer_cursor(
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
        AttachedExternalObserverCursorSource::ArrobaOwned(key) => {
            app.attached_provider_transcript_cursor_store()
                .set(key.clone(), cursor);
        }
    }
    Ok(())
}

fn persist_external_import_metadata(
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
    use crate::history::{SessionHistoryEntryKind, SessionHistoryEntrySource};
    use crate::local::{
        ExternalProviderSessionCapabilities, ImportExternalProviderAgentRequest,
        ImportExternalProviderSessionRequest,
    };
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn observer_target(agent_id: &str) -> AttachedExternalObserverTarget {
        attached_external_observer_target_from_import(
            format!("session-{agent_id}"),
            agent_id.to_string(),
            None,
            ExternalProviderImportMetadata::observed_history(
                format!("codex:thread-{agent_id}"),
                "codex".to_string(),
                format!("thread-{agent_id}"),
            ),
        )
    }

    fn test_codex_run(
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        provider_session_id: &str,
    ) -> RuntimeProviderRun {
        let request =
            LaunchProviderRequest::new(session_id, "codex", "codex", "default", "gpt-test")
                .with_agent_id(agent_id);
        let launch = crate::provider::ProviderLaunchResult {
            process_label: "codex:test".to_string(),
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("codex:test".to_string()),
        };
        let mut run = RuntimeProviderRun::new(provider_run_id, &request, launch);
        run.set_provider_session_id(Some(provider_session_id.to_string()));
        run.mark_running();
        run
    }

    fn single_attached_target(app: &DaemonApp) -> AttachedExternalObserverTarget {
        let targets = attached_external_observer_targets(app);
        assert_eq!(
            targets.len(),
            1,
            "expected exactly one attached observer target"
        );
        targets.into_iter().next().expect("target should exist")
    }

    #[test]
    fn due_attached_external_observer_targets_prioritizes_overdue_targets() {
        let now = tokio::time::Instant::now();
        let mut schedule = BTreeMap::new();
        for agent in ["a", "b"] {
            schedule.insert(
                attached_observer_target_key(&observer_target(agent)),
                AttachedExternalObserverSchedule {
                    next_due_at: now,
                    active_until: Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW),
                    last_changed_at: None,
                    consecutive_errors: 0,
                },
            );
        }
        for agent in ["c", "d"] {
            schedule.insert(
                attached_observer_target_key(&observer_target(agent)),
                AttachedExternalObserverSchedule {
                    next_due_at: now - Duration::from_secs(10),
                    active_until: Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW),
                    last_changed_at: None,
                    consecutive_errors: 0,
                },
            );
        }

        let due = due_attached_external_observer_targets(
            ["a", "b", "c", "d"]
                .into_iter()
                .map(observer_target)
                .collect(),
            &mut schedule,
            now,
            2,
        );

        assert_eq!(
            due.iter()
                .map(|target| target.agent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "d"]
        );
    }

    #[test]
    fn import_external_provider_session_creates_session_agent_and_run() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let store = {
                let app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
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
            assert!(store
                .get("dev-stub:external-1")
                .expect("record should remain indexed")
                .is_attached_to_arroba());
        });
    }

    #[test]
    fn persist_external_import_metadata_refreshes_runtime_session_projection() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-import-projection",
                "worktree-import-projection",
            ))
            .expect("session should create");
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
        app.sessions
            .set_active_provider_run(session.id(), None)
            .expect("test should clear stale stored active run");
        app.update_session_projection(
            app.sessions()
                .get_session(session.id())
                .expect("session should still exist"),
        );

        persist_external_import_metadata(
            &mut app,
            session.id(),
            agent.id(),
            ExternalProviderImportMetadata::observed_history(
                "dev-stub:external-import-projection".to_string(),
                "dev-stub".to_string(),
                "external-import-projection".to_string(),
            ),
        )
        .expect("external import metadata should persist");

        let projected = app
            .session_state_projection_store()
            .get(session.id())
            .expect("session projection should refresh");
        assert_eq!(projected.active_provider_run_id(), Some(run.id()));
        assert_eq!(
            projected.external_provider_imports()[0].external_provider_session_id,
            "dev-stub:external-import-projection"
        );
        let projected_agent = projected
            .agents()
            .iter()
            .find(|projected_agent| projected_agent.id() == agent.id())
            .expect("projected session should include imported agent");
        assert_eq!(
            projected_agent
                .external_provider_import()
                .expect("projected agent import metadata should refresh")
                .external_provider_session_provider_id,
            "external-import-projection"
        );
    }

    #[test]
    fn import_codex_session_without_model_uses_persisted_thread_model() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let store = {
                let app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
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
                let app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
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
    fn import_external_provider_session_rejects_thread_owned_by_agent_resume_state() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let (session_id, agent_id, store) = {
                let mut app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
                let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new("workspace", "worktree"))
                    .expect("session should create");
                app.agents()
                    .set_agent_runtime_profile(
                        agent.id(),
                        "codex",
                        Some("gpt-test".to_string()),
                        None,
                        ProviderResumeState::from_codex_thread_id("thread-owned-by-resume"),
                    )
                    .expect("agent runtime profile should update");
                let store = app.external_provider_session_index_store();
                (session.id().to_string(), agent.id().to_string(), store)
            };
            store.upsert(record(
                "codex",
                "thread-owned-by-resume",
                "/tmp/thread-owned-by-resume",
            ));
            assert!(
                store
                    .get("codex:thread-owned-by-resume")
                    .expect("record should start indexed")
                    .is_attachable_to_arroba(),
                "test starts from a stale attachable store record",
            );

            let error = execute_external_provider_session_request(
                &app,
                None,
                LocalDaemonRequest::ImportExternalProviderSession(
                    ImportExternalProviderSessionRequest {
                        external_session_id: "codex:thread-owned-by-resume".to_string(),
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
            .expect_err("Arroba-owned Codex thread should not import as a second session");

            let message = error.to_string();
            assert!(message.contains(&format!(
                "already attached to Arroba session `{session_id}` agent `{agent_id}`"
            )));
            assert!(store
                .get("codex:thread-owned-by-resume")
                .expect("record should remain indexed")
                .is_attached_to_arroba());
        });
    }

    #[test]
    fn import_external_provider_session_rejects_discovered_thread_owned_by_agent_resume_state() {
        let _guard = crate::env_lock::lock();
        let codex_home = temp_root("codex-owned-discovery");
        let previous_codex_home = env::var_os("CODEX_HOME");
        env::set_var("CODEX_HOME", &codex_home);
        let session_dir = codex_home.join("archived_sessions");
        fs::create_dir_all(&session_dir).expect("codex session dir should create");
        fs::write(
            session_dir.join("owned-discovered.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-owned-discovered\",\"cwd\":\"/tmp/owned-discovered\",\"model_provider\":\"openai\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"This thread was created by Arroba and should not be attachable.\"}]}}\n",
            ),
        )
        .expect("codex session should write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let (session_id, agent_id, store) = {
                let mut app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
                let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new("workspace", "worktree"))
                    .expect("session should create");
                app.agents()
                    .set_agent_runtime_profile(
                        agent.id(),
                        "codex",
                        Some("gpt-test".to_string()),
                        None,
                        ProviderResumeState::from_codex_thread_id("thread-owned-discovered"),
                    )
                    .expect("agent runtime profile should update");
                let store = app.external_provider_session_index_store();
                (session.id().to_string(), agent.id().to_string(), store)
            };
            assert!(
                store.get("codex:thread-owned-discovered").is_none(),
                "test must start from an empty external session cache"
            );

            let error = execute_external_provider_session_request(
                &app,
                None,
                LocalDaemonRequest::ImportExternalProviderSession(
                    ImportExternalProviderSessionRequest {
                        external_session_id: "codex:thread-owned-discovered".to_string(),
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
            .expect_err("discovered Arroba-owned Codex thread should not import");

            let message = error.to_string();
            assert!(message.contains(&format!(
                "already attached to Arroba session `{session_id}` agent `{agent_id}`"
            )));
            assert!(store
                .get("codex:thread-owned-discovered")
                .expect("discovered record should remain indexed")
                .is_attached_to_arroba());
        });

        restore_env_var("CODEX_HOME", previous_codex_home);
        let _ = fs::remove_dir_all(codex_home);
    }

    #[test]
    fn import_external_provider_agent_adds_agent_to_existing_session() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let (session_id, store) = {
                let mut app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
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
    fn import_external_provider_agent_rejects_thread_owned_by_provider_run() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
        runtime.block_on(async {
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
            ));
            let (target_session_id, owner_session_id, owner_agent_id, store) = {
                let mut app = crate::runtime::app_lock::lock_app_instrumented(
                    &app,
                    "external_provider_session_control",
                )
                .await;
                let (target_session, _) = crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new(
                        "workspace-target",
                        "worktree-target",
                    ))
                    .expect("target session should create");
                let (owner_session, owner_agent) = crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new(
                        "workspace-owner",
                        "worktree-owner",
                    ))
                    .expect("owner session should create");
                let run = test_codex_run(
                    owner_session.id(),
                    owner_agent.id(),
                    "run-owned-thread",
                    "thread-owned-by-run",
                );
                app.providers_mut().insert_run_for_test(run);
                let store = app.external_provider_session_index_store();
                (
                    target_session.id().to_string(),
                    owner_session.id().to_string(),
                    owner_agent.id().to_string(),
                    store,
                )
            };
            store.upsert(record(
                "codex",
                "thread-owned-by-run",
                "/tmp/thread-owned-by-run",
            ));
            assert!(
                store
                    .get("codex:thread-owned-by-run")
                    .expect("record should start indexed")
                    .is_attachable_to_arroba(),
                "test starts from a stale attachable store record",
            );

            let error = execute_external_provider_session_request(
                &app,
                None,
                LocalDaemonRequest::ImportExternalProviderAgent(
                    ImportExternalProviderAgentRequest {
                        session_id: target_session_id,
                        external_session_id: "codex:thread-owned-by-run".to_string(),
                        alias: None,
                        provider: None,
                        model: None,
                        effort: None,
                        focus: Some(true),
                    },
                ),
                "external-agent-user",
            )
            .await
            .expect_err("Arroba-owned Codex thread should not import as a second agent");

            let message = error.to_string();
            assert!(message.contains(&format!(
                "already attached to Arroba session `{owner_session_id}` agent `{owner_agent_id}`"
            )));
            assert!(store
                .get("codex:thread-owned-by-run")
                .expect("record should remain indexed")
                .is_attached_to_arroba());
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
        assert!(attached.is_attached_to_arroba());
        assert_eq!(attached.first_attached_session_id(), Some(session.id()));
        assert_eq!(attached.first_attached_agent_id(), Some(agent.id()));
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
    fn arroba_owned_provider_run_provider_session_id_becomes_observer_target() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let run = test_codex_run(
            session.id(),
            agent.id(),
            "run-arroba-owned",
            "thread-arroba",
        );
        app.providers_mut().insert_run_for_test(run.clone());

        let target = single_attached_target(&app);

        assert_eq!(target.session_id, session.id());
        assert_eq!(target.agent_id, agent.id());
        assert_eq!(target.provider_run_id.as_deref(), Some(run.id()));
        assert_eq!(target.external_session_id, "codex:thread-arroba");
        assert_eq!(target.provider, "codex");
        assert_eq!(target.provider_session_id, "thread-arroba");
        assert!(matches!(
            target.cursor_source,
            AttachedExternalObserverCursorSource::ArrobaOwned(_)
        ));
        assert!(app
            .agents()
            .get_agent(agent.id())
            .expect("agent should load")
            .external_provider_import()
            .is_none());
    }

    #[test]
    fn imported_observer_target_keeps_import_cursor_source_when_provider_run_matches() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let import = ExternalProviderImportMetadata::observed_history(
            "codex:thread-imported".to_string(),
            "codex".to_string(),
            "thread-imported".to_string(),
        );
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("import metadata should persist");
        let run = test_codex_run(session.id(), agent.id(), "run-imported", "thread-imported");
        app.providers_mut().insert_run_for_test(run.clone());

        let target = single_attached_target(&app);

        assert_eq!(target.provider_run_id.as_deref(), Some(run.id()));
        assert!(matches!(
            target.cursor_source,
            AttachedExternalObserverCursorSource::Imported(_)
        ));
    }

    #[test]
    fn append_observed_external_user_turn_for_arroba_owned_run_adds_history_and_active_prompt() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let run = test_codex_run(
            session.id(),
            agent.id(),
            "run-arroba-owned",
            "thread-arroba",
        );
        app.providers_mut().insert_run_for_test(run);
        let target = single_attached_target(&app);

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-native".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::User,
                    text: "native prompt outside Arroba".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("observed external native user turn should append");

        assert_eq!(outcome.changed_count, 1);
        let active = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .expect("external active prompt should be set");
        assert_eq!(active.prompt_origin(), PromptOrigin::External);
        assert_eq!(active.prompt(), "native prompt outside Arroba");
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_external_provider_observed());
        assert_eq!(entries[0].text, "native prompt outside Arroba");
        assert!(app
            .agents()
            .get_agent(agent.id())
            .expect("agent should load")
            .external_provider_import()
            .is_none());
    }

    #[test]
    fn append_observed_arroba_owned_prompt_echoes_are_skipped_without_import_metadata() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let run = test_codex_run(
            session.id(),
            agent.id(),
            "run-arroba-owned",
            "thread-arroba",
        );
        app.providers_mut().insert_run_for_test(run);
        app.append_history_entry(
            session.id(),
            SessionHistoryEntry::user_prompt(
                session.id(),
                "attachment-1",
                agent.id(),
                "arroba owned prompt",
            ),
        );
        let target = single_attached_target(&app);
        let cursor_key = match target.cursor_source.clone() {
            AttachedExternalObserverCursorSource::ArrobaOwned(key) => key,
            AttachedExternalObserverCursorSource::Imported(_) => {
                panic!("Arroba-owned target should not use import metadata")
            }
        };

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-owned".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "arroba owned prompt\n<image name=[Image #1] path=\"/tmp/screenshot.png\"> </image>".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-owned".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "reply to Arroba owned prompt".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("observed Arroba-owned prompt echo should be skipped");

        assert_eq!(outcome.changed_count, 0);
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none());
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "arroba owned prompt");
        assert!(app
            .agents()
            .get_agent(agent.id())
            .expect("agent should load")
            .external_provider_import()
            .is_none());
        let cursor = app
            .attached_provider_transcript_cursor_store()
            .get(&cursor_key);
        assert_eq!(
            cursor.last_observed_turn_id.as_deref(),
            Some("assistant-owned")
        );
    }

    #[tokio::test]
    async fn append_observed_arroba_owned_completion_settles_and_advances_queued_prompt() {
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
        let run = test_codex_run(
            session.id(),
            agent.id(),
            "run-arroba-owned",
            "thread-arroba",
        );
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        let target = single_attached_target(&app);
        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-native".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::User,
                    text: "native prompt outside Arroba".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("native user turn should mark active prompt");
        let queued_prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "queued Arroba prompt",
            PromptStatus::Queued,
        );
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
            .expect("Arroba prompt should queue behind external active prompt")
        else {
            panic!("Arroba prompt should not start while external active prompt is running");
        };
        let queued_prompt_id = prompt.id().to_string();
        assert!(queued_prompt_id.starts_with("pending-prompt-"));
        assert_eq!(prompt.pending_prompt_id(), Some(queued_prompt_id.as_str()));

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-native".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "native prompt outside Arroba".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("complete-native".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Status,
                        text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("completion should settle active external prompt");
        assert!(outcome.external_active_prompt_settled);

        let app = Arc::new(tokio::sync::Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::clone(&app),
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let dispatched = router
            .runtime_state()
            .dispatch_next_queued_prompt_after_external_settlement(
                session.id(),
                agent.id(),
                run.id(),
            )
            .await
            .expect("queued prompt should dispatch after external settlement");
        assert!(dispatched);
        let session = router
            .runtime_state()
            .session_snapshot(session.id())
            .await
            .expect("session should snapshot");
        let active_prompt = session
            .active_prompt_for_agent(agent.id())
            .expect("promoted queued prompt should become active");
        assert_ne!(active_prompt.id(), queued_prompt_id);
        assert!(active_prompt.id().starts_with("prompt-"));
        assert_eq!(active_prompt.pending_prompt_id(), None);
        assert_eq!(active_prompt.prompt(), "queued Arroba prompt");
        assert!(session
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.is_empty())
            .unwrap_or(true));
    }

    #[tokio::test]
    async fn observed_external_completion_advances_queue_when_active_prompt_mirror_was_lost() {
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
        let run = test_codex_run(
            session.id(),
            agent.id(),
            "run-external-lost-mirror",
            "thread-external-lost-mirror",
        );
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        let target = single_attached_target(&app);

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-native".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::User,
                    text: "native prompt outside Arroba".to_string(),
                    observed_at_ms: Some(42),
                }],
            },
        )
        .expect("native user turn should mark active prompt");
        let queued_prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "queued Arroba prompt",
            PromptStatus::Queued,
        );
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
            .expect("Arroba prompt should queue behind external active prompt")
        else {
            panic!("Arroba prompt should not start while external active prompt is running");
        };
        let queued_prompt_id = prompt.id().to_string();
        app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("test drift should clear the external active prompt mirror");
        let mirrored_session = app
            .sessions()
            .get_session(session.id())
            .expect("session mirror should load");
        assert!(
            mirrored_session
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "test fixture should model a lost external active prompt mirror"
        );
        assert_eq!(
            mirrored_session
                .queued_prompts_for_agent(agent.id())
                .map(|queued| queued.len()),
            Some(1)
        );

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-native".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "native prompt outside Arroba".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("complete-native".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Status,
                        text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("completion should settle even when the mirror was already missing");
        assert!(outcome.external_active_prompt_settled);

        let app = Arc::new(tokio::sync::Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::clone(&app),
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let dispatched = router
            .runtime_state()
            .dispatch_next_queued_prompt_after_external_settlement(
                session.id(),
                agent.id(),
                run.id(),
            )
            .await
            .expect("queued prompt should dispatch after external settlement");
        assert!(dispatched);
        let session = router
            .runtime_state()
            .session_snapshot(session.id())
            .await
            .expect("session should snapshot");
        let active_prompt = session
            .active_prompt_for_agent(agent.id())
            .expect("promoted queued prompt should become active");
        assert_ne!(active_prompt.id(), queued_prompt_id);
        assert_eq!(active_prompt.pending_prompt_id(), None);
        assert_eq!(active_prompt.prompt(), "queued Arroba prompt");
    }

    #[test]
    fn stable_external_settlement_with_lost_mirror_signals_hidden_history_refresh() {
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
            "claude:thread-observed".to_string(),
            "claude".to_string(),
            "thread-observed".to_string(),
        );
        let agent =
            persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
                .expect("metadata should persist");
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "external reply".to_string(),
            observed_at_ms: Some(84),
        };

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed user turn should append");
        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant.clone()],
            },
        )
        .expect("observed assistant turn should append");
        let _ = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());

        let queued_prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "queued Arroba prompt",
            PromptStatus::Queued,
        );
        let crate::session::PromptSubmissionOutcome::Queued { .. } = app
            .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
            .expect("Arroba prompt should queue behind external active prompt")
        else {
            panic!("Arroba prompt should not start while external prompt is running");
        };
        app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("test drift should clear the external active prompt mirror");

        let outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, assistant],
            },
            AttachedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("stable assistant turn should settle even when mirror is missing");

        assert_eq!(outcome.changed_count, 0);
        assert!(outcome.external_active_prompt_settled);
        let records = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(
            records.len(),
            1,
            "hidden settlement history signal should fan out to attached terminals"
        );
        assert_eq!(
            records[0].bytes,
            EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
        );
        assert!(records[0]
            .merge_key
            .as_deref()
            .is_some_and(|merge_key| merge_key.contains(":state:active_prompt_settled:")));
        assert_eq!(
            records[0]
                .external_observation_metadata
                .as_ref()
                .and_then(|metadata| metadata.external_observation.as_ref())
                .map(|observation| observation.settles_active_prompt),
            Some(true)
        );
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
        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    None,
                    import,
                ),
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
    fn append_observed_external_turns_persist_as_reloadable_regular_history_turn() {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "external prompt".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("reasoning-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Reasoning,
                        text: "external reasoning".to_string(),
                        observed_at_ms: Some(63),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("tool-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Tool,
                        text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
                        observed_at_ms: Some(72),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "external answer".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("observed external turn should append");

        assert_eq!(outcome.changed_count, 4);
        let legacy_entries = app
            .history_store()
            .load(&session)
            .expect("legacy session history should load");
        assert_eq!(
            legacy_entries
                .iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>(),
            vec![
                SessionHistoryEntryKind::UserPrompt,
                SessionHistoryEntryKind::ProviderReasoning,
                SessionHistoryEntryKind::ProviderTool,
                SessionHistoryEntryKind::ProviderOutput,
            ]
        );
        assert_eq!(legacy_entries[0].text, "external prompt");
        assert_eq!(
            legacy_entries[0].source,
            Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved)
        );

        let response = tokio::runtime::Runtime::new()
            .expect("runtime should create")
            .block_on(
                crate::runtime::history_requests::execute_session_history_outline_request(
                    app.operational_history_store(),
                    crate::local::GetSessionHistoryOutlineRequest {
                        session_id: session.id().to_string(),
                        agent_ids: Some(vec![agent.id().to_string()]),
                        latest_prompt_count: Some(4),
                        cursor: None,
                    },
                ),
            )
            .expect("outline should load");
        let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } = response else {
            panic!("unexpected response")
        };
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].turns.len(), 1);
        let turn = &agents[0].turns[0];
        assert_eq!(turn.prompt_origin, PromptOrigin::External);
        assert_eq!(turn.external_provider.as_deref(), Some("codex"));
        assert_eq!(
            turn.external_provider_session_id.as_deref(),
            Some("thread-observed")
        );
        assert_eq!(turn.external_provider_turn_id.as_deref(), Some("user-1"));
        assert_eq!(turn.user_prompt.entry.text, "external prompt");
        assert_eq!(
            turn.user_prompt.entry.source,
            Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved)
        );
        assert_eq!(
            turn.summary
                .as_ref()
                .expect("assistant summary should load")
                .entry
                .text,
            "external answer"
        );
        assert_eq!(turn.blobs.len(), 2);
        assert_eq!(
            turn.blobs[0].kind,
            SessionHistoryEntryKind::ProviderReasoning
        );
        assert_eq!(turn.blobs[1].kind, SessionHistoryEntryKind::ProviderTool);
    }

    #[test]
    fn append_observed_external_turns_do_not_attribute_leading_blobs_to_future_prompt() {
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

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    None,
                    import,
                ),
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("status-before-user".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Status,
                        text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
                            .to_string(),
                        observed_at_ms: Some(21),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "external prompt".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-1".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "external answer".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("observed external turn should append");

        assert_eq!(outcome.changed_count, 3);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, SessionHistoryEntryKind::ProviderStatus);
        assert_eq!(
            entries[0].external_provider_turn_id.as_deref(),
            Some("status-before-user")
        );
        assert_eq!(
            entries[1].external_provider_turn_id.as_deref(),
            Some("user-1")
        );
        assert_eq!(
            entries[2].external_provider_turn_id.as_deref(),
            Some("user-1")
        );
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

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    None,
                    import,
                ),
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import.clone(),
        );
        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let first_assistant_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let stable_assistant_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant.clone()],
            },
        )
        .expect("observed Codex assistant turn should append");

        let stable_assistant_outcome =
            append_observed_external_turns_for_attached_target_with_options(
                &mut app,
                AttachedExternalObserverRead {
                    target: target.clone(),
                    turns: vec![prompt.clone(), assistant],
                },
                AttachedExternalObserverAppendOptions {
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

        let complete_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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
    fn append_observed_external_codex_turn_aborted_settles_active_prompt() {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
        let prompt = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("user-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            observed_at_ms: Some(42),
        };
        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed prompt should append");
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "observed prompt should create an external active prompt"
        );

        let abort = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("turn-aborted-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "codex event turn_aborted { \"type\": \"turn_aborted\" }".to_string(),
            observed_at_ms: Some(84),
        };
        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, abort],
            },
        )
        .expect("Codex turn_aborted should append and settle");

        assert_eq!(outcome.changed_count, 1);
        assert!(outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "Codex turn_aborted should clear the external active prompt"
        );
    }

    #[test]
    fn append_observed_external_provider_abort_completion_statuses_settle_active_prompt() {
        for (provider, status_text) in [
            (
                "claude",
                "claude message completed\n{\"stop_reason\":\"interrupted\"}",
            ),
            (
                "opencode",
                "opencode message completed\n{\"finish\":\"cancelled\"}",
            ),
        ] {
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let import = ExternalProviderImportMetadata::observed_history(
                format!("{provider}:thread-observed"),
                provider.to_string(),
                "thread-observed".to_string(),
            );
            let agent = persist_external_import_metadata(
                &mut app,
                session.id(),
                agent.id(),
                import.clone(),
            )
            .expect("metadata should persist");
            let target = attached_external_observer_target_from_import(
                session.id().to_string(),
                agent.id().to_string(),
                None,
                import,
            );
            let prompt = crate::app::ObservedExternalProviderTurn {
                provider_turn_id: Some("user-1".to_string()),
                role: crate::app::ObservedExternalProviderTurnRole::User,
                text: "external prompt".to_string(),
                observed_at_ms: Some(42),
            };
            append_observed_external_turns_for_attached_target(
                &mut app,
                AttachedExternalObserverRead {
                    target: target.clone(),
                    turns: vec![prompt.clone()],
                },
            )
            .expect("observed prompt should append");
            assert!(
                app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                    .expect("active prompt should load")
                    .is_some(),
                "{provider} prompt should mark the external turn running"
            );

            let abort_status = crate::app::ObservedExternalProviderTurn {
                provider_turn_id: Some("abort-status-1".to_string()),
                role: crate::app::ObservedExternalProviderTurnRole::Status,
                text: status_text.to_string(),
                observed_at_ms: Some(84),
            };
            let outcome = append_observed_external_turns_for_attached_target(
                &mut app,
                AttachedExternalObserverRead {
                    target,
                    turns: vec![prompt, abort_status],
                },
            )
            .expect("observed abort-like completion status should append and settle");

            assert!(
                outcome.external_active_prompt_settled,
                "{provider} abort-like completion status should settle"
            );
            assert!(
                app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                    .expect("active prompt should load")
                    .is_none(),
                "{provider} abort-like completion status should clear the external active prompt"
            );
            let entries = app
                .history_store()
                .load(&session)
                .expect("history should load");
            let status_entry = entries
                .iter()
                .find(|entry| entry.kind == SessionHistoryEntryKind::ProviderStatus)
                .expect("status history entry should persist");
            assert_eq!(
                status_entry
                    .external_observation
                    .as_ref()
                    .map(|observation| observation.settles_active_prompt),
                Some(true),
                "{provider} settling status should persist structured observation metadata"
            );
        }
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
        let assistant = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "external reply".to_string(),
            observed_at_ms: Some(84),
        };

        let first_assistant_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![assistant.clone()],
            },
        )
        .expect("observed assistant turn should append");

        let early_stable_outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![assistant.clone()],
            },
            AttachedExternalObserverAppendOptions {
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

        let late_stable_outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![assistant],
            },
            AttachedExternalObserverAppendOptions {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        let initial_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant.clone(), telemetry.clone()],
            },
        )
        .expect("observed Claude turn should append");
        assert_eq!(initial_outcome.changed_count, 3);
        assert_eq!(initial_outcome.active_relevant_changed_count, 2);

        let outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, assistant, telemetry],
            },
            AttachedExternalObserverAppendOptions {
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
        let events = app
            .operational_history_store()
            .query_events(crate::history::HistoryEventQuery {
                session_id: Some(session.id().to_string()),
                agent_id: Some(agent.id().to_string()),
                limit: Some(20),
                ..crate::history::HistoryEventQuery::default()
            })
            .expect("history events should load");
        let settlement = events
            .iter()
            .filter_map(|event| event.to_session_history_entry())
            .find(|entry| {
                entry
                    .merge_key
                    .as_deref()
                    .is_some_and(|merge_key| merge_key.contains(":state:active_prompt_settled:"))
            })
            .expect("implicit Claude settlement must be durable history");
        assert_eq!(settlement.text, "");
        assert_eq!(
            settlement.external_provider_turn_id.as_deref(),
            Some("user-1"),
            "durable settlement must group with the external prompt turn"
        );
        assert_eq!(settlement.observed_at_ms, Some(126));
        assert_eq!(
            settlement
                .external_observation
                .as_ref()
                .map(|observation| observation.settles_active_prompt),
            Some(true)
        );
    }

    #[test]
    fn append_observed_external_claude_new_passive_telemetry_after_assistant_settles_immediately() {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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
        let initial_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), assistant.clone()],
            },
        )
        .expect("observed Claude assistant turn should append");
        assert_eq!(initial_outcome.changed_count, 2);
        assert_eq!(initial_outcome.active_relevant_changed_count, 2);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_some(),
            "new assistant output should keep the external turn running through the grace window"
        );

        let telemetry = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("last-prompt-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Status,
            text: "claude last-prompt {\"lastPrompt\":\"external prompt\"}".to_string(),
            observed_at_ms: Some(126),
        };
        let telemetry_outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, assistant, telemetry],
            },
            AttachedExternalObserverAppendOptions {
                allow_external_active_prompt_settlement: true,
            },
        )
        .expect("new passive telemetry should append and settle the stable assistant turn");

        assert_eq!(telemetry_outcome.changed_count, 1);
        assert_eq!(telemetry_outcome.active_relevant_changed_count, 0);
        assert!(telemetry_outcome.external_active_prompt_settled);
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                .expect("active prompt should load")
                .is_none(),
            "new Claude passive telemetry after a stable assistant message should settle immediately"
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        let initial_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, tool, assistant, completion, telemetry],
            },
            AttachedExternalObserverAppendOptions {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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
        let running_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let completed_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone(), tool.clone()],
            },
        )
        .expect("observed prompt and tool should append");

        let stable_tool_outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, tool],
            },
            AttachedExternalObserverAppendOptions {
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
        let run = test_codex_run(session.id(), agent.id(), "run-imported", "thread-observed");
        app.providers_mut().insert_run_for_test(run.clone());
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            Some(run.id().to_string()),
            import,
        );
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed prompt should append");

        let token_count_outcome = append_observed_external_turns_for_attached_target_with_options(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![prompt, token_count],
            },
            AttachedExternalObserverAppendOptions {
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
        assert_eq!(
            app.providers()
                .get_run(run.id())
                .expect("provider run should load")
                .usage(),
            crate::provider::ProviderRunTokenUsage {
                total_tokens: Some(42),
                last_tokens: Some(42),
                context_tokens: None,
                context_window: None,
            },
            "externally observed Codex token telemetry should update kernel provider-run usage"
        );
        assert_eq!(
            app.provider_run_projection_store()
                .get(run.id())
                .expect("projected provider run should load")
                .usage(),
            crate::provider::ProviderRunTokenUsage {
                total_tokens: Some(42),
                last_tokens: Some(42),
                context_tokens: None,
                context_window: None,
            },
            "externally observed Codex token telemetry should update client-visible provider-run projection"
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let first_status_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let stable_status_outcome =
            append_observed_external_turns_for_attached_target_with_options(
                &mut app,
                AttachedExternalObserverRead {
                    target,
                    turns: vec![prompt, status],
                },
                AttachedExternalObserverAppendOptions {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("observed user turn should append");

        let changed_reply_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let stable_reply_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let complete_outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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
    fn append_observed_external_turns_consumes_arroba_owned_prompt_matches_once() {
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
                "repeatable prompt",
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-owned".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "repeatable prompt".to_string(),
                        observed_at_ms: Some(42),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-owned".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "provider reply to arroba owned prompt".to_string(),
                        observed_at_ms: Some(84),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-external".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::User,
                        text: "repeatable prompt".to_string(),
                        observed_at_ms: Some(126),
                    },
                    crate::app::ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-external".to_string()),
                        role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                        text: "provider reply to external repeated prompt".to_string(),
                        observed_at_ms: Some(168),
                    },
                ],
            },
        )
        .expect("later repeated prompt should be observed as external");

        assert_eq!(outcome.changed_count, 2);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "repeatable prompt");
        assert_eq!(entries[1].text, "repeatable prompt");
        assert!(entries[1].is_external_provider_observed());
        assert_eq!(
            entries[1].merge_key.as_deref(),
            Some("external:codex:thread-observed:user-external")
        );
        assert_eq!(
            entries[2].text,
            "provider reply to external repeated prompt"
        );
        assert_eq!(
            entries[2].external_provider_turn_id.as_deref(),
            Some("user-external")
        );
        let active_prompt = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .expect("latest repeated external prompt should be active");
        assert_eq!(active_prompt.prompt_origin(), PromptOrigin::External);
        assert_eq!(active_prompt.prompt(), "repeatable prompt");
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

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    None,
                    import,
                ),
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
    fn append_observed_external_turns_replaces_changed_duplicate_merge_key() {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import.clone(),
        );
        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![crate::app::ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                    text: "complete external reply".to_string(),
                    observed_at_ms: Some(84),
                }],
            },
        )
        .expect("changed observed assistant duplicate should replace prior content");

        assert_eq!(outcome.changed_count, 1);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "complete external reply");
        assert_eq!(entries[0].observed_at_ms, Some(84));
        assert!(entries[0].is_external_provider_observed());
        assert_eq!(
            entries[0].merge_key.as_deref(),
            Some("external:codex:thread-observed:assistant-1")
        );
        assert_eq!(entries[0].external_provider.as_deref(), Some("codex"));
        assert_eq!(
            entries[0].external_provider_session_id.as_deref(),
            Some("thread-observed")
        );
        assert_eq!(
            entries[0].external_provider_turn_id.as_deref(),
            Some("assistant-1")
        );
    }

    #[test]
    fn append_observed_external_turns_uses_latest_duplicate_merge_key_per_poll() {
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import.clone(),
        );
        let turns = vec![
            crate::app::ObservedExternalProviderTurn {
                provider_turn_id: Some("assistant-1".to_string()),
                role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                text: "partial external reply".to_string(),
                observed_at_ms: Some(42),
            },
            crate::app::ObservedExternalProviderTurn {
                provider_turn_id: Some("assistant-1".to_string()),
                role: crate::app::ObservedExternalProviderTurnRole::Assistant,
                text: "complete external reply".to_string(),
                observed_at_ms: Some(84),
            },
        ];

        let initial = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: turns.clone(),
            },
        )
        .expect("latest duplicate observed assistant turn should append");

        assert_eq!(initial.changed_count, 1);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "complete external reply");
        assert_eq!(entries[0].observed_at_ms, Some(84));

        let stable = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead { target, turns },
        )
        .expect("same duplicate snapshot should not churn history");

        assert_eq!(stable.changed_count, 0);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "complete external reply");
    }

    #[test]
    fn append_observed_external_turns_ignores_provider_run_id_only_changes() {
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
        let turn = crate::app::ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: crate::app::ObservedExternalProviderTurnRole::Assistant,
            text: "complete external reply".to_string(),
            observed_at_ms: Some(84),
        };

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    None,
                    import.clone(),
                ),
                turns: vec![turn.clone()],
            },
        )
        .expect("initial observed assistant turn should append");

        let stable = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    Some("provider-run-2".to_string()),
                    import,
                ),
                turns: vec![turn],
            },
        )
        .expect("provider-run-only change should not churn external history");

        assert_eq!(stable.changed_count, 0);
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider_run_id, None);
        assert_eq!(entries[0].text, "complete external reply");
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
        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    Some(run.id().to_string()),
                    import,
                ),
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
        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: attached_external_observer_target_from_import(
                    session.id().to_string(),
                    agent.id().to_string(),
                    None,
                    import,
                ),
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
        assert_eq!(
            records[0].bytes,
            EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
        );
        let metadata = records[0]
            .external_observation_metadata
            .as_ref()
            .expect("external history refresh should carry observed metadata");
        assert_eq!(
            metadata.source,
            SessionHistoryEntrySource::ExternalProviderObserved
        );
        assert_eq!(metadata.external_provider.as_deref(), Some("codex"));
        assert_eq!(
            metadata.external_provider_session_id.as_deref(),
            Some("thread-observed")
        );
        assert_eq!(
            metadata.external_provider_turn_id.as_deref(),
            Some("item-1")
        );
        assert_eq!(metadata.observed_at_ms, Some(42));
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
        let target = attached_external_observer_target_from_import(
            session.id().to_string(),
            agent.id().to_string(),
            None,
            import,
        );
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

        append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target: target.clone(),
                turns: vec![prompt.clone()],
            },
        )
        .expect("prompt should append");
        let _ = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());

        let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
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
        assert_eq!(
            records[0].bytes,
            EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
        );
        assert_eq!(
            records[1].bytes,
            EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
        );
        assert_eq!(
            records[0]
                .external_observation_metadata
                .as_ref()
                .map(|metadata| metadata.source),
            Some(SessionHistoryEntrySource::ExternalProviderObserved)
        );
        assert_eq!(
            records[1].merge_key.as_deref(),
            Some(
                "external:codex:thread-observed:state:active_prompt_settled:external:codex:thread-observed:task-complete-1"
            )
        );
        let state_metadata = records[1]
            .external_observation_metadata
            .as_ref()
            .expect("state refresh should carry observed metadata");
        assert_eq!(
            state_metadata.source,
            SessionHistoryEntrySource::ExternalProviderObserved
        );
        assert_eq!(state_metadata.external_provider.as_deref(), Some("codex"));
        assert_eq!(
            state_metadata.external_provider_session_id.as_deref(),
            Some("thread-observed")
        );
        assert_eq!(
            state_metadata.external_provider_turn_id.as_deref(),
            Some("user-1")
        );
        assert_eq!(state_metadata.observed_at_ms, Some(84));
        assert_eq!(
            state_metadata
                .external_observation
                .as_ref()
                .map(|observation| observation.settles_active_prompt),
            Some(true)
        );
    }

    #[test]
    fn resume_state_maps_known_external_providers() {
        assert_eq!(
            ProviderResumeState::from_external_provider_session("codex", "thread-1")
                .codex_thread_id(),
            Some("thread-1")
        );
        assert_eq!(
            ProviderResumeState::from_external_provider_session("opencode", "session-1")
                .opencode_session_id(),
            Some("session-1")
        );
        assert_eq!(
            ProviderResumeState::from_external_provider_session("claude", "session-2")
                .claude_session_id(),
            Some("session-2")
        );
        assert!(
            ProviderResumeState::from_external_provider_session("dev-stub", "session-3").is_empty()
        );
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

    fn temp_root(name: &str) -> PathBuf {
        let path =
            env::temp_dir().join(format!("arroba-{name}-{}", crate::session::unix_epoch_ms()));
        fs::create_dir_all(&path).expect("temp root should create");
        path
    }

    fn restore_env_var(key: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => env::set_var(key, value),
            None => env::remove_var(key),
        }
    }
}
