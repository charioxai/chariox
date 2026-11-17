//! An idle agent whose start failed gets its next wake or message once the cause is gone.
use super::agent_reply_tests::Fixture;
use crate::durable_state::agent_lifecycle::{
    AgentWake, DelegateGoneReason, InboxEvent, Operation, Outcome, DELIVERY_TIMEOUT_MS,
};

impl Fixture {
    fn child_inbox(&self) -> Vec<InboxEvent> {
        self.runtime
            .owned
            .durable_state_store
            .agent_inbox(&self.room, &self.child, 0)
            .unwrap()
    }

    fn state_of(&self, sequence: u64) -> String {
        self.child_inbox()
            .into_iter()
            .find(|e| e.sequence == sequence)
            .unwrap()
            .state
    }

    fn child_is_running(&self) -> bool {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.room)
            .unwrap();
        self.runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &self.child)
            .is_some()
    }

    /// The supervision sweep as it runs once the delivery window has passed.
    fn sweep_after_delivery_window(&self) {
        let now = crate::session::unix_epoch_ms() + DELIVERY_TIMEOUT_MS + 1_000;
        self.runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Sweep {
                now,
                busy_recipients: vec![],
                vault_wait_recipients: vec![],
                held_work: vec![],
            })
            .unwrap();
    }

    fn blocked_delivery_tasks(&self) -> usize {
        self.runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&self.room), Some(&self.child))
            .unwrap()
            .into_iter()
            .filter(|t| t.task_id.starts_with("delivery-"))
            .count()
    }

    /// Takes the room's directory away, as a deleted worktree does.
    fn remove_workspace(&self) -> std::path::PathBuf {
        let dir = self._worktree.path().to_path_buf();
        let moved = dir.with_extension("moved");
        std::fs::rename(&dir, &moved).unwrap();
        moved
    }
}

#[tokio::test]
async fn refused_start_ends_with_a_typed_failure_and_the_next_message_starts_a_turn() {
    let f = Fixture::with_child_provider("dev-stub");
    let moved = f.remove_workspace();
    let first = f.message("while-the-directory-is-gone", true, false);
    let error = f
        .runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not exist"), "{error}");
    assert_eq!(f.state_of(first.sequence), "pending");

    f.sweep_after_delivery_window();
    // No provider I/O happened, so the message is definitively undelivered: it
    // ends as failed and tells its sender, instead of fencing the recipient.
    assert_eq!(f.state_of(first.sequence), "failed");
    assert_eq!(
        f.blocked_delivery_tasks(),
        1,
        "the owner is asked about the undelivered message"
    );
    let told = f
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&f.room, &f.parent, 0)
        .unwrap();
    assert!(
        told.iter().any(|e| e.kind == "source_lost"
            && e.payload["public_answer"] == DelegateGoneReason::FailedStart.answer(&f.child)),
        "the sender is told the delivery failed: {told:?}"
    );

    // The cause is gone: the next urgent message starts a turn at once.
    std::fs::rename(&moved, f._worktree.path()).unwrap();
    let next = f.message("after-the-directory-is-back", true, false);
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    assert_eq!(f.state_of(next.sequence), "accepted");
    assert!(f.child_is_running());
}

#[tokio::test]
async fn resume_on_an_unconfirmed_delivery_card_delivers_the_message_again() {
    let f = Fixture::with_child_provider("dev-stub");
    let store = &f.runtime.owned.durable_state_store;
    let lost = f.message("receipt-never-came", true, false);
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::Attempt {
            room: f.room.clone(),
            agent: f.child.clone(),
            sequence: lost.sequence,
            prompt: "lost-prompt".into(),
            target: None,
            run: None,
            now,
            work: None,
            through: lost.sequence,
        })
        .unwrap();
    let Outcome::Event(_) = store
        .agent_lifecycle(Operation::Receipt {
            room: f.room.clone(),
            agent: f.child.clone(),
            sequence: lost.sequence,
            state: "uncertain".into(),
            now,
        })
        .unwrap()
    else {
        panic!("event")
    };
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(lost.sequence), "blocked");
    let card = store
        .agent_tasks(Some(&f.room), Some(&f.child))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id.starts_with("delivery-"))
        .expect("the owner is asked about the unconfirmed delivery");

    f.runtime
        .resolve_agent_task_owner_action(
            &card,
            super::agent_task_owner_resolution::OwnerTaskAction::Resume,
        )
        .await;

    assert_eq!(f.state_of(lost.sequence), "accepted");
    assert!(f.child_is_running());
}

#[tokio::test]
async fn a_due_wake_starts_a_turn_after_an_earlier_start_failed() {
    let f = Fixture::with_child_provider("dev-stub");
    let moved = f.remove_workspace();
    let first = f.message("while-the-directory-is-gone", true, false);
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(first.sequence), "failed");
    std::fs::rename(&moved, f._worktree.path()).unwrap();

    let Outcome::Event(wake) = f
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(
            crate::durable_state::agent_lifecycle::occurrence(
                &f.room,
                &f.child,
                "watched-process",
                "watched-process-exit",
                "source_completed",
                serde_json::json!({"success": true}),
            ),
        ))
        .unwrap()
    else {
        panic!("event")
    };
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    assert_eq!(f.state_of(wake.sequence), "accepted");
    assert!(f.child_is_running());
}

#[tokio::test]
async fn a_due_timer_starts_a_turn_after_an_earlier_start_failed() {
    let f = Fixture::with_child_provider("dev-stub");
    let moved = f.remove_workspace();
    let first = f.message("while-the-directory-is-gone", true, false);
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(first.sequence), "failed");
    std::fs::rename(&moved, f._worktree.path()).unwrap();

    let store = &f.runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: f.room.clone(),
            agent: f.child.clone(),
            prompt: "timer-turn".into(),
            run: None,
            now,
        })
        .unwrap()
    else {
        panic!("task")
    };
    let timer: AgentWake = serde_json::from_value(serde_json::json!({
        "id": "timer-after-failed-start", "task_id": task.task_id, "room_id": f.room,
        "agent_id": f.child, "registration_id": "completion-timer-after-failed-start",
        "kind": "timer", "label": "backup check", "state": "scheduled", "created_at_ms": now,
        "verified_at_ms": null, "next_due_ms": now + 1_000, "interval_ms": null,
        "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
        "last_sequence": null, "last_delivery": null, "last_delivered_at_ms": null,
        "last_acknowledged_at_ms": null, "alerted_sequence": null
    }))
    .unwrap();
    store
        .agent_lifecycle(Operation::CreateWake {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            wake: timer,
        })
        .unwrap();

    // The timer firing is the only trigger: no lifecycle sweep runs.
    f.runtime.sweep_agent_wakes(now + 70_000).await.unwrap();
    let fired = store
        .agent_wake("timer-after-failed-start")
        .unwrap()
        .unwrap();
    let sequence = fired.last_sequence.expect("the timer fired into the inbox");
    for _ in 0..50 {
        if f.state_of(sequence) == "accepted" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(f.state_of(sequence), "accepted");
    assert!(f.child_is_running());
}

#[tokio::test]
async fn resume_on_the_card_of_an_undelivered_message_sends_it_again() {
    let f = Fixture::with_child_provider("dev-stub");
    let moved = f.remove_workspace();
    let lost = f.message("while-the-directory-is-gone", true, false);
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(lost.sequence), "failed");
    std::fs::rename(&moved, f._worktree.path()).unwrap();
    let card = f
        .runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&f.room), Some(&f.child))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id.starts_with("delivery-"))
        .expect("the owner is asked about the undelivered message");

    f.runtime
        .resolve_agent_task_owner_action(
            &card,
            super::agent_task_owner_resolution::OwnerTaskAction::Resume,
        )
        .await;

    assert_eq!(f.state_of(lost.sequence), "accepted");
    assert!(f.child_is_running());
}
