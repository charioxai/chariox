//! An agent may always end its turn, also with a watched process still running: the task ends
//! Done with no correction and no "Agent needs your action" card, the process stays supervised and
//! its end wakes the agent.
use super::tests::{fixture_with_run_profile, running, Fixture};
use super::watched_process_grant_tests::begin_task;
use super::KernelSudoTurn;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
use crate::local::{CancelActivePromptRequest, LocalDaemonRequest, LocalDaemonResponse};
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

    let (settled, correction) = end_turn_with_an_answer(&f, &turn).await;

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

/// Stop reaches work a Done task retains: the process is killed and the obligation is cancelled,
/// while the task stays the Done turn it was.
#[tokio::test]
async fn stop_after_a_turn_ended_with_a_running_process_kills_the_process() {
    let f = fixture_with_run_profile(None, true, "claude", "claude");
    let turn = running(&f);
    let task = begin_task(&f, &turn, false, AgentPermissionLevel::Yolo).await;
    let started = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.process",
            serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id,
                "label":"long", "argv":["/bin/sh","-c","sleep 600"]}),
        )
        .await
        .expect("the watched process starts");
    let wake_id = started.payload["wake_id"].as_str().unwrap().to_owned();
    let (settled, _) = end_turn_with_an_answer(&f, &turn).await;
    assert_eq!(settled.state, ExecutionState::Done);
    let store = &f.state.owned.durable_state_store;
    assert_eq!(
        store.agent_wake(&wake_id).unwrap().unwrap().state,
        "running"
    );

    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: None,
        expected_prompt_id: None,
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "stop-done-with-process",
        None,
        None,
        &request,
    );
    let response = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task: stopped, .. } = response else {
        panic!("expected the stopped task")
    };
    assert_eq!(stopped.task_id, settled.task_id);
    assert_eq!(
        stopped.state,
        ExecutionState::Done,
        "the finished turn stays done"
    );

    let ended = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let wake = store.agent_wake(&wake_id).unwrap().unwrap();
            if !matches!(wake.state.as_str(), "running" | "cancelling" | "starting") {
                break wake.state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "Stop must kill the process the done task retained"
    );
    let after = store.agent_task(&settled.task_id).unwrap().unwrap();
    assert_eq!(after.state, ExecutionState::Done);
    assert!(
        after.obligations.iter().all(|o| o.status != "open"),
        "no open obligation remains: {:?}",
        after.obligations
    );
    f.state.owned.agent_wakes.processes.shutdown();
}

/// The provider ends the turn with an answer without waiting on what it started.
async fn end_turn_with_an_answer(
    f: &Fixture,
    turn: &KernelSudoTurn,
) -> (
    crate::durable_state::agent_lifecycle::AgentTaskExecution,
    bool,
) {
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
    (settled, correction)
}
