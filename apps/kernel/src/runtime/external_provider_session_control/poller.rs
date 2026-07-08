use super::*;

pub(super) async fn poll_attached_external_provider_transcripts(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: &crate::runtime::state::KernelRuntimeState,
    schedule: &mut BTreeMap<String, AttachedExternalObserverSchedule>,
) -> AttachedExternalObserverPollOutcome {
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
    if target_count == 0 {
        schedule.clear();
        return AttachedExternalObserverPollOutcome {
            target_count,
            due_count: 0,
        };
    }
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
        return AttachedExternalObserverPollOutcome {
            target_count,
            due_count: 0,
        };
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
    AttachedExternalObserverPollOutcome {
        target_count,
        due_count,
    }
}

pub(super) fn due_attached_external_observer_targets(
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

pub(super) async fn refresh_external_provider_session_index(
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
    let cached_signature = cache.as_ref().and_then(|cache| cache.signature.as_ref());
    if !signature_read.full_scan
        && cached_signature.is_some_and(|cached| cached != &signature_read.signature)
    {
        if cached_signature
            .is_some_and(|cached| cached.same_candidate_files(&signature_read.signature))
        {
            if let Some(cache) = cache.as_mut() {
                cache.signature = Some(signature_read.signature);
                cache.candidate_paths = Some(signature_read.candidate_paths);
                cache.cached_signature_checks = cache.cached_signature_checks.saturating_add(1);
            }
            let total_elapsed = refresh_started.elapsed();
            if total_elapsed >= EXTERNAL_PROVIDER_DISCOVERY_SLOW_SIGNATURE {
                crate::logging::info_with_fields(
                    "daemon.external_provider_sessions",
                    "external provider session discovery content changed",
                    serde_json::json!({
                        "signature_ms": signature_ms,
                        "total_ms": total_elapsed.as_millis(),
                        "full_scan": false,
                        "cached_signature_checks": cache
                            .as_ref()
                            .map(|cache| cache.cached_signature_checks)
                            .unwrap_or(0),
                    }),
                );
            }
            return;
        }
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

pub(super) async fn read_external_provider_discovery_signature(
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

pub(super) fn count_external_provider_sessions(
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
            refresh_external_provider_session_index(app, runtime_state, None, true).await;
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

pub(super) async fn refresh_attached_external_provider_histories(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    provider_filter: Option<&str>,
) {
    refresh_attached_external_provider_histories_matching(
        app,
        runtime_state,
        provider_filter,
        None,
    )
    .await;
}

pub(crate) async fn refresh_attached_external_provider_histories_for_session(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    session_id: &str,
) {
    refresh_attached_external_provider_histories_matching(
        app,
        runtime_state,
        None,
        Some(session_id),
    )
    .await;
}

pub(super) async fn refresh_attached_external_provider_histories_matching(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    provider_filter: Option<&str>,
    session_filter: Option<&str>,
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
                attached_external_observer_target_matches_refresh_filters(
                    target,
                    provider_filter,
                    session_filter,
                )
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

pub(super) fn attached_external_observer_target_matches_refresh_filters(
    target: &AttachedExternalObserverTarget,
    provider_filter: Option<&str>,
    session_filter: Option<&str>,
) -> bool {
    attached_external_observer_provider_matches_filter(&target.provider, provider_filter)
        && session_filter
            .map(|session_id| target.session_id == session_id)
            .unwrap_or(true)
}

pub(super) fn attached_external_observer_provider_matches_filter(
    target_provider: &str,
    provider_filter: Option<&str>,
) -> bool {
    let Some(provider_filter) = provider_filter else {
        return true;
    };
    let provider_filter = provider_filter.trim().to_ascii_lowercase();
    external_provider_session_providers().contains(&provider_filter.as_str())
        && target_provider == provider_filter
}

pub(super) async fn dispatch_next_queued_prompt_after_external_settlement(
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
