//! Row 316: a new slice that names another machine as its worker is refused before any
//! record exists, so a refused spawn leaves nothing behind that blocks a later plain spawn.
use super::*;

fn hosted_config() -> DaemonConfig {
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some("wss://relay.example.test".to_string());
    config.relay_token = Some("shared-token".to_string());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "kernel-owner".into(),
        realm_id: "test-realm".into(),
        relay_url: "wss://relay.example.test".into(),
        machine_id: Some(config.host_machine_id.clone()),
        ..Default::default()
    });
    config
}

fn create_request(name: &str, worker: Option<&str>) -> LocalDaemonRequest {
    LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        source_slice_ref: None,
        name: name.to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headed,
        display_backend: Default::default(),
        workspace_id: Some("workspace".to_string()),
        worktree_id: Some("worktree".to_string()),
        workspace_mount: Some("worktree".to_string()),
        development: None,
        worker_kernel_ref: worker.map(str::to_string),
        display_url: None,
        provider_auth: Vec::new(),
        from_saved_state: None,
        base: None,
    })
}

async fn create(
    router: &CommandRouter,
    name: &str,
    worker: Option<&str>,
) -> Result<LocalDaemonResponse, crate::error::DaemonError> {
    let request = create_request(name, worker);
    let command = KernelCommand::from_local_request(&format!("cmd-{name}"), None, None, &request);
    router.dispatch(command, request).await
}

// S1 + S2 + S3
#[tokio::test]
async fn a_refused_hosted_slice_with_another_machine_as_worker_leaves_no_record() {
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(hosted_config()).unwrap()));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let before = app.lock().await.slices().list().len();

    let error = create(&router, "s0-slice-aaaaa", Some("kernel-05ac923c384c5ea3"))
        .await
        .expect_err("a hosted slice cannot take another machine as its worker");

    let text = error.to_string();
    for internal in [
        "slice.relay_identity",
        "slice:",
        "worker reference",
        "machine-scoped",
    ] {
        assert!(!text.contains(internal), "{internal} leaks into: {text}");
    }
    assert!(
        text.contains("without --slice"),
        "names the next step: {text}"
    );
    let slices = app.lock().await.slices();
    assert_eq!(
        slices.list().len(),
        before,
        "refusal must not create a record"
    );
    assert!(slices
        .resolve_by_worker_kernel_ref("kernel-05ac923c384c5ea3")
        .is_none());
    let events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .unwrap();
    assert!(
        !events.iter().any(|event| event.kind == "slice.created"),
        "no slice.created event"
    );
}

// S4: a hosted slice without a named worker is created as before.
#[tokio::test]
async fn a_hosted_slice_without_a_named_worker_is_still_created() {
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(hosted_config()).unwrap()));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let response = create(&router, "plain-slice", None).await.unwrap();
    let LocalDaemonResponse::SliceCreated { slice } = response else {
        panic!("unexpected response");
    };
    assert!(crate::slice::machine_scoped_slice_worker_ref(
        &slice.worker_kernel_ref,
        &slice.owner_machine_id
    ));
}

// S4: a private (non-Cloud) home keeps its explicit worker reference.
#[tokio::test]
async fn a_self_hosted_slice_keeps_its_named_worker() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap(),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let response = create(&router, "self-hosted", Some("self-hosted-worker"))
        .await
        .unwrap();
    let LocalDaemonResponse::SliceCreated { slice } = response else {
        panic!("unexpected response");
    };
    assert_eq!(slice.worker_kernel_ref, "self-hosted-worker");
}
