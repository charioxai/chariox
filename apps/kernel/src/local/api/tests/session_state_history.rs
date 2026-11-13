//! MP-08 / MP-09 / MP-10 / MP-11 KR-06: a state read of a Room costs the same after thousands of
//! finished turns, in it and in other Rooms, as after a few.
use super::*;

const READS: usize = 7;

fn fastest_state_read(harness: &LocalRouterTestHarness, session_id: &str) -> Duration {
    (0..READS)
        .map(|_| {
            let started = Instant::now();
            let response = harness
                .dispatch(LocalDaemonRequest::GetSessionState(
                    GetSessionStateRequest {
                        session_id: session_id.into(),
                    },
                ))
                .unwrap();
            let elapsed = started.elapsed();
            assert!(matches!(response, LocalDaemonResponse::SessionState { .. }));
            elapsed
        })
        .min()
        .unwrap()
}

fn room_with_finished_turn(
    harness: &LocalRouterTestHarness,
    label: &str,
) -> (
    String,
    crate::durable_state::agent_lifecycle::AgentTaskExecution,
) {
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            harness
                .fixture_session_request(label)
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        other => panic!("unexpected response: {other:?}"),
    };
    harness.launch_workflow_test_provider(session.id(), agent.id());
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().into(),
                client_id: label.into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected response: {other:?}"),
    };
    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
            prompt: "one turn".into(),
            attachments: vec![],
        }))
        .unwrap()
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => prompt,
        other => panic!("unexpected response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().into(),
        }))
        .unwrap();
    let task = harness
        .with_app(|app| {
            app.durable_state_store()
                .agent_turn_task(session.id(), agent.id(), prompt.id())
        })
        .unwrap()
        .expect("the finished turn keeps its task");
    assert_eq!(
        task.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Done
    );
    (session.id().to_string(), task)
}

/// Adds `count` finished turns like `task` to its agent, as a long-lived Room accumulates them.
fn add_finished_turns(
    db: &mut rusqlite::Connection,
    task: &crate::durable_state::agent_lifecycle::AgentTaskExecution,
    count: usize,
) {
    let tx = db.transaction().unwrap();
    for turn in 0..count {
        let mut finished = task.clone();
        finished.task_id = format!("{}-history-{turn}", task.task_id);
        finished.prompt_id = format!("{}-history-{turn}", task.prompt_id);
        tx.execute(
            "INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload) VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![
                finished.task_id,
                finished.room_id,
                finished.agent_id,
                finished.prompt_id,
                serde_json::to_string(&finished).unwrap()
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn kr06_session_state_read_does_not_grow_with_finished_turns() {
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let harness = LocalRouterTestHarness::with_config(config);
    let (probed, probed_task) = room_with_finished_turn(&harness, "kr06-probed-room");
    let (_, other_task) = room_with_finished_turn(&harness, "kr06-other-room");
    let young = fastest_state_read(&harness, &probed);

    // Two hours of the KR-06 load: a turn every three seconds in the probed Room, nine more busy Rooms.
    let path = harness.with_app(|app| app.durable_state_store().path().to_path_buf());
    let mut db = rusqlite::Connection::open(path).unwrap();
    add_finished_turns(&mut db, &probed_task, 2_400);
    add_finished_turns(&mut db, &other_task, 21_600);
    let aged = fastest_state_read(&harness, &probed);

    eprintln!(
        "KR-06 GetSessionState fastest of {READS}: young {:.2} ms, after 24000 finished turns {:.2} ms",
        young.as_secs_f64() * 1000.0,
        aged.as_secs_f64() * 1000.0
    );
    assert!(
        aged <= young * 2 + Duration::from_millis(5),
        "a state read took {young:?} in a young Room and {aged:?} after 24000 finished turns"
    );
}
