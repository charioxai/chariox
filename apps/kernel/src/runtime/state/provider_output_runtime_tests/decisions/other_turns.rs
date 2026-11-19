//! Live (orch12 11-14): a parent with several tasks is woken by a child's decision in a task of
//! its own. Another task of the same parent ending must not take the decision that task was
//! handed, and an event for a decision the parent no longer holds must not be left needing an
//! answer it cannot give.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};

impl Tree {
    fn run_id(&self, agent: &str) -> String {
        self.runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap()
            .id()
            .into()
    }

    /// The task the decision event was steered into is running.
    fn begin_event_task(&self, agent: &str) {
        let prompt = self
            .session()
            .active_prompt_for_agent(agent)
            .unwrap()
            .clone();
        self.runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: Some(self.run_id(agent)),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }

    /// Another task of `agent`, `task`, runs and ends done through the real settlement path.
    async fn run_and_end_other_task(&self, agent: &str, task: &str) {
        let store = &self.runtime.owned.durable_state_store;
        let run = self.run_id(agent);
        let now = crate::session::unix_epoch_ms();
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: agent.into(),
                prompt: task.into(),
                run: Some(run.clone()),
                now,
            })
            .unwrap();
        let Outcome::Settled {
            task: settled,
            correction,
        } = store
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: agent.into(),
                prompt: task.into(),
                run,
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap()
        else {
            panic!("settled")
        };
        assert_eq!(settled.state, ExecutionState::Done);
        let prompt = self
            .session()
            .active_prompt_for_agent(agent)
            .unwrap()
            .clone();
        self.runtime
            .finish_agent_task_settlement(Some((settled, correction)), &prompt)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn another_task_of_the_holder_ending_leaves_the_decision_to_the_task_it_was_handed_to() {
    let t = tree(
        "decision-other-task",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-other", &t.c)).await;
    t.begin_event_task(&t.p);

    t.run_and_end_other_task(&t.p, "other-task-w").await;

    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "perm-other", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered,
        "the event task still holds the decision"
    );
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
}

#[tokio::test]
async fn the_task_the_decision_was_handed_to_ending_hands_it_up() {
    let t = tree(
        "decision-other-task-ends",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-other-end", &t.c)).await;
    t.begin_event_task(&t.p);
    t.run_and_end_other_task(&t.p, "other-task-w").await;

    t.end_turn(&t.p).await;

    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "perm-other-end", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::NotHolder,
        "its task ended without an answer"
    );
}

/// The delivery of the event was still in flight when the decision moved on (a withdrawn event
/// in flight stays open): the parent cannot answer it, so it is not asked to.
#[tokio::test]
async fn an_event_for_a_decision_the_agent_no_longer_holds_can_be_acknowledged() {
    let t = tree(
        "decision-ack-not-holder",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let lane = t
        .runtime
        .provider_runtime_lanes
        .acquire(&t.run_id(&t.p))
        .await;
    let _answer = t.raise(permission("perm-ack-moved", &t.c)).await;
    drop(lane);
    let sequence = t.decision_events(&t.p)[0].sequence;
    t.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Attempt {
            room: t.room.clone(),
            agent: t.p.clone(),
            sequence,
            prompt: "event-task-e".into(),
            target: None,
            run: Some(t.run_id(&t.p)),
            now: crate::session::unix_epoch_ms(),
            work: None,
            through: sequence,
        })
        .unwrap();
    assert_eq!(t.decision_events(&t.p)[0].state, "submitting");
    t.runtime
        .pass_child_decision(&t.room, &t.p, "perm-ack-moved")
        .await
        .unwrap();

    let read = t
        .runtime
        .read_agent_inbox(&t.room, &t.p, &serde_json::json!({}))
        .unwrap();
    let view = read["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["sequence"] == sequence)
        .expect("the event is still listed");
    assert_eq!(view["needs_answer"], false, "{view}");
    // The delivery in flight still has to settle; the decision is no longer what holds it up.
    let error = t
        .runtime
        .ack_agent_inbox(
            &t.room,
            &t.p,
            &serde_json::json!({"sequence": sequence, "handled": true}),
            1,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("needs an answer"), "{error}");
}
