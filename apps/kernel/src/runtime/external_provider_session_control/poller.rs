use super::*;

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
    let profile_roots = runtime_state
        .map(|runtime_state| registered_external_provider_profile_roots(runtime_state, None));
    let mut signature_read = match read_external_provider_discovery_signature(
        cached_candidate_paths,
        profile_roots.clone(),
    )
    .await
    {
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
            refresh_attached_external_provider_histories_matching(
                app,
                runtime_state,
                None,
                None,
                true,
                true,
            )
            .await;
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
        signature_read =
            match read_external_provider_discovery_signature(None, profile_roots.clone()).await {
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
        refresh_attached_external_provider_histories(app, runtime_state, None).await;
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
    let providers_to_refresh = if force {
        external_provider_session_providers()
            .iter()
            .map(|provider| (*provider).to_string())
            .collect::<BTreeSet<_>>()
    } else if let Some(cached_signature) = cached_signature {
        cached_signature.providers_with_changed_candidate_files(&signature)
    } else {
        external_provider_session_providers()
            .iter()
            .map(|provider| (*provider).to_string())
            .collect::<BTreeSet<_>>()
    };
    if providers_to_refresh.is_empty() {
        if let Some(cache) = cache.as_mut() {
            cache.signature = Some(signature);
            cache.candidate_paths = Some(signature_read.candidate_paths);
            cache.cached_signature_checks = 0;
        }
        refresh_attached_external_provider_histories(app, runtime_state, None).await;
        return;
    }
    let discovery_started = Instant::now();
    let discovery_providers = providers_to_refresh.clone();
    let discovery_profile_roots = profile_roots.clone();
    let discovered = match tokio::task::spawn_blocking(move || {
        discovery_providers
            .iter()
            .flat_map(|provider| {
                discovery_profile_roots.as_ref().map_or_else(
                    || crate::app::discover_external_provider_sessions(Some(provider.as_str())),
                    |profiles| {
                        crate::app::discover_external_provider_sessions_for_profiles(
                            profiles,
                            Some(provider.as_str()),
                        )
                    },
                )
            })
            .collect::<Vec<_>>()
    })
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
    let store = external_provider_session_index_store(app, runtime_state).await;
    for provider in &providers_to_refresh {
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
    let attached_refs =
        attached_external_provider_session_refs_for_runtime(app, runtime_state).await;
    for attachment in attached_refs {
        store.mark_attached(
            &attachment.external_session_id,
            &attachment.session_id,
            &attachment.agent_id,
        );
    }
    refresh_attached_external_provider_histories(app, runtime_state, None).await;
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
                "providers_refreshed": providers_to_refresh,
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
    profile_roots: Option<Vec<crate::app::ExternalProviderSessionProfileRoot>>,
) -> Option<ExternalProviderSessionDiscoverySignatureRead> {
    let full_scan = cached_candidate_paths.is_none();
    match tokio::task::spawn_blocking(move || {
        let candidate_paths = cached_candidate_paths.unwrap_or_else(|| match profile_roots {
            Some(profiles) => {
                crate::app::external_provider_session_candidate_paths_for_profiles(&profiles, None)
            }
            None => crate::app::external_provider_session_discovery_candidate_paths(None),
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
    let store = external_provider_session_index_store(app, runtime_state).await;
    let account_owner_user_id = runtime_state.map_or_else(
        || crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        |runtime_state| runtime_state.provider_account_authority_owner_user_id(caller_user_id),
    );
    match request {
        LocalDaemonRequest::ListExternalProviderSessions(request) => {
            refresh_external_provider_session_index(app, runtime_state, None, true).await;
            mark_attached_external_provider_sessions_for_runtime(app, runtime_state, &store).await;
            Ok(LocalDaemonResponse::ExternalProviderSessionsListed {
                page: store.list_for_owner(&account_owner_user_id, &request),
            })
        }
        LocalDaemonRequest::RefreshExternalProviderSessions(request) => {
            let provider = request.provider.clone();
            let discovered = runtime_state.map_or_else(
                || crate::app::discover_external_provider_sessions(provider.as_deref()),
                |runtime_state| {
                    crate::app::discover_external_provider_sessions_for_profiles(
                        &registered_external_provider_profile_roots(runtime_state, None),
                        provider.as_deref(),
                    )
                },
            );
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
            mark_attached_external_provider_sessions_for_runtime(app, runtime_state, &store).await;
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
                page: store.list_for_owner(&account_owner_user_id, &list_request),
            })
        }
        LocalDaemonRequest::ImportExternalProviderSession(request) => {
            import_external_provider_session_for_runtime(
                app,
                runtime_state,
                &store,
                request,
                caller_user_id,
                &account_owner_user_id,
            )
            .await
        }
        LocalDaemonRequest::ImportExternalProviderAgent(request) => {
            import_external_provider_agent_for_runtime(
                app,
                runtime_state,
                &store,
                request,
                caller_user_id,
                &account_owner_user_id,
            )
            .await
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
    // Background discovery must never turn every idle Chariox-owned agent into
    // provider transcript I/O. Active turns and explicitly imported sessions
    // opt into responsive observation; an attached UI requests its own
    // one-time session catch-up through the session-filtered path below.
    refresh_attached_external_provider_histories_matching(
        app,
        runtime_state,
        provider_filter,
        None,
        true,
        true,
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
        true,
        false,
    )
    .await;
}

pub(super) async fn refresh_attached_external_provider_histories_matching(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    provider_filter: Option<&str>,
    session_filter: Option<&str>,
    changed_transcripts_only: bool,
    responsive_targets_only: bool,
) {
    let targets =
        attached_external_observer_targets_for_runtime(app, runtime_state, responsive_targets_only)
            .await
            .into_iter()
            .filter(|target| {
                attached_external_observer_target_matches_refresh_filters(
                    target,
                    provider_filter,
                    session_filter,
                ) && (!responsive_targets_only || target.needs_responsive_refresh)
                    && (!changed_transcripts_only || {
                        runtime_state.map_or_else(
                    || {
                        crate::app::external_provider_session_transcript_needs_refresh(
                            &target.provider,
                            &target.provider_session_id,
                        )
                    },
                    |runtime_state| {
                        let roots = registered_external_provider_profile_roots_for(
                            runtime_state,
                            &target.owner_user_id,
                            &target.provider,
                            &target.account_profile,
                        );
                        crate::app::external_provider_session_transcript_needs_refresh_for_profile(
                            &target.provider,
                            &target.provider_session_id,
                            &roots,
                        )
                    },
                )
                    })
            })
            .collect::<Vec<_>>();
    for target in targets {
        let provider = target.provider.clone();
        let account_profile = target.account_profile.clone();
        let owner_user_id = target.owner_user_id.clone();
        let provider_session_id = target.provider_session_id.clone();
        let roots = runtime_state.map(|runtime_state| {
            registered_external_provider_profile_roots_for(
                runtime_state,
                &owner_user_id,
                &provider,
                &account_profile,
            )
        });
        let read = match tokio::task::spawn_blocking(move || {
            roots.map_or_else(
                || {
                    crate::app::read_external_provider_observed_turns(
                        &provider,
                        &provider_session_id,
                    )
                },
                |roots| {
                    crate::app::read_external_provider_observed_turns_for_profile(
                        &provider,
                        &provider_session_id,
                        &roots,
                    )
                },
            )
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
        let _ = append_observed_external_turns_for_attached_target_for_runtime(
            app,
            runtime_state,
            read,
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

async fn external_provider_session_index_store(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
) -> crate::app::ExternalProviderSessionIndexStore {
    if let Some(runtime_state) = runtime_state {
        return runtime_state
            .with_app_side_effect(|app| app.external_provider_session_index_store())
            .await;
    }
    let app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    app.external_provider_session_index_store()
}

async fn attached_external_observer_targets_for_runtime(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    responsive_targets_only: bool,
) -> Vec<AttachedExternalObserverTarget> {
    if let Some(runtime_state) = runtime_state {
        let (inputs, session_store) = runtime_state
            .with_app_side_effect(|app| {
                (
                    ExternalObserverRuntimeInputs::capture(app, responsive_targets_only),
                    app.session_state_store(),
                )
            })
            .await;
        return attached_external_observer_targets_from_inputs(&inputs, &session_store);
    }
    let app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    attached_external_observer_targets(&app, responsive_targets_only)
}

async fn attached_external_provider_session_refs_for_runtime(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
) -> BTreeSet<AttachedExternalProviderSessionRef> {
    if let Some(runtime_state) = runtime_state {
        let (inputs, session_store) = runtime_state
            .with_app_side_effect(|app| {
                (
                    ExternalObserverRuntimeInputs::capture(app, false),
                    app.session_state_store(),
                )
            })
            .await;
        return attached_external_provider_session_refs_from_inputs(
            &inputs,
            &session_store,
            runtime_state.provider_runs_for_external_session_attachment(),
        );
    }
    let app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    attached_external_provider_session_refs(&app, None)
}

async fn mark_attached_external_provider_sessions_for_runtime(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
) {
    if let Some(runtime_state) = runtime_state {
        let (inputs, session_store, runtime_provider_runs) = runtime_state
            .with_app_side_effect(|app| {
                (
                    ExternalObserverRuntimeInputs::capture(app, false),
                    app.session_state_store(),
                    runtime_state.provider_runs_for_external_session_attachment(),
                )
            })
            .await;
        let attached_refs = attached_external_provider_session_refs_from_inputs(
            &inputs,
            &session_store,
            runtime_provider_runs,
        );
        let known_agents = inputs.known_agent_keys();
        prune_stale_external_provider_session_refs_with_known_agents(
            &known_agents,
            &attached_refs,
            store,
        );
        for attachment in attached_refs {
            store.mark_attached(
                &attachment.external_session_id,
                &attachment.session_id,
                &attachment.agent_id,
            );
        }
        return;
    }
    let app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    mark_attached_external_provider_sessions(&app, None, store);
}

async fn append_observed_external_turns_for_attached_target_for_runtime(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    read: AttachedExternalObserverRead,
) -> AttachedExternalObserverAppendOutcome {
    if let Some(runtime_state) = runtime_state {
        return runtime_state
            .with_app_side_effect(|app| {
                append_observed_external_turns_for_attached_target(app, read).unwrap_or_default()
            })
            .await;
    }
    let mut app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    append_observed_external_turns_for_attached_target(&mut app, read).unwrap_or_default()
}

async fn import_external_provider_session_for_runtime(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderSessionRequest,
    caller_user_id: &str,
    account_owner_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    if let Some(runtime_state) = runtime_state {
        return runtime_state
            .with_app_side_effect(|app| {
                mark_attached_external_provider_sessions(app, Some(runtime_state), store);
                import_external_provider_session(
                    app,
                    Some(runtime_state),
                    store,
                    request,
                    caller_user_id,
                    account_owner_user_id,
                )
            })
            .await;
    }
    let mut app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    mark_attached_external_provider_sessions(&app, None, store);
    import_external_provider_session(
        &mut app,
        None,
        store,
        request,
        caller_user_id,
        account_owner_user_id,
    )
}

async fn import_external_provider_agent_for_runtime(
    app: &Arc<Mutex<DaemonApp>>,
    runtime_state: Option<&KernelRuntimeState>,
    store: &crate::app::ExternalProviderSessionIndexStore,
    request: ImportExternalProviderAgentRequest,
    caller_user_id: &str,
    account_owner_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    if let Some(runtime_state) = runtime_state {
        return runtime_state
            .with_app_side_effect(|app| {
                mark_attached_external_provider_sessions(app, Some(runtime_state), store);
                import_external_provider_agent(
                    app,
                    Some(runtime_state),
                    store,
                    request,
                    caller_user_id,
                    account_owner_user_id,
                )
            })
            .await;
    }
    let mut app = app
        .try_lock()
        .expect("legacy external-provider tests should not hold the daemon app lock");
    mark_attached_external_provider_sessions(&app, None, store);
    import_external_provider_agent(
        &mut app,
        None,
        store,
        request,
        caller_user_id,
        account_owner_user_id,
    )
}
