use super::*;

/// The home task context and the final runtime-origin directive must describe
/// the same turn, even though the worker allocates a separate backing prompt.
#[test]
fn leased_managed_context_uses_the_home_runtime_origin() {
    let worktree = crate::test_support::TestWorktree::new("leased-runtime-origin");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "origin-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let context_file = std::env::temp_dir().join("unused-origin-context");
    let run = startup_readiness_run(
        session.id(),
        agent.id(),
        "origin-worker-run",
        &context_file,
        &context_file,
        String::new(),
    );
    let home_origin = "agent-event-home-child-244724";
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".into(),
        home_session_id: "home-room".into(),
        home_agent_id: "home-child".into(),
        home_prompt_id: Some(home_origin.into()),
        leased_agent_id: "origin-lease-agent".into(),
        worker_kernel_id: "worker-kernel".into(),
        worker_machine_id: "worker-machine".into(),
        worker_provider_run_id: run.id().into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(session.worktree_id()),
    };
    let hidden = format!("<chariox-task-context>For chariox.events tools use task_id `spawn-home-child` and origin_prompt_id `{home_origin}`.</chariox-task-context>");
    let prompt = crate::session::PromptQueueItem::new(
        "worker-backing-prompt",
        attachment.id(),
        agent.id(),
        "read metadata",
        crate::session::PromptStatus::Queued,
    )
    .with_leased_skills(Default::default(), Some(context));
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt.clone(), false)
        .unwrap();
    let rendered = ProviderOutputClaudeNativeBridge::new(&mut app)
        .managed_base_context(
            session.id(),
            context_file.to_str().unwrap(),
            &run,
            &ClaudeNativePromptInjection {
                id: prompt.id(),
                origin_prompt_id: prompt.id(),
                prompt: prompt.prompt(),
                hidden_system_context: &hidden,
                attachments: &[],
                context_handoff: &|_| String::new(),
            },
        )
        .unwrap();
    assert!(
        rendered.contains(&crate::prompt_assembly::agent_message_origin_context(
            home_origin
        )),
        "home identity missing from final directive: {rendered}"
    );
    assert!(
        !rendered.contains(&crate::prompt_assembly::agent_message_origin_context(
            prompt.id()
        )),
        "worker backing identity must remain internal"
    );
}
