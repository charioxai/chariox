//! A blocked task's card is a decision about an agent: it goes to the parent that
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
        let task = self.working_task_of(agent);
        let store = &self.runtime.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::CreateWake {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                wake: timer(&task, "wake-blk"),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Block {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                reason: reason.into(),
            })
            .unwrap();
        let task = store.agent_task(&task.task_id).unwrap().unwrap();
        assert_eq!(task.state, ExecutionState::Blocked);
        task
    }

    /// `agent`'s admitted turn begins its task.
    fn working_task_of(&self, agent: &str) -> AgentTaskExecution {
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
        store
            .agent_tasks(Some(&self.room), Some(agent))
            .unwrap()
            .pop()
            .unwrap()
    }

    /// `from`'s working turn sends `to` an urgent message, and the kernel starts writing it into
    /// the provider as `prompt`: the row is submitting until its exact receipt.
    fn submitting_message(&self, from: &AgentTaskExecution, to: &str, prompt: &str) -> u64 {
        let store = &self.runtime.owned.durable_state_store;
        let Outcome::Event(message) = store
            .agent_lifecycle(Operation::Send {
                task: from.task_id.clone(),
                prompt: from.prompt_id.clone(),
                event: InboxEvent {
                    sequence: 0,
                    room_id: self.room.clone(),
                    agent_id: to.into(),
                    source_id: from.agent_id.clone(),
                    occurrence_id: format!("message-as-{prompt}"),
                    kind: "message".into(),
                    payload: serde_json::json!({"message": "status?"}),
                    urgent: true,
                    reply_requested: false,
                    state: "pending".into(),
                    prompt_id: None,
                    target_prompt_id: None,
                    provider_run_id: None,
                    attempted_at_ms: None,
                    submit_epoch: None,
                    resumed_at_ms: None,
                },
            })
            .unwrap()
        else {
            panic!("event")
        };
        store
            .agent_lifecycle(Operation::Attempt {
                room: self.room.clone(),
                agent: to.into(),
                sequence: message.sequence,
                prompt: prompt.into(),
                target: None,
                run: None,
                now: crate::session::unix_epoch_ms(),
                work: None,
                through: message.sequence,
            })
            .unwrap();
        message.sequence
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
    assert_eq!(
        t.panels(),
        vec![(t.g.clone(), vec![id.clone()])],
        "the person sees it in the top panel while the parent holds it"
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
        choices.contains("resume") && choices.contains("cancel") && choices.contains("stop"),
        "{choices}"
    );
    assert!(
        t.decision_events(&t.g).is_empty(),
        "only the holder is told"
    );
}

#[tokio::test]
async fn the_parent_stopping_a_blocked_helper_ends_it_and_its_timer() {
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
            .answer_child_decision(&t.room, &t.p, &id, "stop")
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
async fn dismissing_a_blocked_question_keeps_its_task_and_inbox_card() {
    let t = tree(
        "blocked-question-dismiss",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.working_task_of(&t.g);
    let store = &t.runtime.owned.durable_state_store;
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::BlockForQuestion {
            task: task.task_id,
            prompt: task.prompt_id,
            reason: "Which branch should I use?".into(),
        })
        .unwrap()
    else {
        panic!("expected a blocked task");
    };
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);
    assert_eq!(
        t.session()
            .active_interactions()
            .iter()
            .find(|card| card.id() == id)
            .unwrap()
            .choice("cancel")
            .unwrap()
            .label(),
        "Dismiss"
    );

    t.runtime
        .owned
        .resolve_runtime_interaction(&t.room, &id, "cancel", None, Some("local"), false)
        .unwrap();
    assert!(t
        .runtime
        .owned
        .session_snapshot(&t.room)
        .unwrap()
        .active_interactions()
        .iter()
        .all(|card| card.id() != id));
    let crate::local::LocalDaemonResponse::PersonInbox { cards, .. } =
        crate::runtime::person_inbox::execute_person_inbox_request(
            &t.runtime,
            &crate::runtime::command::KernelCaller::default(),
        )
        .unwrap()
    else {
        panic!("expected the person's Inbox");
    };
    assert!(
        cards.iter().any(|card| card.card_id == id),
        "the dismissed card remains actionable in the person's Inbox"
    );
    assert_eq!(store.agent_task(&task.task_id).unwrap().unwrap(), task);

    // Reattaching the decision after a kernel reload must keep the same card out of the Room.
    t.runtime
        .owned
        .timeout_runtime_interaction(&t.room, &id)
        .unwrap();
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    assert!(t
        .runtime
        .owned
        .session_snapshot(&t.room)
        .unwrap()
        .active_interactions()
        .iter()
        .all(|card| card.id() != id));

    t.runtime
        .owned
        .resolve_runtime_interaction(&t.room, &id, "stop", None, Some("local"), false)
        .unwrap();
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
    assert!(t
        .session()
        .active_interactions()
        .iter()
        .all(|card| card.id() != id));
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
        resumed.contains("Its turn ended before its work was done")
            && resumed.contains("has now been resumed"),
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
    let sequence: i64 = rusqlite::Connection::open(store.path())
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

// Live (orchfix8, 11-16): the kernel expired the process outcome of a blocked task, the resumed
// turn was told to acknowledge "the reviewed receipt" without a sequence, guessed one that is not
// in its inbox and the card came back three times.
#[tokio::test]
async fn a_resumed_task_is_told_the_sequence_of_an_outcome_the_kernel_expired() {
    let t = tree(
        "blocked-expired-receipt",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.c, "Review the timer outcome before continuing");
    let store = &t.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::SourceOutcome {
            room: t.room.clone(),
            source: "wake-blk".into(),
            occurrence: "timer-fired".into(),
            success: true,
            public_answer: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let sequence: u64 = store
        .agent_inbox(&t.room, &t.c, 0)
        .unwrap()
        .iter()
        .find(|event| event.source_id == "wake-blk")
        .unwrap()
        .sequence;
    store
        .agent_lifecycle(Operation::Expire {
            room: t.room.clone(),
            agent: t.c.clone(),
            sequence,
        })
        .unwrap();
    let task = store.agent_task(&task.task_id).unwrap().unwrap();

    let text = t
        .runtime
        .owned
        .task_resume_text(&task, &task.reason)
        .unwrap();
    assert!(
        text.contains(&format!(
            "chariox.events.ack with sequence {sequence} and handled:true"
        )),
        "{text}"
    );

    // The named acknowledgement settles the obligation, so nothing is left to correct.
    store
        .agent_lifecycle(Operation::Ack {
            room: t.room.clone(),
            agent: t.c.clone(),
            sequence,
            handled: true,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let settled = store.agent_task(&task.task_id).unwrap().unwrap();
    assert!(
        settled.obligations.iter().all(|o| !o.requires_resolution()),
        "{:?}",
        settled.obligations
    );
}

#[tokio::test]
async fn corrupt_completion_registrations_keep_the_parents_stop_decision() {
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
            && detail.contains("Stop this task"),
        "{detail}"
    );
}

/// Answers `task`'s card with `choice` through the card entry point and returns the task once
/// the owner action has settled it (left Blocked, or failed visibly).
async fn answer_card(t: &Tree, task: &AgentTaskExecution, choice: &str) -> AgentTaskExecution {
    t.runtime
        .owned
        .resolve_runtime_interaction(
            &t.room,
            &Tree::blocked_card_id(task),
            choice,
            None,
            Some("local"),
            false,
        )
        .unwrap();
    let store = &t.runtime.owned.durable_state_store;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let now = store.agent_task(&task.task_id).unwrap().unwrap();
            let failed = t
                .runtime
                .agent_notices_for_test(&t.room, &task.agent_id)
                .iter()
                .any(|n| n.contains("Owner action failed"));
            if now.state != ExecutionState::Blocked || failed {
                return now;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the owner's answer must settle the task or fail visibly")
}

#[tokio::test]
async fn resume_is_not_refused_by_another_turns_unconfirmed_delivery() {
    let t = tree(
        "blocked-card-other-delivery",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.g, INVALID_TURN_END);
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    // A message for a later turn is on its way into the provider: its receipt is not in yet.
    let sender = t.working_task_of(&t.p);
    t.submitting_message(&sender, &t.g, "later-turn");

    let resumed = answer_card(&t, &task, "resume").await;

    assert_ne!(resumed.state, ExecutionState::Blocked, "{}", resumed.reason);
    let notices = t.runtime.agent_notices_for_test(&t.room, &t.g);
    assert!(
        notices.iter().all(|n| !n.contains("Owner action failed")),
        "{notices:?}"
    );
}

#[tokio::test]
async fn a_refused_resume_says_why_on_the_agent() {
    let t = tree(
        "blocked-card-refused-resume",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.blocked_task_of(&t.g, INVALID_TURN_END);
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    // The blocked turn's own wake is still on its way in: Resume must wait for its receipt.
    let sender = t.working_task_of(&t.p);
    t.submitting_message(&sender, &t.g, &task.prompt_id);

    answer_card(&t, &task, "resume").await;

    let notices = t.runtime.agent_notices_for_test(&t.room, &t.g);
    assert!(
        notices
            .iter()
            .any(|n| n.contains("Owner action failed") && n.contains("unconfirmed delivery")),
        "the notice must name why the Resume did not take: {notices:?}"
    );
}

#[tokio::test]
async fn d1_dismissing_a_question_card_does_not_cancel_the_task() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let t = tree(
        "d1-dismiss",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let task = t.working_task_of(&t.g);
    let store = &t.runtime.owned.durable_state_store;
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::BlockForQuestion {
            task: task.task_id,
            prompt: task.prompt_id,
            reason: "Which branch should I use?".into(),
        })
        .unwrap()
    else {
        panic!("expected a blocked task");
    };
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);
    t.runtime
        .owned
        .resolve_runtime_interaction(&t.room, &id, "cancel", None, Some("local"), false)
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        store.agent_task(&task.task_id).unwrap().unwrap().state,
        ExecutionState::Blocked,
        "Dismiss must not cancel the agent's question task"
    );
    // D3: any client reads the same session projection; the card is closed for everyone.
    assert!(t
        .runtime
        .owned
        .session_snapshot(&t.room)
        .unwrap()
        .active_interactions()
        .iter()
        .all(|card| card.id() != id));
}

/// D4: a child's question card that its parents passed up to the person (`agent pass`) is only
/// set aside by the person's Cancel/Dismiss: the child's task keeps waiting and the decision
/// stays answerable in the person's Inbox.
#[tokio::test]
async fn d4_dismissing_a_passed_up_question_card_does_not_cancel_the_childs_task() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    let t = tree(
        "d4-passed-dismiss",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.begin_task(&t.g);
    t.begin_task(&t.p);
    let task = t.working_task_of(&t.c);
    let store = &t.runtime.owned.durable_state_store;
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::BlockForQuestion {
            task: task.task_id,
            prompt: task.prompt_id,
            reason: "Which snapshot should I delete?".into(),
        })
        .unwrap()
    else {
        panic!("expected a blocked task");
    };
    t.runtime
        .ensure_task_owner_interaction(task.clone())
        .await
        .unwrap();
    let id = Tree::blocked_card_id(&task);
    for holder in [&t.p, &t.g] {
        assert_eq!(t.holder(&id), DecisionHolder::Agent(holder.clone()));
        assert_eq!(
            t.runtime
                .pass_child_decision(&t.room, holder, &id)
                .await
                .unwrap(),
            ChildDecisionStatus::Passed
        );
    }
    assert_eq!(t.holder(&id), DecisionHolder::User);

    t.runtime
        .owned
        .resolve_runtime_interaction(&t.room, &id, "cancel", None, Some("local"), false)
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        store.agent_task(&task.task_id).unwrap().unwrap().state,
        ExecutionState::Blocked,
        "the person's Dismiss must not cancel the child's task"
    );
    assert!(t
        .runtime
        .owned
        .session_snapshot(&t.room)
        .unwrap()
        .active_interactions()
        .iter()
        .all(|card| card.id() != id));
    let crate::local::LocalDaemonResponse::PersonInbox { cards, .. } =
        crate::runtime::person_inbox::execute_person_inbox_request(
            &t.runtime,
            &crate::runtime::command::KernelCaller::default(),
        )
        .unwrap()
    else {
        panic!("expected the person's Inbox");
    };
    assert!(
        cards.iter().any(|card| card.card_id == id),
        "the dismissed card stays answerable in the person's Inbox"
    );
}
