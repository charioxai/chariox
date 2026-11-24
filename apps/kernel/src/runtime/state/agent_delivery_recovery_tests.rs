//! An idle agent whose start failed gets its next wake or message once the cause is gone.
use super::agent_reply_tests::Fixture;
use crate::durable_state::agent_lifecycle::{
    AgentWake, DelegateGoneReason, InboxEvent, Operation, Outcome, DELIVERY_TIMEOUT_MS,
};

impl Fixture {
    pub(super) fn child_inbox(&self) -> Vec<InboxEvent> {
        self.runtime
            .owned
            .durable_state_store
            .agent_inbox(&self.room, &self.child, 0)
            .unwrap()
    }

    pub(super) fn state_of(&self, sequence: u64) -> String {
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

    pub(super) fn blocked_delivery_tasks(&self) -> usize {
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
    pub(super) fn remove_workspace(&self) -> std::path::PathBuf {
        let dir = self._worktree.path().to_path_buf();
        let moved = dir.with_extension("moved");
        std::fs::rename(&dir, &moved).unwrap();
        moved
    }
}

#[tokio::test]
async fn card_rules_refused_start_settles_without_an_owner_decision() {
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

    // The start is retried with growing pauses, not given up on after the delivery window.
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(first.sequence), "pending");
    assert_eq!(
        f.blocked_delivery_tasks(),
        0,
        "no card while retries remain"
    );

    // Once the attempts are spent no provider I/O ever happened, so the message is definitively
    // undelivered: it ends as failed and tells its sender, instead of fencing the recipient.
    f.spend_start_attempts(first.sequence).await;
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(first.sequence), "failed");
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    let room = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(
        room.active_interactions()
            .iter()
            .all(|card| !card.id().starts_with("task-blocked-delivery-")),
        "the failed receipt already settled the delivery"
    );
    assert_eq!(f.blocked_delivery_tasks(), 0);
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
    assert_eq!(f.state_of(next.sequence), "acknowledged");
    assert!(f.child_is_running());
}

impl Fixture {
    /// A delivery attempt whose provider receipt never came, as a restart leaves it.
    fn lost_receipt(&self, label: &str, prompt: &str) -> InboxEvent {
        let lost = self.message(label, true, false);
        let store = &self.runtime.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        store
            .agent_lifecycle(Operation::Attempt {
                room: self.room.clone(),
                agent: self.child.clone(),
                sequence: lost.sequence,
                prompt: prompt.into(),
                target: None,
                run: None,
                now,
                work: None,
                through: lost.sequence,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Receipt {
                room: self.room.clone(),
                agent: self.child.clone(),
                sequence: lost.sequence,
                state: "uncertain".into(),
                now,
            })
            .unwrap();
        lost
    }

    async fn sweep_settling_after_delivery_window(&self) {
        let now = crate::session::unix_epoch_ms() + DELIVERY_TIMEOUT_MS + 1_000;
        self.runtime.settle_interrupted_deliveries(now).unwrap();
        self.sweep_after_delivery_window();
    }
}

/// The turn never started: the kernel sends the same event again, with no card for a person.
#[tokio::test]
async fn an_interrupted_delivery_before_its_turn_started_is_sent_again_without_a_card() {
    let f = Fixture::with_child_provider("dev-stub");
    let lost = f.lost_receipt("receipt-never-came", "lost-prompt");

    f.sweep_settling_after_delivery_window().await;

    assert_eq!(f.state_of(lost.sequence), "pending");
    assert_eq!(
        f.blocked_delivery_tasks(),
        0,
        "no card asks the owner about the delivery"
    );
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    assert_eq!(f.state_of(lost.sequence), "acknowledged");
    assert!(f.child_is_running());
    assert_eq!(
        f.child_inbox()
            .iter()
            .filter(|e| e.sequence == lost.sequence)
            .count(),
        1,
        "the same event, not a copy"
    );
}

/// One damaged row must not stop the lifecycle pass for the rest.
#[tokio::test]
async fn a_damaged_interrupted_delivery_row_does_not_stop_the_lifecycle_pass() {
    let f = Fixture::with_child_provider("dev-stub");
    let good = f.lost_receipt("receipt-never-came", "lost-prompt");
    f.runtime.owned.durable_state_store.agent_ledger_sql_for_test(&format!(
        "INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES('{}','{}','x','damaged','{{\"state\":\"submitting\",\"prompt_id\":\"p\",\"attempted_at_ms\":1}}')",
        f.room, f.child
    ));

    // Both attempts began long ago, as after a restart; the pass runs at the real time.
    f.runtime
        .owned
        .durable_state_store
        .agent_ledger_sql_for_test(&format!(
        "UPDATE agent_inbox SET payload=json_set(payload,'$.attempted_at_ms',1) WHERE sequence={}",
        good.sequence
    ));
    f.runtime
        .sweep_agent_lifecycle_at_for_test(crate::session::unix_epoch_ms())
        .await
        .unwrap();

    assert_eq!(
        f.state_of(good.sequence),
        "pending",
        "the other delivery is settled"
    );
}

/// The earlier busy refusal used up the budget before the restart: the recovered delivery gets
/// its retry first, with the refusal budget measured from its recovery.
#[tokio::test]
async fn a_recovered_delivery_is_retried_before_an_old_refusal_budget_fails_it() {
    let f = Fixture::with_child_provider("dev-stub");
    let lost = f.lost_receipt("receipt-never-came", "lost-prompt");
    f.runtime
        .owned
        .durable_state_store
        .agent_ledger_sql_for_test(&format!(
            "INSERT INTO agent_inbox_refusals VALUES({}, 1)",
            lost.sequence
        ));

    f.sweep_settling_after_delivery_window().await;

    assert_eq!(f.state_of(lost.sequence), "pending");
    assert_eq!(f.blocked_delivery_tasks(), 0);
}

/// A batch settles through its lead's receipt: the companions are found settled, not settled twice.
#[tokio::test]
async fn batch_companions_of_an_interrupted_delivery_settle_once() {
    let f = Fixture::with_child_provider("dev-stub");
    let lead = f.message("batch-lead", true, false);
    let companion = f.message("batch-companion", true, false);
    let store = &f.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Attempt {
            room: f.room.clone(),
            agent: f.child.clone(),
            sequence: lead.sequence,
            prompt: "batch-prompt".into(),
            target: None,
            run: None,
            now: crate::session::unix_epoch_ms(),
            work: None,
            through: companion.sequence,
        })
        .unwrap();

    let now = crate::session::unix_epoch_ms() + DELIVERY_TIMEOUT_MS + 1_000;
    f.runtime.settle_interrupted_deliveries(now).unwrap();
    f.runtime.settle_interrupted_deliveries(now).unwrap();

    assert_eq!(f.state_of(lead.sequence), "pending");
    assert_eq!(f.state_of(companion.sequence), "pending");
}

/// The turn already took the event: it is delivered, and nothing sends it a second time.
#[tokio::test]
async fn an_interrupted_delivery_its_turn_took_is_delivered_without_a_card() {
    let f = Fixture::with_child_provider("dev-stub");
    let lost = f.lost_receipt("receipt-lost-after-turn", "took-prompt");
    f.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: f
                .runtime
                .owned
                .session_store
                .get_session(&f.room)
                .unwrap()
                .owner_user_id()
                .into(),
            room: f.room.clone(),
            agent: f.child.clone(),
            prompt: "took-prompt".into(),
            run: Some("run-1".into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();

    f.sweep_settling_after_delivery_window().await;

    assert_eq!(f.state_of(lost.sequence), "accepted");
    assert_eq!(
        f.blocked_delivery_tasks(),
        0,
        "no card asks the owner about the delivery"
    );
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
    f.spend_start_attempts(first.sequence).await;
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
    assert_eq!(f.state_of(wake.sequence), "handled");
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
    f.spend_start_attempts(first.sequence).await;
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(first.sequence), "failed");
    std::fs::rename(&moved, f._worktree.path()).unwrap();

    let store = &f.runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    f.create_one_shot_timer("timer-after-failed-start", now);

    // The timer firing is the only trigger: no lifecycle sweep runs.
    f.runtime.sweep_agent_wakes(now + 70_000).await.unwrap();
    let fired = store
        .agent_wake("timer-after-failed-start")
        .unwrap()
        .unwrap();
    let sequence = fired.last_sequence.expect("the timer fired into the inbox");
    for _ in 0..50 {
        if f.state_of(sequence) == "handled" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(f.state_of(sequence), "handled");
    assert!(f.child_is_running());
}

impl Fixture {
    fn create_one_shot_timer(
        &self,
        id: &str,
        now: u64,
    ) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
        let store = &self.runtime.owned.durable_state_store;
        let Outcome::Task(task) = store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: self.child.clone(),
                prompt: format!("{id}-turn"),
                run: None,
                now,
            })
            .unwrap()
        else {
            panic!("task")
        };
        let timer: AgentWake = serde_json::from_value(serde_json::json!({
        "id": id, "task_id": task.task_id, "room_id": self.room,
        "agent_id": self.child, "registration_id": format!("completion-{id}"),
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

        task
    }
}

#[tokio::test]
async fn refused_one_shot_wake_settles_its_waiting_task_at_exhaustion() {
    use crate::durable_state::agent_lifecycle::ExecutionState;
    let f = Fixture::with_child_provider("dev-stub");
    let store = &f.runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let task = f.create_one_shot_timer("exhausted-timer", now);
    store
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            registrations: vec!["completion-exhausted-timer".into()],
            cursor: 0,
            deadline: Some(now + 600_000),
            reason: "wait for the one-shot timer".into(),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.child.clone(),
            prompt: task.prompt_id.clone(),
            run: "timer-setup-run".into(),
            has_answer: false,
            cancelled: false,
            now,
        })
        .unwrap();
    assert_eq!(
        store.agent_task(&task.task_id).unwrap().unwrap().state,
        ExecutionState::Waiting
    );
    store
        .agent_lifecycle(Operation::FireWakes { now: now + 1_000 })
        .unwrap();
    let wake = store.agent_wake("exhausted-timer").unwrap().unwrap();
    let sequence = wake.last_sequence.expect("the one-shot timer fired");

    let moved = f.remove_workspace();
    let error = f
        .runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not exist"), "{error}");
    // The start is retried with growing pauses; the wake ends only when the attempts are spent.
    assert_eq!(f.state_of(sequence), "pending");
    f.sweep_after_delivery_window();
    assert_eq!(f.state_of(sequence), "pending", "attempts remain");
    f.spend_start_attempts(sequence).await;
    std::fs::rename(&moved, f._worktree.path()).unwrap();
    assert_eq!(f.state_of(sequence), "pending");
    assert_eq!(
        store.agent_task(&task.task_id).unwrap().unwrap().state,
        ExecutionState::Waiting
    );

    f.sweep_after_delivery_window();

    assert_eq!(f.state_of(sequence), "failed");
    let settled = store.agent_task(&task.task_id).unwrap().unwrap();
    assert!(
        matches!(
            settled.state,
            ExecutionState::Done | ExecutionState::Cancelled
        ),
        "an exhausted wake stranded the task in {:?}",
        settled.state
    );
    assert!(
        settled.reason.contains("could not start"),
        "{}",
        settled.reason
    );
    assert!(settled.wait.is_none());
}

#[tokio::test]
async fn refused_wait_deadline_delivers_after_the_workspace_returns() {
    use crate::durable_state::agent_lifecycle::ExecutionState;

    let f = Fixture::with_child_provider("dev-stub");
    let store = &f.runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let task = f.create_one_shot_timer("deadline-recovery", now);
    store
        .agent_lifecycle(Operation::Yield {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            registrations: vec!["completion-deadline-recovery".into()],
            cursor: 0,
            deadline: Some(now + 100),
            reason: "wait for the timer or its deadline".into(),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.child.clone(),
            prompt: task.prompt_id.clone(),
            run: "deadline-setup-run".into(),
            has_answer: false,
            cancelled: false,
            now,
        })
        .unwrap();

    let moved = f.remove_workspace();
    store
        .agent_lifecycle(Operation::Sweep {
            now: now + 100,
            busy_recipients: vec![],
            vault_wait_recipients: vec![],
            held_work: vec![],
        })
        .unwrap();
    let deadline = f
        .child_inbox()
        .into_iter()
        .find(|e| e.kind == "deadline_reached")
        .expect("the live wait reached its deadline");
    let error = f
        .runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not exist"), "{error}");
    f.spend_start_attempts(deadline.sequence).await;
    std::fs::rename(moved, f._worktree.path()).unwrap();

    f.sweep_after_delivery_window();
    assert_eq!(
        store.agent_task(&task.task_id).unwrap().unwrap().state,
        ExecutionState::Waiting
    );
    f.sweep_after_delivery_window();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    assert_eq!(f.state_of(deadline.sequence), "acknowledged");
    assert!(f.child_is_running());
}

impl Fixture {
    /// The child's waiting task whose wake was refused before it started (a declined Vault
    /// unlock) and that was blocked for its owner: the wake stays pending, held for Resume.
    fn blocked_task_with_held_wake(&self) -> (String, u64) {
        let store = &self.runtime.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        let Outcome::Task(task) = store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: self.child.clone(),
                prompt: "blocked-turn".into(),
                run: None,
                now,
            })
            .unwrap()
        else {
            panic!("task")
        };
        let timer: AgentWake = serde_json::from_value(serde_json::json!({
            "id": "held-timer", "task_id": task.task_id, "room_id": self.room,
            "agent_id": self.child, "registration_id": "completion-held-timer",
            "kind": "timer", "label": "check later", "state": "scheduled", "created_at_ms": now,
            "verified_at_ms": null, "next_due_ms": now + 3_600_000, "interval_ms": null,
            "command": [], "match_text": null, "matched_at_ms": null, "pid": null,
            "exit_code": null, "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
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
        store
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                registrations: vec!["completion-held-timer".into()],
                cursor: 0,
                deadline: Some(now + 7_200_000),
                reason: "check later".into(),
                now,
            })
            .unwrap();
        let Outcome::Event(wake) = store
            .agent_lifecycle(Operation::Occur(
                crate::durable_state::agent_lifecycle::occurrence(
                    &self.room,
                    &self.child,
                    "held-source",
                    "held-source-done",
                    "source_completed",
                    serde_json::json!({"task_id": task.task_id, "success": true}),
                ),
            ))
            .unwrap()
        else {
            panic!("event")
        };
        store
            .agent_lifecycle(Operation::Attempt {
                room: self.room.clone(),
                agent: self.child.clone(),
                sequence: wake.sequence,
                prompt: "refused-wake".into(),
                target: None,
                run: None,
                now,
                work: None,
                through: wake.sequence,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Receipt {
                room: self.room.clone(),
                agent: self.child.clone(),
                sequence: wake.sequence,
                state: "rejected".into(),
                now,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Block {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                reason: "The Vault unlock was declined; unlock the Vault and Resume, or Cancel"
                    .into(),
            })
            .unwrap();
        (task.task_id, wake.sequence)
    }
}

/// An urgent message to an idle agent whose blocked task holds a refused wake for its owner's
/// Resume starts a turn of its own; the held wake keeps waiting for the Resume.
#[tokio::test]
async fn an_urgent_message_passes_a_wake_held_for_a_blocked_tasks_resume() {
    let f = Fixture::with_child_provider("dev-stub");
    let (task, held) = f.blocked_task_with_held_wake();
    let message = f.message("urgent-to-blocked", true, false);

    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();

    assert_eq!(f.state_of(message.sequence), "acknowledged");
    assert!(f.child_is_running(), "the urgent message starts a turn");
    assert_eq!(
        f.state_of(held),
        "pending",
        "the held wake waits for Resume"
    );
    let blocked = f
        .runtime
        .owned
        .durable_state_store
        .agent_task(&task)
        .unwrap()
        .unwrap();
    assert_eq!(
        blocked.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Blocked
    );
}

/// A message that is not urgent waits for the next turn, as it does for any agent: the held
/// wake does not start one for it either.
#[tokio::test]
async fn a_queued_message_behind_a_held_wake_starts_nothing() {
    let f = Fixture::with_child_provider("dev-stub");
    let (_, held) = f.blocked_task_with_held_wake();
    let message = f.message("queued-to-blocked", false, false);

    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();

    assert_eq!(f.state_of(message.sequence), "pending");
    assert_eq!(f.state_of(held), "pending");
    assert!(!f.child_is_running());
}

/// An urgent message steers the child's running turn, and that turn ends while the steer waits
/// for its provider lane: the steer is skipped at the native write. The message goes back to
/// pending and is delivered again at once, as a turn of its own, instead of waiting unseen.
#[tokio::test]
async fn a_steer_skipped_at_the_native_write_is_delivered_again() {
    let f = Fixture::with_child_provider("dev-stub");
    let first = f.message("starts-the-turn", true, false);
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    assert_eq!(f.state_of(first.sequence), "acknowledged");
    assert!(f.child_is_running());
    let run = f
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&f.room, &f.child)
        .unwrap();
    let steer = f.message("steers-the-turn", true, false);
    let lane = f.runtime.provider_runtime_lanes.acquire(run.id()).await;
    let delivery = tokio::spawn({
        let runtime = f.runtime.clone();
        let (room, child) = (f.room.clone(), f.child.clone());
        async move { runtime.deliver_agent_inbox(&room, &child).await }
    });
    for _ in 0..50 {
        if f.state_of(steer.sequence) == "submitting" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(f.state_of(steer.sequence), "submitting");
    // The running turn ends while the steer waits for the lane.
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    f.runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &f.child)
        .expect("the child's turn was running");
    drop(lane);
    delivery.await.unwrap().unwrap();

    for _ in 0..50 {
        if f.state_of(steer.sequence) == "acknowledged" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(f.state_of(steer.sequence), "acknowledged");
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let active = f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.child)
        .expect("the message starts the child's next turn");
    assert_eq!(active.id(), steer.transport_prompt_id(false));
    assert_eq!(
        f.child_inbox()
            .iter()
            .filter(|event| event.occurrence_id == steer.occurrence_id)
            .count(),
        1,
        "redelivery keeps the original inbox occurrence"
    );
}
