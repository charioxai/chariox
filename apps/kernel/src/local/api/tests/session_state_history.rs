//! MP-08 / MP-09 / MP-10 / MP-11 KR-06: a state read of a Room, and a turn of its agent, cost the
//! same after thousands of finished turns, in it and in other Rooms, as after a few.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

const SAMPLES: usize = 7;

struct Room {
    session: String,
    agent: String,
    attachment: String,
}

fn kr06_harness() -> LocalRouterTestHarness {
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    LocalRouterTestHarness::with_config(config)
}

fn fastest(mut sample: impl FnMut() -> Duration) -> Duration {
    (0..SAMPLES).map(|_| sample()).min().unwrap()
}

fn state_read(harness: &LocalRouterTestHarness, room: &Room) -> Duration {
    let started = Instant::now();
    let response = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: room.session.clone(),
            },
        ))
        .unwrap();
    let elapsed = started.elapsed();
    assert!(matches!(response, LocalDaemonResponse::SessionState { .. }));
    elapsed
}

/// One turn as the KR-06 load drives it: the client submits a prompt and then completes it.
fn turn(harness: &LocalRouterTestHarness, room: &Room) -> (Duration, AgentTaskExecution) {
    let started = Instant::now();
    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: room.session.clone(),
            attachment_id: room.attachment.clone(),
            target_agent_id: Some(room.agent.clone()),
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
            session_id: room.session.clone(),
        }))
        .unwrap();
    let elapsed = started.elapsed();
    let task = harness
        .with_app(|app| {
            app.durable_state_store()
                .agent_turn_task(&room.session, &room.agent, prompt.id())
        })
        .unwrap()
        .expect("the finished turn keeps its task");
    assert_eq!(task.state, ExecutionState::Done);
    (elapsed, task)
}

fn room(harness: &LocalRouterTestHarness, label: &str) -> Room {
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
    Room {
        session: session.id().into(),
        agent: agent.id().into(),
        attachment: attachment.id().into(),
    }
}

/// Adds `count` finished turns like `task` to its agent, as a long-lived Room accumulates them:
/// each one's terminal outcome is recorded, as it is once a real turn has finished.
fn add_finished_turns(harness: &LocalRouterTestHarness, task: &AgentTaskExecution, count: usize) {
    let path = harness.with_app(|app| app.durable_state_store().path().to_path_buf());
    let mut db = rusqlite::Connection::open(path).unwrap();
    let tx = db.transaction().unwrap();
    for turn in 0..count {
        let mut finished = task.clone();
        finished.task_id = format!("{}-history-{turn}", task.task_id);
        finished.prompt_id = format!("{}-history-{turn}", task.prompt_id);
        tx.execute(
            "INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload,terminal_outcome_recorded) \
             VALUES(?1,?2,?3,?4,?5,1)",
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

fn assert_flat(what: &str, young: Duration, aged: Duration) {
    eprintln!(
        "KR-06 {what} fastest of {SAMPLES}: young {:.2} ms, after 24000 finished turns {:.2} ms",
        young.as_secs_f64() * 1000.0,
        aged.as_secs_f64() * 1000.0
    );
    assert!(
        aged <= young * 2 + Duration::from_millis(5),
        "{what} took {young:?} in a young Room and {aged:?} after 24000 finished turns"
    );
}

#[test]
fn kr06_session_state_read_does_not_grow_with_finished_turns() {
    let harness = kr06_harness();
    let probed = room(&harness, "kr06-probed-room");
    let other = room(&harness, "kr06-other-room");
    let (_, probed_task) = turn(&harness, &probed);
    let (_, other_task) = turn(&harness, &other);
    let young = fastest(|| state_read(&harness, &probed));

    // Two hours of the KR-06 load: a turn every three seconds in the probed Room, nine more busy Rooms.
    add_finished_turns(&harness, &probed_task, 2_400);
    add_finished_turns(&harness, &other_task, 21_600);
    let aged = fastest(|| state_read(&harness, &probed));
    assert_flat("GetSessionState", young, aged);
}

#[test]
fn kr06_turn_does_not_grow_with_finished_turns() {
    let harness = kr06_harness();
    let busy = room(&harness, "kr06-busy-room");
    let other = room(&harness, "kr06-quiet-room");
    let (_, busy_task) = turn(&harness, &busy);
    let (_, other_task) = turn(&harness, &other);
    let young = fastest(|| turn(&harness, &busy).0);

    // Each turn holds the reader every state read waits for; it must not read the agent's history.
    add_finished_turns(&harness, &busy_task, 2_400);
    add_finished_turns(&harness, &other_task, 21_600);
    let aged = fastest(|| turn(&harness, &busy).0);
    assert_flat("SubmitPrompt and CompletePrompt", young, aged);
}
