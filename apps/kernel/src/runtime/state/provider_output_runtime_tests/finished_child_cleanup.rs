//! A parent that handled its child's `child_finished` receipt does not delete the child itself:
//! the kernel ends a sub-agent the parent spawned once the parent acked its receipt as handled.
use super::child_finished::{family_of, Family};
use super::{owned_runtime_state, Arc, Mutex};
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

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
    // Yield records intent; only the native turn boundary commits Waiting.
    store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.parent.clone(),
            prompt: parent_task.prompt_id.clone(),
            run: team.parent_run.clone(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let waiting = store.agent_task(&parent_task.task_id).unwrap().unwrap();
    assert_eq!(waiting.wait.as_ref().unwrap().deadline_ms, None);
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.parent, Some(&team.parent_run))
        .unwrap();
    store
        .agent_lifecycle(Operation::Sweep {
            now: crate::session::unix_epoch_ms(),
            vault_wait_recipients: Vec::new(),
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        })
        .unwrap();
    assert!(
        store
            .agent_inbox(&team.room, &team.parent, 0)
            .unwrap()
            .is_empty(),
        "nothing wakes the parent while the child runs"
    );
    let still_waiting = store.agent_task(&parent_task.task_id).unwrap().unwrap();
    assert_eq!(still_waiting.state, ExecutionState::Waiting);
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
async fn a_parent_keeps_a_finished_child_for_its_next_round() {
    let team = family_of("child-cleanup-keep", 1, None, true, true).await;
    let child = team.kids[0].child.clone();
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    team.runtime
        .ack_agent_inbox(
            &team.room,
            &team.parent,
            &serde_json::json!({"sequence": receipt, "handled": true, "keep_children": true}),
            crate::session::unix_epoch_ms(),
        )
        .await
        .unwrap();
    assert!(
        child_exists(&team, &child),
        "a child the parent keeps survives its handled receipt"
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

/// The child finishes a newer, independent task `prompt`; its older task stays unfinished.
/// Returns the receipt of the newer task in the parent's inbox.
async fn child_finishes_a_newer_task(team: &Family, prompt: &str) -> u64 {
    let store = &team.runtime.owned.durable_state_store;
    let kid = &team.kids[0];
    let crate::durable_state::agent_lifecycle::Outcome::Task(_) = store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: prompt.into(),
            run: None,
            now: 3,
        })
        .unwrap()
    else {
        panic!("begun")
    };
    let crate::durable_state::agent_lifecycle::Outcome::Settled { task, .. } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: prompt.into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    assert_ne!(task.task_id, kid.task.task_id);
    team.runtime
        .finish_agent_task_settlement(Some((task, false)), &team.kids[0].child_prompt)
        .await
        .unwrap();
    store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap()
        .into_iter()
        .rfind(|event| event.kind == "child_finished")
        .expect("the newer task's finish reaches the parent")
        .sequence
}

// Review P1: a child with an older task still waiting is not finished by the newer task's receipt.
#[tokio::test]
async fn a_handled_receipt_keeps_a_child_whose_older_task_is_unfinished() {
    let team = family_of("child-cleanup-older-task", 1, None, true, true).await;
    let child = team.kids[0].child.clone();
    let newer = child_finishes_a_newer_task(&team, "newer-prompt").await;
    ack(&team, newer, true).await;
    assert!(
        child_exists(&team, &child),
        "the older task is still unfinished: the child stays"
    );
    // The older task ends and its receipt is handled: now the child goes.
    let store = &team.runtime.owned.durable_state_store;
    let crate::durable_state::agent_lifecycle::Outcome::Settled { task, .. } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: child.clone(),
            prompt: team.kids[0].child_prompt.id().into(),
            run: team.kids[0].child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 5,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &child, Some(&team.kids[0].child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((task, false)), &team.kids[0].child_prompt)
        .await
        .unwrap();
    let older = store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap()
        .into_iter()
        .rfind(|event| event.kind == "child_finished")
        .unwrap()
        .sequence;
    assert_ne!(older, newer);
    ack(&team, older, true).await;
    assert!(
        !child_exists(&team, &child),
        "with nothing left unfinished, the handled receipt ends the child"
    );
}

/// The parent's turn ends while it acks: the receipt is handled, but the cleanup is refused.
async fn ack_with_a_retired_turn(team: &Family, sequence: u64) {
    let retired = team
        .runtime
        .with_room_provider_origin(Some(&team.parent), Some("retired-provider-run"));
    let _ = retired
        .ack_agent_inbox(
            &team.room,
            &team.parent,
            &serde_json::json!({"sequence": sequence, "handled": true}),
            crate::session::unix_epoch_ms(),
        )
        .await;
    let receipt = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap()
        .into_iter()
        .find(|event| event.sequence == sequence)
        .unwrap();
    assert_eq!(receipt.state, "handled");
}

// Review P2: a kernel stop between the handled ack and the delete must not lose the delete.
#[tokio::test]
async fn a_stop_between_the_ack_and_the_delete_is_finished_after_a_restart() {
    let team = family_of("child-cleanup-restart", 1, None, true, true).await;
    let child = team.kids[0].child.clone();
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    ack_with_a_retired_turn(&team, receipt).await;
    assert!(
        child_exists(&team, &child),
        "the stop came before the delete"
    );
    let Family {
        runtime,
        room,
        parent,
        parent_run,
        kids,
        config,
        _worktree,
    } = team;
    drop(runtime);
    let app = crate::test_support::bootstrap_after_test_owner_exit(config.clone()).await;
    let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    let team = Family {
        runtime,
        room,
        parent,
        parent_run,
        kids,
        config,
        _worktree,
    };
    assert!(
        child_exists(&team, &child),
        "the restart alone deletes nothing"
    );
    team.runtime.end_orphaned_sub_agents().await;
    assert!(
        !child_exists(&team, &child),
        "the first pass of the restarted kernel ends the handled child"
    );
}

#[tokio::test]
async fn a_failed_cleanup_is_retried_not_lost() {
    let team = family_of("child-cleanup-retry", 1, None, true, true).await;
    let child = team.kids[0].child.clone();
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    ack_with_a_retired_turn(&team, receipt).await;
    let retired = team
        .runtime
        .with_room_provider_origin(Some(&team.parent), Some("retired-provider-run"));
    assert_eq!(
        retired.end_orphaned_sub_agents().await,
        0,
        "the pass is refused too"
    );
    assert!(
        child_exists(&team, &child),
        "a failed cleanup keeps the child"
    );
    assert_eq!(team.runtime.end_orphaned_sub_agents().await, 1);
    assert!(!child_exists(&team, &child), "the next pass ends it");
}

#[tokio::test]
async fn kfinish_handled_child_receipt_finishes_the_original_parent_task() {
    let team = family_of("kfinish-parent-settlement", 1, None, true, true).await;
    let (parent_task, receipt) = parent_waits_and_child_finishes(&team).await;
    ack(&team, receipt, true).await;
    let task = team
        .runtime
        .owned
        .durable_state_store
        .agent_task(&parent_task)
        .unwrap()
        .unwrap();
    assert_eq!(task.state, ExecutionState::Done, "{task:?}");
    assert!(task.wait.is_none());
    assert!(!task.reason.is_empty());
    assert!(!child_exists(&team, &team.kids[0].child));
    // The receipt was handled by a separate event turn. Once that turn ends,
    // completion of the original task must also reach its subscribers.
    let event_task = team
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&team.room), Some(&team.parent))
        .unwrap()
        .into_iter()
        .find(|task| task.task_id != parent_task && task.state == ExecutionState::Working)
        .expect("the receipt has its own event turn");
    let event_prompt = team
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &team
                .runtime
                .owned
                .session_store
                .get_session(&team.room)
                .unwrap(),
            &team.parent,
        )
        .unwrap();
    let crate::durable_state::agent_lifecycle::Outcome::Settled {
        task: event_settled,
        correction,
    } = team
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.parent.clone(),
            prompt: event_task.prompt_id.clone(),
            run: team.parent_run.clone(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("event turn settled")
    };
    assert!(!correction);
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.parent, Some(&team.parent_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((event_settled, false)), &event_prompt)
        .await
        .unwrap();
    assert!(
        team.runtime
            .owned
            .durable_state_store
            .agent_task_terminal_recorded(&task)
            .unwrap(),
        "the older wait must publish before this event turn can trigger agent cleanup"
    );
    let db = rusqlite::Connection::open(team.runtime.owned.durable_state_store.path()).unwrap();
    let sequence = |task: &str| {
        db.query_row(
        "SELECT sequence FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND occurrence_id=?3",
        rusqlite::params![team.room, task, format!("task-terminal-{task}")],
        |row| row.get::<_,i64>(0),
    ).unwrap()
    };
    let old_sequence = sequence(&parent_task);
    assert!(old_sequence < sequence(&event_task.task_id));
    team.runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    assert_eq!(
        sequence(&parent_task),
        old_sequence,
        "recovery does not republish the source"
    );
}

#[tokio::test]
async fn kfinish_auto_cleanup_waits_for_all_terminal_receipts_to_be_published() {
    let team = family_of("kfinish-unpublished-completion", 1, None, true, true).await;
    let (_, receipt) = parent_waits_and_child_finishes(&team).await;
    let kid = &team.kids[0];
    let store = &team.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: "retained-finish".into(),
            run: Some(kid.child_run.clone()),
            now: 3,
        })
        .unwrap();
    let crate::durable_state::agent_lifecycle::Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: "retained-finish".into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Done);
    ack(&team, receipt, true).await;
    assert!(
        child_exists(&team, &kid.child),
        "a terminal outcome is still owed"
    );
    team.runtime
        .finish_agent_task_settlement(Some((task, false)), &kid.child_prompt)
        .await
        .unwrap();
    let last = store
        .agent_inbox(&team.room, &team.parent, receipt)
        .unwrap()
        .into_iter()
        .find(|event| {
            event.kind == "child_finished" && event.payload["child_task_id"] == "retained-finish"
        })
        .unwrap();
    ack(&team, last.sequence, true).await;
    assert!(!child_exists(&team, &kid.child));
}

#[tokio::test]
async fn kfinish_an_unrelated_turn_does_not_publish_an_older_tasks_answer() {
    let team = family_of("kfinish-unrelated-publication", 1, None, true, true).await;
    let kid = &team.kids[0];
    let store = &team.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: "unrelated-completed".into(),
            run: Some(kid.child_run.clone()),
            now: 3,
        })
        .unwrap();
    let crate::durable_state::agent_lifecycle::Outcome::Settled { task: older, .. } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: "unrelated-completed".into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap()
    else {
        panic!("older task settled")
    };
    let crate::durable_state::agent_lifecycle::Outcome::Settled { task: current, .. } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: kid.child_prompt.id().into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 5,
        })
        .unwrap()
    else {
        panic!("current task settled")
    };
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &kid.child, Some(&kid.child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((current, false)), &kid.child_prompt)
        .await
        .unwrap();
    assert!(
        !store.agent_task_terminal_recorded(&older).unwrap(),
        "an unrelated native turn cannot supply the older task's public answer"
    );
    team.runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    assert!(
        store.agent_task_terminal_recorded(&older).unwrap(),
        "recovery publishes the older task from its own history"
    );
}
