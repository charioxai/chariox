use super::*;

#[test]
fn get_provider_catalog_uses_warmed_projection_without_app_lock() {
    run_provider_projection_large_stack_test(
        "get-provider-catalog-uses-warmed-projection-without-app-lock",
        get_provider_catalog_uses_warmed_projection_without_app_lock_inner,
    );
}

async fn get_provider_catalog_uses_warmed_projection_without_app_lock_inner() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let catalog = OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        }],
        default: Default::default(),
        connected: vec!["codex".to_string()],
    };
    let request = GetProviderCatalogRequest::default();
    let cache_key = crate::runtime::provider_catalog_control::provider_catalog_cache_key(
        crate::session::DEFAULT_LOCAL_USER_ID,
        &request,
    );
    app.provider_catalog_projection_store()
        .update_scoped(&cache_key, catalog);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let app_guard = app.lock().await;
    let catalog_request = LocalDaemonRequest::GetProviderCatalog(request);
    let catalog_command = KernelCommand::from_local_request(
        "cmd-provider-catalog-projection",
        None,
        None,
        &catalog_request,
    );
    let catalog_router = router.clone();
    let catalog_task = tokio::spawn(async move {
        catalog_router
            .dispatch(catalog_command, catalog_request)
            .await
    });

    let catalog_response = tokio::time::timeout(Duration::from_millis(100), catalog_task)
        .await
        .expect("warmed GetProviderCatalog should not wait for the app lock")
        .expect("catalog task should join")
        .expect("catalog should resolve");
    drop(app_guard);

    match catalog_response {
        LocalDaemonResponse::ProviderCatalog { catalog } => {
            assert_eq!(catalog.connected, vec!["codex"]);
        }
        _ => panic!("unexpected provider catalog response"),
    }
}

#[test]
fn relay_configure_invalidates_provider_catalog_projection() {
    run_provider_projection_large_stack_test(
        "relay-configure-invalidates-provider-catalog-projection",
        relay_configure_invalidates_provider_catalog_projection_inner,
    );
}

async fn relay_configure_invalidates_provider_catalog_projection_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let catalog = OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        }],
        default: Default::default(),
        connected: vec!["codex".to_string()],
    };
    let request = GetProviderCatalogRequest::default();
    let cache_key = crate::runtime::provider_catalog_control::provider_catalog_cache_key(
        crate::session::DEFAULT_LOCAL_USER_ID,
        &request,
    );
    let store = app.provider_catalog_projection_store().clone();
    store.update_scoped(&cache_key, catalog.clone());
    let warmed_generation = store.invalidation_generation();
    app.configure_relay(None, None)
        .expect("relay configure should invalidate provider catalog projection");
    app.invalidate_provider_catalog_projection();

    // Invalidation rejects fresh-cache hits and fences an older refresh, but
    // keeps the scoped metadata as the stale copy served while discovery reruns.
    let ttl = crate::local::provider_requests::PROVIDER_CATALOG_CACHE_TTL;
    assert!(
        store.get_scoped(&cache_key, ttl).is_none(),
        "an invalidated catalog is no fresh hit"
    );
    assert_eq!(
        store.cached_scoped(&cache_key).map(|stale| stale.connected),
        Some(vec!["codex".to_string()])
    );
    assert!(
        !store.update_scoped_if_generation(&cache_key, catalog, warmed_generation),
        "a refresh that started before the invalidation must not publish"
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let catalog_request = LocalDaemonRequest::GetProviderCatalog(request);
    let catalog_command = KernelCommand::from_local_request(
        "cmd-provider-catalog-invalidated",
        None,
        None,
        &catalog_request,
    );
    let catalog_router = router.clone();
    let catalog_task = tokio::spawn(async move {
        catalog_router
            .dispatch(catalog_command, catalog_request)
            .await
    });
    let response = tokio::time::timeout(Duration::from_secs(2), catalog_task)
        .await
        .expect("an invalidated catalog is answered from the stale copy without the app lock")
        .expect("catalog task should join")
        .expect("catalog should resolve");
    drop(app_guard);
    match response {
        LocalDaemonResponse::ProviderCatalog { catalog } => {
            assert_eq!(catalog.connected, vec!["codex"])
        }
        _ => panic!("unexpected provider catalog response"),
    }
}

#[test]
fn mp08_mp10_mp11_catalog_revisions_use_owned_projection_without_app_lock() {
    run_provider_projection_large_stack_test("client-catalog-revisions-owned", || async {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let request = GetProviderCatalogRequest::default();
        let key = crate::runtime::provider_catalog_control::provider_catalog_cache_key(
            crate::session::DEFAULT_LOCAL_USER_ID,
            &request,
        );
        app.provider_catalog_projection_store().update_scoped(
            &key,
            OpenCodeProviderCatalog {
                all: Vec::new(),
                default: Default::default(),
                connected: vec!["codex".into()],
            },
        );
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let guard = app.lock().await;
        let request = LocalDaemonRequest::GetClientCatalogRevisions(request);
        let command =
            KernelCommand::from_local_request("catalog-revisions-owned", None, None, &request);
        let response = tokio::time::timeout(
            Duration::from_millis(100),
            router.dispatch(command, request),
        )
        .await
        .expect("catalog revisions must not wait on app mutex")
        .unwrap();
        drop(guard);
        let LocalDaemonResponse::ClientCatalogRevisions { revisions } = response else {
            panic!("revisions")
        };
        assert_eq!(revisions.provider_catalog.len(), 64);
        assert_eq!(revisions.provider_commands.len(), 64);
        assert_eq!(revisions.provider_accounts.len(), 64);
        assert!(revisions.terminal_commands.starts_with("sha256:"));
        assert_eq!(revisions.terminal_commands.len(), 71);
        assert!(serde_json::to_vec(&revisions).unwrap().len() < 400);
    });
}

#[test]
fn mp08_mp10_conditional_catalog_snapshot_retains_unchanged_sections() {
    run_provider_projection_large_stack_test("conditional-catalog-snapshot", || async {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let scope = GetProviderCatalogRequest::default();
        let key = crate::runtime::provider_catalog_control::provider_catalog_cache_key(
            crate::session::DEFAULT_LOCAL_USER_ID,
            &scope,
        );
        app.provider_catalog_projection_store().update_scoped(
            &key,
            OpenCodeProviderCatalog {
                all: Vec::new(),
                default: Default::default(),
                connected: Vec::new(),
            },
        );
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let guard = app.lock().await;
        let request: LocalDaemonRequest = serde_json::from_value(
            serde_json::json!({"GetClientCatalogSnapshot": {"scope": scope, "known": null}}),
        )
        .expect("conditional catalog snapshot request must be supported");
        let command = KernelCommand::from_local_request("catalog-full", None, None, &request);
        let response =
            tokio::time::timeout(Duration::from_secs(1), router.dispatch(command, request))
                .await
                .expect("catalog read must not wait on app mutex")
                .unwrap();
        let full = serde_json::to_value(response).unwrap();
        let snapshot = &full["ClientCatalogSnapshot"]["snapshot"];
        assert!(snapshot["provider_catalog"].is_object());
        assert!(snapshot["provider_accounts"].is_array());
        assert!(snapshot["provider_commands"].is_object());
        assert!(snapshot["terminal_commands"].is_object());
        let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({"GetClientCatalogSnapshot": {"scope": scope, "known": snapshot["revisions"]}})).unwrap();
        let command = KernelCommand::from_local_request("catalog-unchanged", None, None, &request);
        let response =
            tokio::time::timeout(Duration::from_secs(1), router.dispatch(command, request))
                .await
                .unwrap()
                .unwrap();
        drop(guard);
        let unchanged = serde_json::to_value(response).unwrap();
        assert_eq!(
            unchanged["ClientCatalogSnapshot"]["snapshot"],
            serde_json::json!({"revisions": snapshot["revisions"]})
        );
        assert!(serde_json::to_vec(&unchanged).unwrap().len() < 460);
    });
}

async fn slow_discovery_router(delay_ms: u64) -> (Arc<Mutex<DaemonApp>>, CommandRouter) {
    let mut config = DaemonConfig::for_tests();
    config.provider_catalog_read_delay_ms = delay_ms;
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    (app, router)
}

fn snapshot_request(known: Option<serde_json::Value>) -> LocalDaemonRequest {
    serde_json::from_value(serde_json::json!({"GetClientCatalogSnapshot": {"scope": GetProviderCatalogRequest::default(), "known": known}})).unwrap()
}

// MP-08 / MP-10 / MP-11: a client that already holds a catalog is never held back by model discovery.
#[test]
fn mp08_mp10_mp11_cold_scope_with_a_client_copy_answers_before_discovery_ends() {
    run_provider_projection_large_stack_test("cold-scope-client-copy", || async {
        let (_app, router) = slow_discovery_router(1500).await;
        let held = serde_json::json!({"provider_accounts": "0".repeat(64), "provider_catalog": "1".repeat(64), "provider_commands": "2".repeat(64), "terminal_commands": format!("sha256:{}", "3".repeat(64))});
        let request = snapshot_request(Some(held.clone()));
        let command = KernelCommand::from_local_request("cold-with-copy", None, None, &request);
        let response = tokio::time::timeout(
            Duration::from_millis(750),
            router.dispatch(command, request),
        )
        .await
        .expect("a cold scope must answer a client that holds a catalog before discovery ends")
        .unwrap();
        let value = serde_json::to_value(response).unwrap();
        let snapshot = &value["ClientCatalogSnapshot"]["snapshot"];
        assert!(
            snapshot["provider_accounts"].is_array(),
            "accounts never wait for models"
        );
        assert!(
            snapshot["provider_catalog"].is_null(),
            "the client's own copy stays until discovery ends"
        );
        assert_eq!(
            snapshot["revisions"]["provider_catalog"],
            held["provider_catalog"]
        );
    });
}

// MP-08 / MP-10 / MP-11: a rename during slow discovery is in the response that ends it, and is not an error.
#[test]
fn mp08_mp10_mp11_account_rename_during_discovery_is_in_the_response_that_ends_it() {
    run_provider_projection_large_stack_test("rename-during-discovery", || async {
        let (app, router) = slow_discovery_router(700).await;
        let (registry, store) = {
            let app = app.lock().await;
            (
                app.provider_account_profile_registry(),
                app.provider_catalog_projection_store(),
            )
        };
        let owner = crate::session::DEFAULT_LOCAL_USER_ID;
        let profile = registry.create_managed(owner, "claude", "before").unwrap();
        let request = snapshot_request(None);
        let command =
            KernelCommand::from_local_request("rename-during-discovery", None, None, &request);
        let read = tokio::spawn({
            let router = router.clone();
            async move { router.dispatch(command, request).await }
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        registry
            .rename(owner, "claude", &profile.profile_id, "after")
            .unwrap();
        store.invalidate();
        let response = read
            .await
            .unwrap()
            .expect("an invalidation during discovery must not fail the read");
        let value = serde_json::to_value(response).unwrap();
        let snapshot = &value["ClientCatalogSnapshot"]["snapshot"];
        let labels: Vec<_> = snapshot["provider_accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["label"].as_str().unwrap().to_string())
            .collect();
        assert!(labels.contains(&"after".to_string()), "{labels:?}");
        assert!(!labels.contains(&"before".to_string()), "{labels:?}");
    });
}
