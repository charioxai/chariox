//! MP-08 / MP-10 / MP-11: a rejected hook must never leave a phantom active turn.
use super::*;

#[test]
fn expired_native_context_request_never_starts_a_prompt() {
    context_failure_case(ContextFailureScenario::ExpiredNative);
}

#[test]
fn failed_native_context_delivery_settles_only_its_bound_prompt() {
    context_failure_case(ContextFailureScenario::FailedNative);
}

#[test]
fn expired_managed_context_request_keeps_a_matching_failure_binding() {
    context_failure_case(ContextFailureScenario::ExpiredManaged);
}

#[derive(Clone, Copy)]
enum ContextFailureScenario {
    ExpiredNative,
    FailedNative,
    ExpiredManaged,
}

fn context_failure_case(scenario: ContextFailureScenario) {
    let expired = !matches!(scenario, ContextFailureScenario::FailedNative);
    let managed = matches!(scenario, ContextFailureScenario::ExpiredManaged);
    let worktree = crate::test_support::TestWorktree::new("native-context-failure");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
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
    if managed {
        let active = match app
            .record_native_prompt_started_with_attachments(
                session.id(),
                attachment.id(),
                attachment.id(),
                agent.id(),
                "retry audit",
                Vec::new(),
            )
            .unwrap()
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
            other => panic!("unexpected managed admission: {other:?}"),
        };
        let path = context.display().to_string();
        write_claude_native_marker(
            &path,
            &format!("submit-wait:{}:{}", active.id(), unix_epoch_ms()),
        );
        write_claude_headless_submit_retry(&path, active.id(), 0, unix_epoch_ms(), "retry audit");
    }
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
    if expired && !managed {
        assert!(active.is_none(), "expired hook manufactured a WORKING turn");
        return;
    }
    assert!(active.is_some());
    assert!(
        context.with_extension("txt.context-delivery").exists(),
        "expired managed hook lost the binding needed to settle its already admitted prompt"
    );
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
    assert!(
        context.with_extension("txt.context-delivery").exists(),
        "stale receipt discarded the active delivery"
    );
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
        !context.with_extension("txt.context-delivery").exists()
            && failure.terminal_failure.is_none(),
        "matched hook rejection must remove its pending delivery without failing the provider"
    );
}

#[test]
fn managed_prompt_matching_accepts_the_official_paste_wrapper_only() {
    let expected = "Review native handoff.\nPreserve the accepted decision.";
    let wrapped =
        format!("<pasted_content id=\"5dad\">\n{expected}\n</pasted_content id=\"5dad\">");
    assert!(claude_headless_prompt_matches(expected, &wrapped));
    let inline = format!("<pasted_content id=\"5dad\"> {expected} </pasted_content id=\"5dad\">");
    assert!(claude_headless_prompt_matches(expected, &inline));
    assert!(claude_headless_prompt_matches(
        expected,
        &inline.replace('\n', "\r\n")
    ));
    for invalid in [
        inline.replace(
            "</pasted_content id=\"5dad\">",
            "</pasted_content id=\"other\">",
        ),
        format!("{inline} extra instructions"),
        format!("extra instructions {inline}"),
        format!("<pasted_content id=\"5dad\">changed {expected}</pasted_content id=\"5dad\">"),
        format!("<pasted_content id=\"\">{expected}</pasted_content id=\"\">"),
        format!("<pasted_content id=\"bad/id\">{expected}</pasted_content id=\"bad/id\">"),
    ] {
        assert!(!claude_headless_prompt_matches(expected, &invalid));
    }
    assert!(claude_headless_prompt_matches(
        expected,
        &wrapped.replace('\n', "\r\n")
    ));
    assert!(!claude_headless_prompt_matches(
        "different prompt",
        &wrapped
    ));
    assert!(!claude_headless_prompt_matches(
        expected,
        &wrapped.replace(
            "</pasted_content id=\"5dad\">",
            "</pasted_content id=\"other\">"
        )
    ));
    assert!(!claude_headless_prompt_matches(
        expected,
        &format!("{wrapped}\nextra instructions")
    ));
    assert!(claude_headless_prompt_matches(&wrapped, &wrapped));
}
