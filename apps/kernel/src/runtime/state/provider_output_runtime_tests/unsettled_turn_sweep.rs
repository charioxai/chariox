use super::*;

// MP-08 / MP-10 / MP-11 (amfix item 1): whatever lost a turn end, an answered
// turn whose prompt already settled is assessed by the sweep, not blocked.
#[tokio::test]
async fn sweep_assesses_an_answered_turn_whose_end_never_reached_the_ledger() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    let worktree = crate::test_support::TestWorktree::new("amfix-sweep-answered-turn");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "amfix-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .submit_prompt(session.id(), attachment.id(), Some(agent.id()), "work", Vec::new())
        .unwrap()
    else {
        panic!("prompt should start")
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let store = &runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: prompt.id().into(),
            run: Some(run.id().into()),
            now: 1,
        })
        .unwrap();
    let answer = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        run.id(),
        Some(agent.id()),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        "The requested work is finished.",
    );
    for entry in [answer] {
        runtime
            .owned
            .operational_history_store
            .append_transcript(&entry, Default::default())
            .unwrap();
    }
    // The prompt settles without the ledger ever seeing the turn end.
    runtime
        .owned
        .complete_local_prompt_without_advance(session.id(), agent.id(), Some(run.id()))
        .unwrap();
    runtime.sweep_agent_lifecycle().await.unwrap();
    let task = store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap()
        .into_iter()
        .find(|t| t.prompt_id == prompt.id())
        .unwrap();
    assert!(
        !task.reason.contains("No live provider turn"),
        "an answered turn must be assessed, not blocked: {:?} {}",
        task.state,
        task.reason
    );
    assert_ne!(task.state, ExecutionState::Working);
}
