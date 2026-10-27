use crate::slice::hosted_worker_test_support::*;

async fn assert_browser_dispatch(
    machine: &str,
    worker: &str,
    slice: Option<&str>,
    expected: Option<&str>,
) {
    let environment = Environment::new(machine, worker, slice);
    let script = environment.root.join("public-screen-fixture.sh");
    let marker = environment.root.join("screen-called");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf called > '{}'\nprintf '{{\"physical_fixture\":true}}\\n'\n",
            marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.daemon_id = worker.into();
    config.daemon_alias = Some("slice:friendly-name".into());
    config.host_machine_id = machine.into();
    let app = crate::DaemonApp::bootstrap(config).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    let run = provider_run("unbound-headless-session", "agent");
    let result = runtime
        .dispatch_slice_runtime_tool_call(
            &run,
            crate::transport::runtime_tools::SLICE_SCREEN_STATUS_TOOL,
            serde_json::json!({}),
        )
        .await;
    if let Some(slice_id) = expected {
        let output = result.expect("physical slice screen dispatch must accept canonical worker");
        assert!(output.ok);
        assert_eq!(output.payload["slice_id"], slice_id);
        assert!(marker.exists(), "physical screen fixture was reached");
        assert!(!runtime
            .slice_tool_specs_for_provider_runs(&[run])
            .is_empty());
    } else {
        assert!(
            result.is_err(),
            "ordinary/foreign worker must not get slice fallback"
        );
        assert!(!marker.exists(), "no physical browser contact");
        assert!(runtime
            .slice_tool_specs_for_provider_runs(&[run])
            .is_empty());
    }
}

#[tokio::test]
async fn hosted_slice_context_browser_dispatch_accepts_canonical_parent_without_room() {
    assert_browser_dispatch(MACHINE, &canonical_worker(0), Some(SLICE), Some(SLICE)).await;
}
#[tokio::test]
async fn hosted_slice_context_browser_dispatch_preserves_private_synthetic_worker() {
    assert_browser_dispatch("slice:private", "private-worker", None, Some("private")).await;
}
#[tokio::test]
async fn hosted_slice_context_browser_dispatch_does_not_trust_ordinary_friendly_alias() {
    assert_browser_dispatch(MACHINE, "ordinary-worker", Some(SLICE), None).await;
}
#[tokio::test]
async fn hosted_slice_context_browser_dispatch_rejects_wrong_parent() {
    assert_browser_dispatch("machine-other", &canonical_worker(0), Some(SLICE), None).await;
}
