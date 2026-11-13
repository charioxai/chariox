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
async fn three_wakes_without_handled_progress_name_what_is_missing_on_the_parents_card() {
    let t = tree(
        "blocked-card-wakes",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(
        &t.c,
        "Three consecutive wakes without handled progress; owner must resume or cancel",
    );
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);

    assert_eq!(t.holder(&id), DecisionHolder::Agent(t.p.clone()));
    let events = t.decision_events(&t.p);
    let detail = events[0].payload["detail"].as_str().unwrap();
    assert!(
        detail.contains("chariox.events.ack") && detail.contains("handled"),
        "what is missing: {detail}"
    );
}
