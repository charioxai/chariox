//! A parent that handled its child's `child_finished` receipt does not delete the child itself:
//! the kernel ends a sub-agent the parent spawned once the parent acked its receipt as handled.
use super::child_finished::{family_of, Family};
use crate::durable_state::agent_lifecycle::Operation;

/// Calls a runtime tool as the parent's running turn.
async fn parent_tool(team: &Family, tool: &str, arguments: serde_json::Value) -> serde_json::Value {
    let token = team
        .runtime
        .owned
        .provider_store
        .get_run(&team.parent_run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    team.runtime
        .dispatch_authenticated_runtime_tool_call(&token, tool, arguments)
        .await
        .unwrap()
        .payload
}

fn child_exists(team: &Family, child: &str) -> bool {
    team.runtime.owned.agent_store.get_agent(child).is_ok()
}

/// The parent subscribes to its one working child, yields on it with no deadline and ends its turn;
/// the child then finishes. Returns the parent task id and the receipt's sequence.
async fn parent_waits_and_child_finishes(team: &Family) -> (String, u64) {
    let store = &team.runtime.owned.durable_state_store;
    let kid = &team.kids[0];
    let parent_task = store
        .agent_tasks(Some(&team.room), Some(&team.parent))
        .unwrap()
        .remove(0);
    store
        .agent_lifecycle(Operation::BindDelegate {
            parent_task: parent_task.task_id.clone(),
            child_task: kid.task.task_id.clone(),
        })
        .unwrap();
    let registration = parent_tool(
        team,
        "chariox.events.subscribe",
        serde_json::json!({
            "task_id": parent_task.task_id,
            "origin_prompt_id": parent_task.prompt_id,
            "source_id": kid.child,
            "obligation_id": parent_task.obligations[0].id,
        }),
    )
    .await["registration_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // No deadline_ms: the wait ends with the child.
    parent_tool(
        team,
        "chariox.events.yield",
        serde_json::json!({
            "task_id": parent_task.task_id,
            "origin_prompt_id": parent_task.prompt_id,
            "registration_ids": [registration],
            "inbox_cursor": 0,
            "reason": "waiting for the child",
        }),
    )
    .await;
    let waiting = store.agent_task(&parent_task.task_id).unwrap().unwrap();
    assert_eq!(waiting.wait.as_ref().unwrap().deadline_ms, None);
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.parent, Some(&team.parent_run))
        .unwrap();
    // The child runs for longer than three of the old 20-minute deadlines: no timed wake, no strike.
    let now = crate::session::unix_epoch_ms();
    for round in 1..=4 {
        store
            .agent_lifecycle(Operation::Sweep {
                now: now + round * 3 * 20 * 60_000,
                vault_wait_recipients: Vec::new(),
                busy_recipients: Vec::new(),
                held_work: Vec::new(),
            })
            .unwrap();
    }
    assert!(
        store
            .agent_inbox(&team.room, &team.parent, 0)
            .unwrap()
            .is_empty(),
        "nothing wakes the parent while the child runs"
    );
    let still_waiting = store.agent_task(&parent_task.task_id).unwrap().unwrap();
    assert_eq!(still_waiting.no_progress_wakes, 0);
    assert!(
        blocked_cards(team).is_empty(),
        "no owner card for a wait on a running child"
    );
    let crate::durable_state::agent_lifecycle::Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: kid.child_prompt.id().into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &kid.child, Some(&kid.child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((task, correction)), &kid.child_prompt)
        .await
        .unwrap();
    let receipt = store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "child_finished")
        .expect("the finish is the only wake of a wait without a deadline");
    assert!(
        receipt.prompt_id.is_some(),
        "the parent was woken: {receipt:?}"
    );
    (parent_task.task_id, receipt.sequence)
}

async fn ack(team: &Family, sequence: u64, handled: bool) {
    team.runtime
        .ack_agent_inbox(
            &team.room,
            &team.parent,
            &serde_json::json!({"sequence": sequence, "handled": handled}),
            crate::session::unix_epoch_ms(),
        )
        .await
        .unwrap();
}

// Drill: spawn one child, yield with no deadline, wake only on its child_finished, ack it handled,
// and the child is gone.
#[tokio::test]
async fn a_handled_child_finished_receipt_deletes_the_spawned_child() {
    let team = family_of("child-cleanup-handled", 1, None, true, true).await;
    let child = team.kids[0].child.clone();
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    assert!(
        child_exists(&team, &child),
        "the child stays until its receipt is handled"
    );
    ack(&team, receipt, true).await;
    assert!(
        !child_exists(&team, &child),
        "a handled receipt ends the child"
    );
    assert!(
        team.runtime
            .owned
            .agent_store
            .list_agents()
            .iter()
            .all(|agent| agent.id() != child),
        "the child is gone from the agent list"
    );
}

#[tokio::test]
async fn an_unhandled_ack_keeps_the_child() {
    let team = family_of("child-cleanup-unhandled", 1, None, true, true).await;
    let child = team.kids[0].child.clone();
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    ack(&team, receipt, false).await;
    assert!(
        child_exists(&team, &child),
        "only a handled ack ends the child"
    );
}

#[tokio::test]
async fn a_child_a_person_moved_under_the_parent_is_kept() {
    let team = family_of("child-cleanup-person", 1, None, true, false).await;
    let child = team.kids[0].child.clone();
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    ack(&team, receipt, true).await;
    assert!(
        child_exists(&team, &child),
        "an agent a person made is never deleted"
    );
}

fn blocked_cards(team: &Family) -> Vec<String> {
    team.runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap()
        .active_interactions()
        .iter()
        .map(|interaction| interaction.id().to_owned())
        .filter(|id| id.starts_with("task-blocked-"))
        .collect()
}

// Pilot O: an interrupted child's Resume/Cancel card stayed for ever once the child was deleted,
// because the child's task kept the card alive.
#[tokio::test]
async fn deleting_a_blocked_child_clears_its_interrupted_card() {
    let team = family_of("child-cleanup-card", 1, None, true, true).await;
    let kid = &team.kids[0];
    let crate::durable_state::agent_lifecycle::Outcome::Task(blocked) = team
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Block {
            task: kid.task.task_id.clone(),
            prompt: kid.child_prompt.id().into(),
            reason: "interrupted by a kernel restart".into(),
        })
        .unwrap()
    else {
        panic!("blocked")
    };
    team.runtime
        .ensure_task_owner_interaction(blocked)
        .await
        .unwrap();
    assert_eq!(
        blocked_cards(&team).len(),
        1,
        "the owner is asked what to do"
    );
    team.runtime
        .destroy_one_agent(&kid.child, "local", true)
        .await
        .unwrap();
    assert!(
        blocked_cards(&team).is_empty(),
        "nobody is left to resume it"
    );
}
