//! Cloud enrollment re-owns the App records of the local owner, so an App
//! automation set up before enrollment still starts its workflow afterwards.
use super::*;

const ENROLLED: &str = "enrolled-person";

#[test]
fn an_app_automation_set_up_before_enrollment_runs_its_workflow_for_the_enrolled_owner() {
    crate::test_support::isolated_env_test!();
    on_runtime(exercise);
}

async fn exercise() {
    let root = std::env::temp_dir().join(format!("chariox-evval-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(root.clone());
    let services = Services::new(&root).await;
    let config = test_config(&root, &services);
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let control = app.app_control_service();
    let cleanup = WorkerCleanup(control.lifecycle().clone());
    let _observations = control.lifecycle().fixture_inbox_automation_workers();
    let store = app.durable_state_store();
    install_fixture_app(&store);
    let sessions = vec![workflow(&mut app, &root.join("workspace/first"), "first")];
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    configure(&state, &sessions[0].0, &sessions[0].1, "first");
    record_fixture_connection(&state, &config);
    assert!(matches!(
        route(&router, "first", json!({"event.kind":"wanted"}))
            .await
            .unwrap(),
        LocalDaemonResponse::AppInboxRoutes { .. }
    ));
    assert_eq!(counts_for(&state, "local"), (0, 0, 0));
    control.lifecycle().shutdown_blocking().unwrap();
    drop(state);
    drop(router);
    drop(cleanup);
    drop(control);
    drop(store);

    // The account is enrolled while the kernel is down; startup adopts the work.
    let mut enrolled = config.clone();
    enrolled
        .persist_cloud_relay_profile(Some(crate::config::PersistedCloudRelayProfile {
            user_id: ENROLLED.into(),
            realm_id: "realm".into(),
            kernel_id: Some(enrolled.daemon_id.clone()),
            ..Default::default()
        }))
        .unwrap();
    let app = DaemonApp::bootstrap(enrolled.clone()).unwrap();
    let control = app.app_control_service();
    let cleanup = WorkerCleanup(control.lifecycle().clone());
    let observations = control.lifecycle().fixture_inbox_automation_workers();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    record_fixture_connection(&state, &enrolled);
    let kernel_id =
        crate::transport::gateway_identity::GatewayIdentity::from_config(&enrolled).kernel_id();
    let resumes = state
        .event_delivery_resumes(&kernel_id, &enrolled.event_delivery_environment_id)
        .unwrap();
    let claims = &resumes[0].routes;
    assert_eq!(claims.len(), 1, "the route is still listed for the owner");
    let deliveries: Vec<_> = claims
        .iter()
        .map(|claim| {
            envelope(
                &claim.binding_id,
                "after-enrollment",
                crate::session::unix_epoch_ms(),
            )
        })
        .collect();
    let held_wake = state
        .app_control()
        .wake_pump()
        .try_begin(crate::session::unix_epoch_ms())
        .unwrap();
    let connection = connector(&state, &enrolled, &services).await;
    assert_eq!(
        services
            .generate(deliveries, &json!({"event":{"kind":"wanted"}}), true)
            .await,
        1
    );
    assert_eq!(counts_for(&state, ENROLLED), (1, 0, 0));
    assert_eq!(counts_for(&state, "local"), (0, 0, 0));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while counts_for(&state, ENROLLED).0 > 0 {
        state.app_inbox_pass(crate::session::unix_epoch_ms()).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "App inbox must drain"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(counts_for(&state, ENROLLED), (0, 1, 0));
    let owned = state.clone();
    tokio::task::spawn_blocking(move || owned.fixture_app_event_pass())
        .await
        .unwrap();
    assert_eq!(
        runs(&state, &sessions).len(),
        1,
        "exactly one workflow run starts under the enrolled owner"
    );

    connection.shutdown().await;
    drop(held_wake);
    control.lifecycle().shutdown_blocking().unwrap();
    drop(state);
    drop(router);
    drop(cleanup);
    drop(control);
    services.stop().await;
    assert!(observations
        .lock()
        .unwrap()
        .iter()
        .all(|o| o.was_reaped() && o.lease_was_dropped()));
    drop(observations);
    drop(scratch);
    assert!(!root.exists(), "all owned state removed");
}
