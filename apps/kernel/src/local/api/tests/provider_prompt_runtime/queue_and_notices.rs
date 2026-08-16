use super::*;

#[test]
fn local_request_api_exposes_queue_config_and_notices() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let a = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let b = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-b".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");
    });

    let first = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: a.id().to_string(),
            target_agent_id: None,
            prompt: "first".to_string(),
            attachments: Vec::new(),
            prompt_source: None,
        }))
        .expect("first prompt should start");
    let second = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: b.id().to_string(),
            target_agent_id: None,
            prompt: "second".to_string(),
            attachments: Vec::new(),
            prompt_source: None,
        }))
        .expect("second prompt should queue");
    let config = harness
        .dispatch(LocalDaemonRequest::UpdateSessionConfig(
            UpdateSessionConfigRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            },
        ))
        .expect("config update should succeed");

    match first {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            session,
            ..
        } => {
            assert!(session.active_prompt().is_some());
        }
        _ => panic!("unexpected first prompt response"),
    }
    let queued_prompt = match second {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { prompt },
            session,
            ..
        } => {
            assert_eq!(session.queued_prompts().len(), 1);
            assert!(prompt.id().starts_with("pending-prompt-"));
            assert_eq!(prompt.pending_prompt_id(), Some(prompt.id()));
            prompt
        }
        _ => panic!("unexpected second prompt response"),
    };
    match config {
        LocalDaemonResponse::SessionConfigUpdated { config, session } => {
            assert_eq!(config.version(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected config response"),
    }

    let queued_notices = harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
            },
        ))
        .expect("active attachment notice polling should succeed");
    match queued_notices {
        LocalDaemonResponse::RuntimeNotices { notices } => {
            assert!(
                notices.iter().any(|notice| {
                    notice.message.contains("queued prompt")
                        && notice.message.contains(queued_prompt.id())
                }),
                "active attachment should be notified when another attachment queues a prompt: {notices:?}"
            );
        }
        _ => panic!("unexpected notices response"),
    }

    let notices = harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session.id().to_string(),
                attachment_id: b.id().to_string(),
            },
        ))
        .expect("notice polling should succeed");
    match notices {
        LocalDaemonResponse::RuntimeNotices { notices } => assert!(!notices.is_empty()),
        _ => panic!("unexpected notices response"),
    }

    let state = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed");
    match state {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.queued_prompts().len(), 1);
            assert_eq!(session.queued_prompts()[0].id(), queued_prompt.id());
            assert_eq!(
                session.queued_prompts()[0].pending_prompt_id(),
                Some(queued_prompt.id())
            );
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected state response"),
    }
    let mut history_before_promotion = Vec::new();
    for _ in 0..100 {
        history_before_promotion = harness
            .with_app(|app| app.load_session_history_entries(&session, Some(agent.id())))
            .expect("history should load before queued prompt promotion");
        if history_before_promotion.iter().any(|entry| {
            entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                && entry.text.contains("first")
        }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        history_before_promotion.iter().any(|entry| {
            entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                && entry.text.contains("first")
        }),
        "started prompt should be in history"
    );
    assert!(
        history_before_promotion.iter().all(|entry| {
            !(entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                && entry.text.contains("second"))
        }),
        "queued prompt must not enter history before promotion"
    );

    let completed = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("complete prompt should succeed");
    let promoted_prompt_id = match completed {
        LocalDaemonResponse::PromptCompleted { completion } => {
            let started_next = completion
                .started_next
                .expect("queued prompt should start after completion");
            assert_eq!(started_next.prompt(), "second");
            assert_ne!(started_next.id(), queued_prompt.id());
            assert_eq!(started_next.pending_prompt_id(), None);
            started_next.id().to_string()
        }
        _ => panic!("unexpected completion response"),
    };
    let expected_merge_key = format!("prompt:{promoted_prompt_id}");
    let mut history_after_promotion = Vec::new();
    for _ in 0..100 {
        history_after_promotion = harness
            .with_app(|app| app.load_session_history_entries(&session, Some(agent.id())))
            .expect("history should load after queued prompt promotion");
        if history_after_promotion.iter().any(|entry| {
            entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                && entry.text.contains("second")
                && entry.merge_key.as_deref() == Some(expected_merge_key.as_str())
        }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        history_after_promotion
            .iter()
            .filter(|entry| {
                entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                    && entry.text.contains("second")
                    && entry.merge_key.as_deref() == Some(expected_merge_key.as_str())
            })
            .count(),
        1,
        "promoted queued prompt should enter history exactly once under its real prompt id"
    );
}

#[test]
fn local_request_api_can_cancel_an_active_prompt() {
    let harness = LocalRouterTestHarness::new();

    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");

    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "first prompt\n".to_string(),
            attachments: Vec::new(),
            prompt_source: None,
        }))
        .expect("first prompt should start");
    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "second prompt\n".to_string(),
            attachments: Vec::new(),
            prompt_source: None,
        }))
        .expect("second prompt should queue");

    let response = harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: None,
            },
        ))
        .expect("cancel should succeed");

    match response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(
                cancellation.prompt.status(),
                crate::session::PromptStatus::Cancelling
            );
            assert!(cancellation.started_next.is_none());
        }
        _ => panic!("unexpected local response"),
    }
}
