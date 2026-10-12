//! MP-08 / MP-10 / MP-11: a rejected hook must never leave a phantom active turn.
use super::*;

#[test]
fn expired_native_context_request_never_starts_a_prompt() {
    context_failure_case(true);
}

#[test]
fn failed_native_context_delivery_settles_only_its_bound_prompt() {
    context_failure_case(false);
}

fn context_failure_case(expired: bool) {
    let worktree = crate::test_support::TestWorktree::new("native-context-failure");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "context-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let context = worktree.path().join("context.txt");
    let events = worktree.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    fs::create_dir(worktree.path().join("hook-context-responses")).unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "context-failure-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-context-test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                    context.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                    events.display().to_string(),
                ),
                ("CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS".into(), "1".into()),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    let submit = serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"retry audit",
        "hook_context_request_id":"failed-request", "hook_context_deadline_ms":
            if expired { unix_epoch_ms().saturating_sub(1) } else { unix_epoch_ms() + 5000 }});
    fs::write(&events, submit.to_string()).unwrap();
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    let active = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .unwrap();
    if expired {
        assert!(active.is_none(), "expired hook manufactured a WORKING turn");
        return;
    }
    assert!(active.is_some());
    // A stale failure must not fail this current native turn.
    fs::write(
        &events,
        serde_json::json!({"hook_event_name":"CharioxContextFailed",
        "hook_context_request_id":"stale-request"})
        .to_string(),
    )
    .unwrap();
    let stale = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    assert!(stale.terminal_failure.is_none());
    fs::write(
        &events,
        serde_json::json!({"hook_event_name":"CharioxContextFailed",
        "hook_context_request_id":"failed-request"})
        .to_string(),
    )
    .unwrap();
    let failure = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    assert!(
        !failure.rejected_prompt_ids.is_empty() && failure.terminal_failure.is_none(),
        "hook rejection was ignored; native turn remains WORKING"
    );
}
