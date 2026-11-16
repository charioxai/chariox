//! An agent may always end its turn, also with a watched process still running: the task ends
//! Done with no correction and no "Agent needs your action" card, the process stays supervised and
//! its end wakes the agent.
use super::tests::{fixture_with_run_profile, running};
use super::watched_process_grant_tests::begin_task;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
use crate::provider::AgentPermissionLevel;
use std::time::Duration;

#[tokio::test]
async fn a_turn_that_ends_with_a_running_process_is_done_and_the_processs_end_wakes_the_agent() {
    let f = fixture_with_run_profile(None, true, "claude", "claude");
    let turn = running(&f);
    let task = begin_task(&f, &turn, false, AgentPermissionLevel::Yolo).await;
    let started = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.process",
            serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id,
                "label":"build", "argv":["/bin/sh","-c","sleep 2"]}),
        )
        .await
        .expect("the watched process starts");
    let wake_id = started.payload["wake_id"].as_str().unwrap().to_owned();

    // The provider ends the turn without waiting on the process.
    let session = f
        .state
        .owned
        .session_store
        .get_session(&turn.session_id)
        .unwrap();
    let prompt = f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &turn.agent_id)
        .unwrap();
    f.state
        .owned
        .complete_local_prompt_without_advance(&turn.session_id, &turn.agent_id, Some(f.run.id()))
        .unwrap();
    let Outcome::Settled {
        task: settled,
        correction,
    } = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt_id.clone().unwrap(),
            run: f.run.id().into(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("the turn did not settle")
    };
    f.state
        .finish_agent_task_settlement(Some((settled.clone(), correction)), &prompt)
        .await
        .unwrap();

    assert!(!correction, "ending the turn is never corrected");
    assert_eq!(settled.state, ExecutionState::Done);
    let cards: Vec<_> = f
        .state
        .owned
        .session_snapshot(&turn.session_id)
        .unwrap()
        .active_interactions()
        .iter()
        .map(|card| card.id().to_owned())
        .collect();
    assert!(
        cards.iter().all(|id| !id.starts_with("task-blocked-")),
        "no Agent needs your action card: {cards:?}"
    );
    let store = &f.state.owned.durable_state_store;
    assert_eq!(
        store.agent_wake(&wake_id).unwrap().unwrap().state,
        "running"
    );

    // The process ends: the agent is woken in a new turn.
    let woken = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let session = f
                .state
                .owned
                .session_store
                .get_session(&turn.session_id)
                .unwrap();
            let prompts = session
                .active_prompt_for_agent(&turn.agent_id)
                .into_iter()
                .chain(
                    session
                        .queued_prompts_for_agent(&turn.agent_id)
                        .into_iter()
                        .flatten(),
                )
                .map(|p| p.id().to_owned())
                .collect::<Vec<_>>();
            if let Some(id) = prompts
                .into_iter()
                .find(|id| id.starts_with("agent-event-"))
            {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(woken.is_ok(), "the process end must wake the agent");
    f.state.owned.agent_wakes.processes.shutdown();
}
