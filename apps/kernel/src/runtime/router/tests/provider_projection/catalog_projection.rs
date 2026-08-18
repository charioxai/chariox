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
    app.provider_catalog_projection_store()
        .update_scoped(&cache_key, catalog);
    app.configure_relay(None, None)
        .expect("relay configure should invalidate provider catalog projection");
    app.invalidate_provider_catalog_projection();

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

    tokio::task::yield_now().await;
    assert!(
        !catalog_task.is_finished(),
        "relay configuration should invalidate warmed provider catalog projection"
    );
    drop(app_guard);
    let _ = catalog_task
        .await
        .expect("catalog task should join after app lock is released");
}
