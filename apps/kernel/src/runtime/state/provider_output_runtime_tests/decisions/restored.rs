//! A sub-agent's question that does not block its turn (`request_popup` wait:false) is a wake
//! source: a kernel restart restores it. Its holder is checked again then, as on any hand-on.
use super::reparent::cards;
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, AgentWake, ExecutionState};
use crate::runtime::state::decision_holder::DecisionHolder;
use crate::runtime::state::wake_driver::Recovery;

impl Tree {
    /// The task of `agent`'s running turn, as the kernel begins it for a provider turn.
    pub(super) fn begin_task(&self, agent: &str) -> AgentTaskExecution {
        let prompt = self
            .session()
            .active_prompt_for_agent(agent)
            .unwrap()
            .clone();
        let run = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap();
        let store = &self.runtime.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        let Outcome::Task(task) = store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: Some(run.id().into()),
                now,
            })
            .unwrap()
        else {
            panic!("begin returns the task")
        };
        task
    }

    /// `agent` asks without waiting in its running turn and parks on the question.
    fn ask_without_waiting(&self, agent: &str, id: &str) -> AgentWake {
        let task = self.begin_task(agent);
        let store = &self.runtime.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        self.runtime
            .arm_interaction_wake(&self.room, agent, permission(id, agent))
            .unwrap();
        let wake = store.agent_wake(id).unwrap().unwrap();
        store
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                registrations: vec![wake.registration_id.clone()],
                cursor: 0,
                deadline: now + 7_200_000,
                reason: "waiting for the answer".into(),
                now,
            })
            .unwrap();
        wake
    }

    /// The kernel restarts: process-local responders are gone, the durable question is recovered
    /// by its wake driver, as the scheduler does at start.
    fn restart_with(&self, wake: &AgentWake) {
        self.runtime
            .owned
            .pending_interactions
            .write()
            .remove(&wake.id);
        let driver = self
            .runtime
            .owned
            .agent_wakes
            .drivers
            .get("interaction")
            .unwrap();
        assert_eq!(driver.recover(wake), Recovery::Reattached);
    }

    fn last_task_state(&self, agent: &str) -> ExecutionState {
        self.runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&self.room), Some(agent))
            .unwrap()
            .pop()
            .unwrap()
            .state
    }
}

/// Live (pcdrive r3): across a restart the question came back held by the parked room agent,
/// with no card anywhere, and the parent never woke for it. A restored holder that is parked
/// cannot take it: with no eligible ancestor, the user's card is listed in the top panel.
#[tokio::test]
async fn a_question_restored_with_a_parked_holder_reaches_the_user() {
    let t = tree(
        "decision-restored-parked",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.end_turn_yielded(&t.p).await;
    t.end_turn_yielded(&t.g).await;
    for parked in [&t.p, &t.g] {
        assert_eq!(t.last_task_state(parked), ExecutionState::Waiting);
    }
    let wake = t.ask_without_waiting(&t.c, "ask-restored");

    t.restart_with(&wake);

    assert_eq!(
        t.holder("ask-restored"),
        DecisionHolder::User,
        "restored with a parked holder"
    );
    assert_eq!(
        t.panels(),
        [(t.g.clone(), cards(&["ask-restored"]))],
        "the user's card is listed in the top panel"
    );
    assert_eq!(t.needs().1, 1, "the user's decision is counted");
    t.restart_with(&wake);
    assert_eq!(
        t.holder("ask-restored"),
        DecisionHolder::User,
        "a second recovery changes nothing"
    );
}

/// The holders passed the question up to the user during their running turns and the kernel
/// restarted before those turns ended. The restored question keeps the holder the passes gave
/// it: no former holder is told again and the user's decision does not return to an agent.
#[tokio::test]
async fn a_question_passed_before_a_restart_keeps_its_holder() {
    let t = tree(
        "decision-restored-passed",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.begin_task(&t.g);
    t.begin_task(&t.p);
    let wake = t.ask_without_waiting(&t.c, "ask-passed");
    for holder in [&t.p, &t.g] {
        assert_eq!(
            t.holder("ask-passed"),
            DecisionHolder::Agent(holder.clone())
        );
        assert_eq!(
            t.runtime
                .pass_child_decision(&t.room, holder, "ask-passed")
                .await
                .unwrap(),
            ChildDecisionStatus::Passed
        );
    }
    assert_eq!(t.holder("ask-passed"), DecisionHolder::User);
    for former in [&t.p, &t.g] {
        assert_eq!(t.last_task_state(former), ExecutionState::Working);
    }

    t.restart_with(&wake);

    assert_eq!(
        t.holder("ask-passed"),
        DecisionHolder::User,
        "the restart gave the passed decision back to an agent"
    );
    for former in [&t.p, &t.g] {
        assert_eq!(
            t.decision_events(former).len(),
            1,
            "a holder that passed the question is never told again"
        );
    }
}
