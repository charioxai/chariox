//! A blocked task's Resume/Cancel card is a decision about an agent: it goes to the parent that
//! spawned the agent, says what the agent still owes, and only a root agent's card reaches the
//! person. Tree: g (room agent) -> p -> c.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, AgentWake, ExecutionState};
use crate::runtime::state::decision_holder::DecisionHolder;

const INVALID_TURN_END: &str = "Repeated invalid turn end; owner must resume or cancel";

fn timer(task: &AgentTaskExecution, id: &str) -> AgentWake {
    let now = crate::session::unix_epoch_ms();
    serde_json::from_value(serde_json::json!({
        "id": id, "task_id": task.task_id, "room_id": task.room_id, "agent_id": task.agent_id,
        "registration_id": format!("completion-{id}"), "kind": "timer", "label": "wait for review",
        "state": "scheduled", "created_at_ms": now, "verified_at_ms": null,
        "next_due_ms": now + 3_600_000, "interval_ms": null, "command": [],
        "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
        "last_sequence": null, "last_delivery": null, "last_delivered_at_ms": null,
        "last_acknowledged_at_ms": null, "alerted_sequence": null
    }))
    .unwrap()
}

impl Tree {
    /// `agent`'s admitted turn begins, arms a timer it never waits on and is blocked with `reason`.
    fn blocked_task_of(&self, agent: &str, reason: &str) -> AgentTaskExecution {
        let session = self.session();
        let prompt = session.active_prompt_for_agent(agent).unwrap().clone();
        let run = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap();
        let store = &self.runtime.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: Some(run.id().into()),
                now: 1,
            })
            .unwrap();
        let task = store
            .agent_tasks(Some(&self.room), Some(agent))
            .unwrap()
            .pop()
            .unwrap();
        store
            .agent_lifecycle(Operation::CreateWake {
                task: task.task_id.clone(),
                prompt: prompt.id().into(),
                wake: timer(&task, "wake-blk"),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Block {
                task: task.task_id.clone(),
                prompt: prompt.id().into(),
                reason: reason.into(),
            })
            .unwrap();
        let task = store.agent_task(&task.task_id).unwrap().unwrap();
        assert_eq!(task.state, ExecutionState::Blocked);
        task
    }

    fn blocked_card_id(task: &AgentTaskExecution) -> String {
        format!("task-blocked-{}-{}", task.task_id, task.blocked_revision)
    }
}

#[tokio::test]
async fn a_blocked_helpers_card_goes_to_its_parent_and_names_what_it_owes() {
    let t = tree(
        "blocked-card-parent",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.c, INVALID_TURN_END);
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);

    assert_eq!(t.holder(&id), DecisionHolder::Agent(t.p.clone()));
    assert!(
        t.panels().is_empty(),
        "no person card while the parent holds it"
    );
    let events = t.decision_events(&t.p);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["interaction_id"], id);
    assert_eq!(events[0].payload["needs_answer"], true);
    let detail = events[0].payload["detail"].as_str().unwrap();
    assert!(
        detail.contains("wake-blk"),
        "the open obligation is named: {detail}"
    );
    assert!(
        detail.contains("wait for review"),
        "the source label is named: {detail}"
    );
    assert!(
        detail.contains("chariox.events.cancel_wake"),
        "the exact settling call is named: {detail}"
    );
    let choices = events[0].payload["choices"].to_string();
    assert!(
        choices.contains("resume") && choices.contains("cancel"),
        "{choices}"
    );
    assert!(
        t.decision_events(&t.g).is_empty(),
        "only the holder is told"
    );
}

#[tokio::test]
async fn the_parent_cancelling_a_blocked_helper_ends_it_and_its_timer() {
    let t = tree(
        "blocked-card-cancel",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.c, INVALID_TURN_END);
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);

    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, &id, "cancel")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered
    );
    let store = &t.runtime.owned.durable_state_store;
    for _ in 0..100 {
        if store.agent_task(&task.task_id).unwrap().unwrap().state == ExecutionState::Cancelled {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        store.agent_task(&task.task_id).unwrap().unwrap().state,
        ExecutionState::Cancelled
    );
    assert_eq!(
        store.agent_wake("wake-blk").unwrap().unwrap().state,
        "cancelled"
    );
}

#[tokio::test]
async fn a_blocked_root_agents_card_stays_the_persons_and_names_what_it_owes() {
    let t = tree(
        "blocked-card-root",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.g, INVALID_TURN_END);
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);

    assert_eq!(t.holder(&id), DecisionHolder::User);
    let session = t.session();
    let card = session
        .active_interactions()
        .iter()
        .find(|i| i.id() == id)
        .unwrap();
    assert!(card.message().contains("wake-blk"), "{}", card.message());
    assert!(t.decision_events(&t.p).is_empty() && t.decision_events(&t.g).is_empty());
}

#[tokio::test]
async fn correction_and_resume_name_the_obligation_label_and_actual_registration() {
    let t = tree(
        "blocked-instructions",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.c, INVALID_TURN_END);
    let owned = &t.runtime.owned;
    for text in [
        owned.task_correction_text(&task).unwrap(),
        owned.task_resume_text(&task, &task.reason).unwrap(),
    ] {
        assert!(text.contains("obligation wake-blk"), "{text}");
        assert!(text.contains("wait for review"), "{text}");
        assert!(
            text.contains(r#"chariox.events.cancel_wake with wake_id "wake-blk""#),
            "{text}"
        );
        assert!(
            text.contains(r#"registration_ids ["completion-wake-blk"]"#),
            "{text}"
        );
        assert!(text.contains("task_id and origin_prompt_id"), "{text}");
    }
    let resumed = owned.task_resume_text(&task, &task.reason).unwrap();
    assert!(
        resumed.contains("Repeated invalid turn end") && resumed.contains("has now been resumed"),
        "{resumed}"
    );
    let mut settled = task;
    settled.obligations.clear();
    let text = owned.task_resume_text(&settled, "blocked").unwrap();
    assert!(
        text.contains("final answer") && !text.contains("chariox.events.yield"),
        "{text}"
    );
}

#[tokio::test]
async fn a_settling_obligation_names_its_exact_receipt_in_correction_and_resume() {
    let t = tree(
        "blocked-receipt",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.c, "Review the timer outcome before continuing");
    let store = &t.runtime.owned.durable_state_store;
    // The completion is beyond the first page of unrelated retained history.
    for index in 0..(crate::durable_state::agent_lifecycle::INBOX_PAGE + 2) {
        store
            .agent_lifecycle(Operation::Occur(
                crate::durable_state::agent_lifecycle::occurrence(
                    &t.room,
                    &t.c,
                    "unrelated",
                    &format!("old-{index}"),
                    "message",
                    serde_json::json!({}),
                ),
            ))
            .unwrap();
    }
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::SourceOutcome {
            room: t.room.clone(),
            source: "wake-blk".into(),
            occurrence: "timer-fired".into(),
            success: true,
            public_answer: None,
            now,
        })
        .unwrap();
    let task = store.agent_task(&task.task_id).unwrap().unwrap();
    let sequence: u64 = rusqlite::Connection::open(store.path())
        .unwrap()
        .query_row(
            "SELECT MAX(sequence) FROM agent_inbox WHERE source_id='wake-blk'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for text in [
        t.runtime.owned.task_correction_text(&task).unwrap(),
        t.runtime
            .owned
            .task_resume_text(&task, &task.reason)
            .unwrap(),
    ] {
        assert!(
            text.contains("wait for review") && text.contains("chariox.events.inbox"),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "chariox.events.ack with sequence {} and handled:true",
                sequence
            )),
            "{text}"
        );
        assert!(!text.contains("chariox.events.yield"), "{text}");
    }
}

#[tokio::test]
async fn corrupt_completion_registrations_keep_the_parents_cancel_decision() {
    let t = tree(
        "blocked-corrupt",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(
        &t.c,
        "Source registration is corrupt; restore the exact receipt or cancel",
    );
    let connection =
        rusqlite::Connection::open(t.runtime.owned.durable_state_store.path()).unwrap();
    connection
        .execute(
            "UPDATE agent_registrations SET payload='broken' WHERE task_id=?1",
            [&task.task_id],
        )
        .unwrap();
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    assert_eq!(
        t.holder(&Tree::blocked_card_id(&task)),
        DecisionHolder::Agent(t.p.clone())
    );
    let event = t.decision_events(&t.p).pop().unwrap();
    let detail = event.payload["detail"].as_str().unwrap();
    assert!(
        detail.contains("Recovery instructions could not be read")
            && detail.contains("Cancel this task"),
        "{detail}"
    );
}
