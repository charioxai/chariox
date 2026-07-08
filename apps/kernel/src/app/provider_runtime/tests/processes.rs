use super::super::*;
use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::{
    DaemonConfig, UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialSourceConfig,
    UserCredentialUse,
};
use crate::provider::{LaunchProviderRequest, ProviderRunState};
use crate::session::{
    CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
};

#[test]
fn provider_processes_list_and_teardown_safe_idle_managed_runs() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");

    let processes = app
        .list_provider_processes(None)
        .expect("provider processes should list");
    assert_eq!(processes.len(), 1);
    assert!(processes[0].teardown_safe);
    assert!(processes[0].attached_session_ids.is_empty());
    assert_eq!(
        processes[0].owner_provider_run_ids,
        vec![run.id().to_string()]
    );
    assert_eq!(app.provider_process_tracking.snapshot().processes.len(), 1);
    assert_eq!(
        app.provider_process_tracking.snapshot().run_processes.len(),
        1
    );
    assert_eq!(
        processes[0].pid,
        app.pty
            .process_id(run.id())
            .expect("pty pid should resolve")
    );

    let torn_down = app
        .teardown_provider_processes(None, false)
        .expect("safe teardown should succeed");
    assert_eq!(torn_down.len(), 1);
    assert!(app
        .list_provider_processes(None)
        .expect("provider processes should relist")
        .is_empty());
    assert!(app
        .provider_process_tracking
        .snapshot()
        .processes
        .is_empty());
    assert!(app
        .provider_process_tracking
        .snapshot()
        .run_processes
        .is_empty());
}

#[test]
fn provider_process_gc_reaps_idle_managed_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");
    let pid = app
        .pty
        .process_id(run.id())
        .expect("pty pid should resolve")
        .expect("managed run should have pid");

    let summary = app
        .reap_idle_provider_processes(crate::session::unix_epoch_ms(), 0, u64::MAX)
        .expect("provider process gc should succeed");

    assert_eq!(summary.tracked_processes_reaped, 1);
    assert!(app
        .provider_process_tracking
        .snapshot()
        .processes
        .is_empty());
    assert!(app
        .provider_process_tracking
        .snapshot()
        .run_processes
        .is_empty());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should still be stored")
            .state(),
        ProviderRunState::Ended,
    );
    assert!(!crate::runtime::process_health::process_running(pid));
}

#[test]
fn provider_process_gc_keeps_attached_managed_run() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let _attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");
    let pid = app
        .pty
        .process_id(run.id())
        .expect("pty pid should resolve")
        .expect("managed run should have pid");

    let summary = app
        .reap_idle_provider_processes(crate::session::unix_epoch_ms(), 0, u64::MAX)
        .expect("provider process gc should succeed");

    assert_eq!(summary.tracked_processes_reaped, 0);
    assert_eq!(app.provider_process_tracking.snapshot().processes.len(), 1);
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should still be stored")
            .state(),
        ProviderRunState::Running,
    );
    assert!(crate::runtime::process_health::process_running(pid));

    app.teardown_provider_processes(None, true)
        .expect("cleanup should succeed");
}

#[test]
fn provider_processes_do_not_teardown_with_per_agent_active_prompt() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider launch should succeed");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "active prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");
    app.detach(attachment.id())
        .expect("detaching should leave active prompt state");

    let processes = app
        .list_provider_processes(None)
        .expect("provider processes should list");
    assert_eq!(processes.len(), 1);
    assert!(!processes[0].teardown_safe);
    assert!(processes[0].attached_session_ids.is_empty());
    assert_eq!(processes[0].teardown_blockers, vec!["active prompt"]);

    let torn_down = app
        .teardown_provider_processes(None, false)
        .expect("safe teardown should succeed");
    assert!(torn_down.is_empty());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should still exist")
            .state(),
        ProviderRunState::Running,
    );
}

#[test]
fn detaching_attachment_with_queued_prompts_preserves_queue() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    app.launch_provider(
        LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "sonnet")
            .with_agent_id(agent.id()),
    )
    .expect("provider launch should succeed");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "active prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");
    let queued = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "queued prompt\n",
            Vec::new(),
        )
        .expect("second prompt should queue");
    let queued_prompt_id = match queued {
        PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        other => panic!("expected queued prompt, got {other:?}"),
    };

    crate::app::KernelSessionService::new(&mut app)
        .detach(attachment.id())
        .expect("detaching should preserve queued prompt state");

    let session_after_detach = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain available");
    assert_eq!(session_after_detach.queued_prompts().len(), 1);
    assert_eq!(
        session_after_detach.queued_prompts()[0].id(),
        queued_prompt_id
    );
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
            .expect("prompt owner should remain available"),
        1,
    );
}

#[test]
fn queued_prompt_promotes_after_source_attachment_reconnects() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let original_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider launch should succeed");
    app.submit_prompt(
        session.id(),
        original_attachment.id(),
        Some(agent.id()),
        "active prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");
    let queued = app
        .submit_prompt(
            session.id(),
            original_attachment.id(),
            Some(agent.id()),
            "queued prompt\n",
            Vec::new(),
        )
        .expect("second prompt should queue");
    assert!(
        matches!(queued, PromptSubmissionOutcome::Queued { .. }),
        "second prompt should queue: {queued:?}"
    );

    crate::app::KernelSessionService::new(&mut app)
        .detach(original_attachment.id())
        .expect("detaching should preserve queued prompt state");
    let replacement_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-2",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("replacement client should attach");

    let completion = app
        .complete_active_prompt(session.id(), agent.id(), Some(run.id()))
        .expect("queued prompt should promote through the replacement attachment");
    let started_next = completion
        .started_next
        .expect("queued prompt should become active");
    assert_eq!(started_next.prompt(), "queued prompt\n");

    let input_records = app.terminal().input_records();
    assert!(
        input_records.iter().any(|record| {
            record.source_attachment_id == replacement_attachment.id()
                && String::from_utf8_lossy(&record.bytes).contains("queued prompt")
        }),
        "promoted queued prompt should be delivered through the replacement attachment: {input_records:?}"
    );
}

#[test]
fn detaching_last_attachment_keeps_background_active_prompt_run_running() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, focused_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let background_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("background"))
        .expect("background agent should spawn");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let background_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(background_agent.id()),
        )
        .expect("background provider launch should succeed");
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        PromptQueueItem::new(
            "prompt-background-active",
            attachment.id(),
            background_agent.id(),
            "background active prompt",
            PromptStatus::Queued,
        ),
        false,
    )
    .expect("background prompt should start");
    app.focus_agent(session.id(), focused_agent.id())
        .expect("focused agent should be restored");
    app.prompt_owner_submit_prepared_prompt(
        session.id(),
        PromptQueueItem::new(
            "prompt-focused-queued",
            attachment.id(),
            focused_agent.id(),
            "focused queued prompt",
            PromptStatus::Queued,
        ),
        true,
    )
    .expect("focused prompt should queue");
    app.sessions
        .set_active_provider_run(session.id(), Some(background_run.id().to_string()))
        .expect("background run should be active");
    let projected_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(
        projected_session.active_prompt().is_none(),
        "legacy focused prompt projection intentionally has no active prompt"
    );
    assert!(
        projected_session.has_any_active_prompt(),
        "per-agent prompt state still has active work"
    );
    assert!(
        app.prompt_owner_has_any_active_prompt(session.id())
            .expect("prompt owner should read session"),
        "prompt owner should still have background active work before detach"
    );

    app.detach(attachment.id())
        .expect("detaching should leave background active prompt state");

    assert_eq!(
        app.providers()
            .get_run(background_run.id())
            .expect("background run should remain")
            .state(),
        ProviderRunState::Running
    );
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should remain")
            .active_provider_run_id(),
        Some(background_run.id())
    );
}

#[test]
fn provider_process_projection_invalidates_after_app_prompt_owner_change() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    app.launch_provider(
        LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "sonnet")
            .with_agent_id(agent.id()),
    )
    .expect("provider launch should succeed");
    app.list_provider_processes(None)
        .expect("provider processes should warm projection");
    assert!(app.provider_process_projection_store().list(None).is_some());

    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "active prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");

    assert!(
        app.provider_process_projection_store().list(None).is_none(),
        "app-level prompt-owner changes should invalidate provider-process projection"
    );
}

#[test]
fn provider_launch_runtime_profile_survives_kernel_restart() {
    let config = DaemonConfig::for_tests();
    let (agent_id, run_model) = {
        let mut app =
            DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session create should succeed");
        let run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider launch should succeed");
        (agent.id().to_string(), run.model().to_string())
    };

    let app = DaemonApp::bootstrap(config).expect("daemon bootstrap after restart should succeed");
    let restored_agent = app
        .agents
        .get_agent(&agent_id)
        .expect("agent should restore");
    assert_eq!(restored_agent.provider(), "claude-code");
    assert_eq!(restored_agent.model(), Some(run_model.as_str()));
}

#[test]
fn provider_launch_scrubs_configured_credential_env_names() {
    let _guard = crate::env_lock::lock();
    let old_home = std::env::var_os("HOME");
    let temp_home = std::env::temp_dir().join("arroba-provider-env-credential-test");
    let _ = std::fs::remove_dir_all(&temp_home);
    std::env::set_var("HOME", &temp_home);
    let source = temp_home.join("credential.yaml");
    std::fs::create_dir_all(&temp_home).unwrap();
    let registry = crate::credential::ArrobaCredentialRegistry::user().unwrap();
    let credential = UserCredentialConfig {
        id: "github".to_string(),
        description: None,
        source: UserCredentialSourceConfig::Env {
            name: "ARROBA_TEST_GH_TOKEN".to_string(),
        },
        allowed_hosts: Vec::new(),
        allowed_uses: vec![UserCredentialUse::Http],
        injection: UserCredentialInjectionConfig::Header {
            name: "authorization".to_string(),
            value: "Bearer ${secret}".to_string(),
        },
        metadata: None,
    };
    std::fs::write(&source, serde_yaml::to_string(&credential).unwrap()).unwrap();
    registry.install_from_file(&source).unwrap();
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");

    assert!(run
        .pty_env_remove()
        .contains(&"ARROBA_TEST_GH_TOKEN".to_string()));
    match old_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    let _ = std::fs::remove_dir_all(&temp_home);
}

#[test]
fn provider_processes_do_not_teardown_when_session_is_attached() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let _attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");

    let processes = app
        .list_provider_processes(None)
        .expect("provider processes should list");
    assert_eq!(processes.len(), 1);
    assert!(!processes[0].teardown_safe);
    assert_eq!(
        processes[0].attached_session_ids,
        vec![session.id().to_string()]
    );
    assert_eq!(
        processes[0].teardown_blockers,
        vec![format!("attached sessions: {}", session.id())]
    );

    let torn_down = app
        .teardown_provider_processes(None, false)
        .expect("safe teardown should succeed");
    assert!(torn_down.is_empty());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should still exist")
            .state(),
        crate::provider::ProviderRunState::Running,
    );

    let torn_down = app
        .teardown_provider_processes(None, true)
        .expect("forced teardown should succeed without active prompts");
    assert_eq!(torn_down.len(), 1);
    assert!(app
        .list_provider_processes(None)
        .expect("provider processes should relist")
        .is_empty());
}

#[test]
fn ending_session_clears_tracked_provider_processes() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session create should succeed");
    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");

    assert!(app
        .provider_process_tracking
        .snapshot()
        .processes
        .values()
        .any(|process| { process.owner_provider_run_ids == vec![run.id().to_string()] }));

    let _ = crate::app::KernelSessionService::new(&mut app)
        .end_session(session.id())
        .expect("session should end");

    assert!(app
        .provider_process_tracking
        .snapshot()
        .processes
        .is_empty());
    assert!(app
        .provider_process_tracking
        .snapshot()
        .run_processes
        .is_empty());
    assert!(app
        .list_provider_processes(None)
        .expect("provider processes should list")
        .is_empty());
}
