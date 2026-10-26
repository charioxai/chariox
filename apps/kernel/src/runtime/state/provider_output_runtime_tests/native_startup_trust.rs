//! MP-08 / MP-10 / MP-11: startup trust settlement runs through the mailbox owner.
use super::*;
use crate::app::{startup_readiness_run, StartupTrustBridge};
use crate::provider::ProviderNativeInteractionBridge;
use std::fs;

#[test]
fn claude_workspace_trust_rejection_settles_only_own_prompt_with_reason() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-trust-rejection");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-startup-trust-rejection-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-startup-trust-rejection",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let (other_session, _other_default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-unrelated-agent",
            root.display().to_string(),
        ))
        .expect("unrelated session should be created");
    let other_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(other_session.id(), "dev-stub")
                .with_alias("unrelated-test-agent")
                .with_model("test-model"),
        )
        .expect("self-contained unrelated test agent should be created");
    let other_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            other_session.id(),
            "client-unrelated-agent",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("unrelated attachment should attach");
    app.record_native_prompt_started_with_attachments(
        other_session.id(),
        other_attachment.id(),
        other_attachment.id(),
        other_agent.id(),
        "unrelated-agent-task",
        Vec::new(),
    )
    .expect("unrelated agent prompt should remain active");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let capture_file = root.join("pty-input.log");
    let trust_frame =
        "Quick safety check\nDo you trust this folder?\n1. No, exit\n2. Yes, I trust this folder";
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "provider-run-startup-trust-rejection",
        &context_file,
        &events_file,
        format!(
            "printf '%s' '{}'; tee {} >/dev/null",
            trust_frame,
            capture_file.display()
        ),
    );
    run.mark_running();
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let _prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "rejected-startup-trust-task",
            Vec::new(),
        )
        .expect("rejected task should become active")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    let bridge = StartupTrustBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());
    app.providers()
        .set_native_interaction_bridge(bridge_ref.clone());
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: run.id().into(),
    };
    let executor = tokio::runtime::Runtime::new().unwrap();
    let runtime = executor.block_on(owned_runtime_state(&app));
    let interaction = bridge.wait_for_interaction_with_pump(|| {
        executor
            .block_on(runtime.pump_owned_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                false,
            ))
            .expect("trust prompt should be projected by the kernel runtime pump");
    });
    assert_eq!(interaction.default_on_timeout(), Some("deny"));
    bridge.resolve_default_no();
    let mut settled = false;
    for _ in 0..500 {
        executor
            .block_on(runtime.pump_owned_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                false,
            ))
            .expect("trust rejection should use the kernel runtime failure path");
        let primary_active = runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .cloned();
        if primary_active.is_none() {
            settled = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        settled,
        "rejected startup trust should settle its own prompt"
    );
    assert!(runtime
        .owned
        .session_snapshot(other_session.id())
        .unwrap()
        .active_prompt_for_agent(other_agent.id())
        .is_some());
    let rejected_run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("rejected provider run should remain inspectable");
    let diagnostic = rejected_run
        .terminal_diagnostic()
        .expect("rejection should preserve a terminal diagnostic");
    assert!(
        diagnostic.contains("workspace trust was denied"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains(root.to_string_lossy().as_ref()));
    assert!(!fs::read_to_string(&capture_file)
        .unwrap_or_default()
        .contains("rejected-startup-trust-task"));
    let _ = executor
        .block_on(app.lock())
        .pty_mut()
        .remove_process(run.id());
    let _ = fs::remove_dir_all(root);
}
