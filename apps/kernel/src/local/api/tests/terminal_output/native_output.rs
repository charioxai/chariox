use super::*;
use crate::local::PumpTerminalOutputRequest;

#[test]
fn append_native_provider_output_fans_out_and_records_history() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let provider_run_id = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "slow-structured",
                "default",
                "default",
            )
            .with_agent_id(agent.id())
            .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("native provider run should launch")
        .id()
        .to_string()
    });

    let records = match harness
        .dispatch(LocalDaemonRequest::AppendNativeProviderOutput(
            super::AppendNativeProviderOutputRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                provider_run_id: provider_run_id.clone(),
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("native-output".to_string()),
                text: "hello from native tui\n".to_string(),
            },
        ))
        .expect("native provider output should append")
    {
        LocalDaemonResponse::TerminalOutput { records } => records,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bytes, b"hello from native tui\n");
    let history = harness
        .with_app(|app| app.load_session_history_entries(&session, Some(agent.id())))
        .expect("history should load");
    assert!(history.iter().any(|entry| {
        entry.provider_run_id.as_deref() == Some(provider_run_id.as_str())
            && entry.text.contains("hello from native tui")
    }));
}

#[test]
fn stale_terminal_sweep_removes_dead_attachment_before_fanout() {
    std::thread::Builder::new()
        .name("stale-terminal-sweep-test".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(stale_terminal_sweep_removes_dead_attachment_before_fanout_inner)
        .expect("stale terminal sweep test thread should spawn")
        .join()
        .expect("stale terminal sweep test thread should not panic");
}

fn stale_terminal_sweep_removes_dead_attachment_before_fanout_inner() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-stale-terminal", "worktree-stale-terminal"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let stale_attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-stale-terminal".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("stale attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let fresh_attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-fresh-terminal".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("fresh attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let provider_run_id = harness.with_app_mut(|app| {
        app.attachments()
            .record_heartbeat(session.id(), stale_attachment.id(), 1)
            .expect("stale heartbeat should record");
        app.attachments()
            .record_heartbeat(
                session.id(),
                fresh_attachment.id(),
                crate::session::unix_epoch_ms(),
            )
            .expect("fresh heartbeat should record");
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "slow-structured",
                "default",
                "default",
            )
            .with_agent_id(agent.id())
            .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("native provider run should launch")
        .id()
        .to_string()
    });

    harness.pump_transport_runtime();

    let attachment_ids =
        harness.with_app(|app| app.attachments().list_session_attachment_ids(session.id()));
    assert_eq!(attachment_ids, vec![fresh_attachment.id().to_string()]);

    let projected_session = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        projected_session
            .attachment_ids()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![fresh_attachment.id().to_string()]
    );

    let records = match harness
        .dispatch(LocalDaemonRequest::AppendNativeProviderOutput(
            super::AppendNativeProviderOutputRequest {
                session_id: session.id().to_string(),
                attachment_id: fresh_attachment.id().to_string(),
                provider_run_id,
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("post-sweep-output".to_string()),
                text: "hello live terminal\n".to_string(),
            },
        ))
        .expect("native provider output should append")
    {
        LocalDaemonResponse::TerminalOutput { records } => records,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].recipient_attachment_ids,
        vec![fresh_attachment.id().to_string()]
    );
    assert_eq!(
        records[0].pending_recipient_attachment_ids,
        vec![fresh_attachment.id().to_string()]
    );
}

#[test]
fn pump_terminal_output_refreshes_terminal_attachment_heartbeat() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-pump-heartbeat", "worktree-pump-heartbeat"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-pump-heartbeat".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.attachments()
            .record_heartbeat(session.id(), attachment.id(), 1)
            .expect("old heartbeat should record");
    });

    match harness
        .dispatch(LocalDaemonRequest::PumpTerminalOutput(
            PumpTerminalOutputRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("terminal output pump should succeed")
    {
        LocalDaemonResponse::TerminalOutput { .. } => {}
        _ => panic!("unexpected local response"),
    }

    harness.pump_transport_runtime();
    let attachment_ids =
        harness.with_app(|app| app.attachments().list_session_attachment_ids(session.id()));
    assert_eq!(attachment_ids, vec![attachment.id().to_string()]);
}

#[test]
fn poll_runtime_notices_refreshes_terminal_attachment_heartbeat() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-notice-heartbeat", "worktree-notice-heartbeat"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-notice-heartbeat".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.attachments()
            .record_heartbeat(session.id(), attachment.id(), 1)
            .expect("old heartbeat should record");
    });

    match harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("runtime notice poll should succeed")
    {
        LocalDaemonResponse::RuntimeNotices { .. } => {}
        _ => panic!("unexpected local response"),
    }

    harness.pump_transport_runtime();
    let attachment_ids =
        harness.with_app(|app| app.attachments().list_session_attachment_ids(session.id()));
    assert_eq!(attachment_ids, vec![attachment.id().to_string()]);
}
