use super::*;

/// The controller answers a click on an element the page replaced. The worker relays
/// that answer as its own typed refusal: no `transport_error`, no retry, no receipt
/// recovery that would append a second sentence, and no physical input.
pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let state_path = fixture._worker_state.root.join("chromium-state.json");
    let status = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("read the Room browser before the stale-element fault");
    let field = status.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .unwrap()
        .to_string();
    let clicks_before = super::controller_recovery::controller_click_count(&state_path);
    let fault = fixture._worker_state.root.join("stale-element");
    std::fs::write(&fault, b"").unwrap();
    let error = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":field}),
        )
        .await
        .expect_err("a click on a replaced element is refused");
    std::fs::remove_file(&fault).unwrap();
    let DaemonError::RelayTransport {
        code,
        message,
        retryable,
        ..
    } = &error
    else {
        panic!("the refusal keeps its typed relay error: {error:?}");
    };
    assert_eq!(code, "browser_controller_failed", "{error}");
    assert!(!retryable, "an answered refusal is not retryable: {error}");
    assert_eq!(
        message.matches("stale_element_reference").count(),
        1,
        "the controller sentence appears once: {message}"
    );
    assert!(!message.contains("remained unavailable"), "{message}");
    assert_eq!(
        super::controller_recovery::controller_click_count(&state_path),
        clicks_before,
        "a stale element must not dispatch physical input"
    );
}
