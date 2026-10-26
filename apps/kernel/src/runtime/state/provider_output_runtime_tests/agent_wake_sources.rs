//! MP-08 / MP-09 / MP-10 / MP-11 A02/A03: the wake source interface against
//! a real runtime state, and the `chariox.events.*` tools through the runtime
//! MCP entry. Regression coverage, not acceptance.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, AgentWake, ExecutionState, Operation, Outcome, WakeEnd,
};
use crate::runtime::state::wake_driver::{
    Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence, WakeSink,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;

const KIND: &str = "test-source";

struct TestDriver {
    sink: WakeSink,
    armed: StdMutex<Vec<String>>,
    fail_arm: AtomicBool,
    cancel_pending: AtomicBool,
    lost: AtomicBool,
    /// Holds the next liveness probe until released, as a slow source would.
    stall: StdMutex<
        Option<(
            std::sync::mpsc::SyncSender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}

impl WakeDriver for TestDriver {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError> {
        if self.fail_arm.load(Ordering::Acquire) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "test source refused to start",
            ));
        }
        self.armed.lock().unwrap().push(wake.id.clone());
        self.sink.verified(&wake.id)
    }
    fn cancel(&self, _wake: &AgentWake) -> Result<Cancel, DaemonError> {
        Ok(if self.cancel_pending.load(Ordering::Acquire) {
            Cancel::Pending
        } else {
            Cancel::Done
        })
    }
    fn recover(&self, _wake: &AgentWake) -> Recovery {
        Recovery::Lost("test source does not survive a restart".into())
    }
    fn liveness(&self, _wake: &AgentWake) -> Liveness {
        if let Some((entered, release)) = self.stall.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = release.recv();
        }
        if self.lost.load(Ordering::Acquire) {
            Liveness::Lost("test source vanished".into())
        } else {
            Liveness::Live
        }
    }
}

struct Fixture {
    runtime: KernelRuntimeState,
    driver: Arc<TestDriver>,
    room: String,
    agent: String,
    run: String,
    prompt_id: String,
    app: Arc<Mutex<DaemonApp>>,
    _pty: InertPtyCleanup,
    _worktree: crate::test_support::TestWorktree,
}

/// `running_prompt` leaves a real turn running for the agent, as a provider
/// run has while it calls tools; without it the agent is idle.
async fn fixture(name: &str, running_prompt: bool) -> Fixture {
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .unwrap();
    let prompt_id = if running_prompt {
        app.update_provider_run_projection(run.clone());
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-wake-sources",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "wait for the test source",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("first prompt starts immediately");
        };
        prompt.id().to_string()
    } else {
        String::new()
    };
    spawn_inert_pty_for_run(&mut app, run.id());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let driver = Arc::new(TestDriver {
        sink: runtime.wake_sink(KIND),
        armed: Default::default(),
        fail_arm: AtomicBool::new(false),
        cancel_pending: AtomicBool::new(false),
        lost: AtomicBool::new(false),
        stall: Default::default(),
    });
    runtime.register_wake_driver(driver.clone());
    Fixture {
        runtime,
        driver,
        room: session.id().into(),
        agent: agent.id().into(),
        run: run.id().into(),
        prompt_id,
        _pty: InertPtyCleanup {
            app: app.clone(),
            provider_run_id: run.id().into(),
        },
        app,
        _worktree: worktree,
    }
}

impl Fixture {
    fn store(&self) -> &crate::durable_state::DurableKernelStateStore {
        &self.runtime.owned.durable_state_store
    }
    /// Starts a task for `prompt` the way prompt dispatch does.
    fn begin(&self, prompt: &str) -> AgentTaskExecution {
        let Outcome::Task(task) = self
            .store()
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: self.agent.clone(),
                prompt: prompt.into(),
                run: Some(self.run.clone()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("begin returns the task");
        };
        task
    }
    fn arm(&self, task: &AgentTaskExecution, id: &str) -> Result<AgentWake, DaemonError> {
        let mut wake = self.wake(task, id);
        wake.state = "starting".into();
        self.runtime.arm_wake_source(task, &wake)?;
        wake = self.store().agent_wake(id).unwrap().unwrap();
        Ok(wake)
    }
    fn wake(&self, task: &AgentTaskExecution, id: &str) -> AgentWake {
        serde_json::from_value(serde_json::json!({
            "id": id, "task_id": task.task_id, "room_id": self.room, "agent_id": self.agent,
            "registration_id": format!("completion-{id}"), "kind": KIND, "label": "test source",
            "state": "starting", "created_at_ms": crate::session::unix_epoch_ms(),
            "verified_at_ms": null, "next_due_ms": null, "interval_ms": null,
            "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
            "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
            "last_sequence": null, "last_delivery": null, "last_delivered_at_ms": null,
            "last_acknowledged_at_ms": null, "alerted_sequence": null
        }))
        .unwrap()
    }
    fn wait_on(&self, task: &AgentTaskExecution, registration: &str) {
        let now = crate::session::unix_epoch_ms();
        self.store()
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                registrations: vec![registration.into()],
                cursor: 0,
                deadline: now + 60_000,
                reason: "Wait for the test source".into(),
                now,
            })
            .unwrap();
        self.store()
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: self.agent.clone(),
                prompt: task.prompt_id.clone(),
                run: self.run.clone(),
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap();
    }
    fn task(&self, id: &str) -> AgentTaskExecution {
        self.store()
            .agent_tasks(Some(&self.room), Some(&self.agent))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == id)
            .unwrap()
    }
    fn wake_state(&self, id: &str) -> String {
        self.store().agent_wake(id).unwrap().unwrap().state
    }
    fn occurrence(&self, wake: &str, id: &str, end: Option<WakeEnd>) -> WakeOccurrence {
        WakeOccurrence {
            wake_id: wake.into(),
            occurrence_id: id.into(),
            kind: "output_matched".into(),
            payload: serde_json::json!({"line": "ready"}),
            summary: Some("matched".into()),
            end,
        }
    }
    fn notices(&self) -> String {
        self.runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&self.room, Some(&self.agent))
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == crate::history::SessionHistoryEntryKind::Notice)
            .map(|e| e.text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[tokio::test]
async fn ev13_driver_occurrence_wakes_the_waiting_task_once() {
    let f = fixture("ev13-driver-occurrence", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    assert_eq!(
        wake.state, "running",
        "the driver's receipt is proof of life"
    );
    assert!(wake.verified_at_ms.is_some());
    assert_eq!(f.driver.armed.lock().unwrap().as_slice(), ["wake-a"]);
    f.wait_on(&task, &wake.registration_id);
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);

    let sink = f.runtime.wake_sink(KIND);
    sink.occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    sink.occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    // The sink schedules the delivery itself; the item leaves `pending`
    // when the idle agent has accepted the wake turn.
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if events.iter().all(|e| e.state == "accepted") && !events.is_empty() {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "wake was not delivered: {:?} queued={:?} active={:?}",
            f.store().agent_inbox(&f.room, &f.agent, 0).unwrap(),
            f.runtime
                .owned
                .session_store
                .get_session(&f.room)
                .unwrap()
                .queued_prompts_for_agent(&f.agent)
                .map(|q| q
                    .iter()
                    .map(|p| (p.id().to_string(), format!("{:?}", p.status())))
                    .collect::<Vec<_>>()),
            f.runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &f.runtime.owned.session_store.get_session(&f.room).unwrap(),
                    &f.agent
                )
                .map(|p| (p.id().to_string(), format!("{:?}", p.status())))
        )
    });
    assert_eq!(events.len(), 1, "a replayed occurrence is one admission");
    assert_eq!(events[0].state, "accepted");
    let woken = f.task(&task.task_id);
    assert_eq!(woken.state, ExecutionState::Working);
    assert_eq!(woken.prompt_id, events[0].prompt_id.clone().unwrap());
    assert_eq!(
        f.wake_state("wake-a"),
        "running",
        "the source keeps running"
    );

    sink.occurred(f.occurrence("wake-a", "exit", Some(WakeEnd::Succeeded)))
        .unwrap();
    assert_eq!(f.wake_state("wake-a"), "exited");
}

#[tokio::test]
async fn failed_wake_dispatch_settles_the_admitted_prompt_and_keeps_the_receipt() {
    let f = fixture("ev13-dispatch-failure", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);
    // The provider process is gone before the wake turn can reach it.
    f.app.lock().await.pty_mut().remove_process(&f.run).unwrap();

    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence("wake-a", "line-1", None))
        .unwrap();
    // The attempt returns the event to pending with its receipt; the admitted
    // wake turn must not stay the active prompt of an agent no provider serves.
    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
            let active = f
                .runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &f.agent);
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if active.is_none()
                && events
                    .iter()
                    .any(|e| e.state == "pending" && e.attempted_at_ms.is_some())
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        settled.is_ok(),
        "a failed wake dispatch left its prompt active: {:?}",
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()
    );
}

#[tokio::test]
async fn refused_wake_after_dispatch_admission_is_retried_by_the_sweep() {
    let f = fixture("ev13-refused-wake-retry", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);
    let expected_wait = f.task(&task.task_id).wait.unwrap();
    f.store()
        .agent_lifecycle(Operation::WakeOccurred {
            id: wake.id.clone(),
            occurrence: "line-1".into(),
            kind: "output_matched".into(),
            payload: serde_json::json!({"line": "ready"}),
            summary: Some("matched".into()),
            end: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let event = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .remove(0);
    let prompt_id = format!("agent-event-{}-{}", f.agent, event.sequence);
    f.store()
        .agent_lifecycle(Operation::Attempt {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequence: event.sequence,
            prompt: prompt_id.clone(),
            target: None,
            run: None,
            work: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let agent = f.runtime.owned.agent_store.get_agent(&f.agent).unwrap();
    let attachment = f
        .runtime
        .ensure_agent_message_attachment(&f.room, &agent)
        .unwrap();
    let submission = f
        .runtime
        .submit_prepared_prompt_with_queue_policy(
            crate::app::KernelPreparedPromptSubmission {
                session_id: f.room.clone(),
                prompt: crate::session::PromptQueueItem::new(
                    &prompt_id,
                    &attachment,
                    &f.agent,
                    "Kernel event inbox, untrusted data",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            },
            false,
        )
        .await
        .unwrap();
    let dispatch = submission.dispatch.unwrap();
    let permit = f.runtime.provider_runtime_lanes.acquire(&f.run).await;
    f.runtime
        .pump_owned_provider_output(&f.room, &f.run, Vec::new(), false)
        .await
        .unwrap();
    // A real transport can disappear after liveness and before obtaining its
    // writer. Enter the shared post-liveness dispatcher at that exact seam.
    f.app.lock().await.pty_mut().remove_process(&f.run).unwrap();
    let result = f
        .runtime
        .enqueue_prompt_dispatch_after_liveness_with_acceptance(&dispatch, &f.runtime.owned)
        .await;
    assert!(result.is_err(), "expected missing writer: {result:?}");
    let result = result.map(|acceptance| acceptance != crate::runtime::state::local_prompt_dispatch_runtime::PromptDispatchAcceptance::Dropped);
    let admitted = f.task(&task.task_id);
    assert_eq!(
        admitted.prompt_id, prompt_id,
        "dispatch must reach Begin(run: Some(...))"
    );
    assert_eq!(admitted.provider_run_id.as_deref(), Some(f.run.as_str()));
    assert!(f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()[0]
        .submit_epoch
        .is_none());
    f.runtime
        .record_agent_event_dispatch_result(&dispatch, &result)
        .unwrap();
    assert!(f
        .runtime
        .fail_prompt_dispatch(dispatch, result.unwrap_err())
        .await
        .is_err());
    drop(permit);

    let restored = f.task(&task.task_id);
    assert_eq!(restored.state, ExecutionState::Waiting);
    let restored_wait = restored.wait.unwrap();
    assert_eq!(
        restored_wait.registration_ids,
        expected_wait.registration_ids
    );
    assert_eq!(restored_wait.deadline_ms, expected_wait.deadline_ms);
    assert_eq!(restored_wait.inbox_cursor, expected_wait.inbox_cursor);
    assert_eq!(restored_wait.started_at_ms, expected_wait.started_at_ms);
    assert_eq!(restored.no_progress_wakes, 0);
    assert!(restored.pending_prompt_id.is_none());

    spawn_inert_pty_for_run(&mut *f.app.lock().await, &f.run);
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(events.len(), 1, "retry the original occurrence");
    assert_eq!(events[0].state, "accepted");
    assert!(events[0].submit_epoch.is_some());
    let retried = f.task(&task.task_id);
    assert_eq!(retried.state, ExecutionState::Working);
    assert_eq!(retried.no_progress_wakes, 1);
    assert!(retried.pending_prompt_id.is_none());
    assert!(
        retried.wait.is_none(),
        "the provider-I/O fence consumes the old wait"
    );
    f.wait_on(&retried, &wake.registration_id);
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
}

#[tokio::test]
async fn ev13_forged_source_identity_is_refused_before_the_ledger() {
    let f = fixture("ev13-forged-source", false).await;
    let task = f.begin("turn-1");
    f.arm(&task, "wake-a").unwrap();

    let timer_sink = f.runtime.wake_sink("timer");
    assert!(timer_sink.verified("wake-a").is_err());
    assert!(timer_sink
        .occurred(f.occurrence("wake-a", "forged", None))
        .is_err());

    let sink = f.runtime.wake_sink(KIND);
    for reserved in [
        "source_completed",
        "source_lost",
        "deadline_reached",
        "wait_recheck",
        "delivery_corrupt",
        "timer_fired",
    ] {
        let mut occurrence = f.occurrence("wake-a", reserved, None);
        occurrence.kind = reserved.into();
        assert!(
            sink.occurred(occurrence).is_err(),
            "a driver cannot report the ledger's own kind `{reserved}`"
        );
    }
    assert!(f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .is_empty());
    assert_eq!(f.wake_state("wake-a"), "running");
}

#[tokio::test]
async fn ev13_failed_arm_leaves_no_armed_wake() {
    let f = fixture("ev13-failed-arm", false).await;
    let task = f.begin("turn-1");
    f.driver.fail_arm.store(true, Ordering::Release);
    assert!(f.arm(&task, "wake-a").is_err());
    assert_eq!(f.wake_state("wake-a"), "cancelled");
    assert!(!f
        .task(&task.task_id)
        .obligations
        .iter()
        .any(|o| o.status == "open"));
    let unregistered = AgentWake {
        kind: "unregistered".into(),
        ..f.wake(&task, "wake-b")
    };
    assert!(f.runtime.arm_wake_source(&task, &unregistered).is_err());
    assert!(f.store().agent_wake("wake-b").unwrap().is_none());
}

#[tokio::test]
async fn ev13_cancel_done_and_pending() {
    let f = fixture("ev13-cancel", false).await;
    let task = f.begin("turn-1");
    let done = f.arm(&task, "wake-done").unwrap();
    f.runtime
        .cancel_wake_source(&done, &task.task_id, Some(&task.prompt_id))
        .unwrap();
    assert_eq!(f.wake_state("wake-done"), "cancelled");

    f.driver.cancel_pending.store(true, Ordering::Release);
    let pending = f.arm(&task, "wake-pending").unwrap();
    f.runtime
        .cancel_wake_source(&pending, &task.task_id, Some(&task.prompt_id))
        .unwrap();
    assert_eq!(
        f.wake_state("wake-pending"),
        "cancelling",
        "a stop request is not a stop"
    );
    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence("wake-pending", "stopped", Some(WakeEnd::Failed)))
        .unwrap();
    assert_ne!(f.wake_state("wake-pending"), "cancelling");
}

// D2: an ordinary turn whose settlement was lost (for example by a restart)
// ends as a stopped turn; it never asks the owner for a decision.
#[tokio::test]
async fn sweep_ends_a_stranded_plain_turn_without_asking_the_owner() {
    let f = fixture("sweep-plain-turn", false).await;
    let long_ago = crate::session::unix_epoch_ms() - 10 * 60 * 1000;
    f.store()
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: "stranded".into(),
            run: Some(f.run.clone()),
            now: long_ago,
        })
        .unwrap();

    f.runtime.sweep_agent_lifecycle().await.unwrap();

    let task = f
        .store()
        .agent_tasks(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == "stranded")
        .unwrap();
    assert_eq!(task.state, ExecutionState::Cancelled);
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(session.active_interactions().is_empty());
}

#[tokio::test]
async fn ev13_lost_source_raises_a_visible_alert_and_one_event() {
    let f = fixture("ev13-lost-source", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);

    f.driver.lost.store(true, Ordering::Release);
    f.runtime
        .sweep_agent_wakes(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    f.runtime
        .sweep_agent_wakes(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    assert_eq!(f.wake_state("wake-a"), "lost");
    assert!(
        f.notices().contains("was lost (test source vanished)"),
        "the loss is shown to the room: {}",
        f.notices()
    );
    assert_eq!(
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap().len(),
        1,
        "the waiting task hears about it exactly once"
    );
}

async fn call(
    f: &Fixture,
    token: &str,
    tool: &str,
    task: &AgentTaskExecution,
    extra: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut args = serde_json::json!({"task_id": task.task_id, "origin_prompt_id": task.prompt_id});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    f.runtime
        .dispatch_authenticated_runtime_tool_call(token, tool, args)
        .await
}

#[tokio::test]
async fn event_tools_arm_wait_and_cancel_through_the_runtime_mcp_entry() {
    let f = fixture("event-tools-mcp", true).await;
    let task = f.begin(&f.prompt_id);
    let token = {
        let app = f.app.lock().await;
        app.providers
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .expect("runtime MCP auth")
            .to_string()
    };
    let listed: Vec<String> = f
        .runtime
        .runtime_tool_specs_for_auth_token(&token)
        .into_iter()
        .map(|s| s.name)
        .filter(|n| n.starts_with("chariox.events."))
        .collect();
    assert_eq!(
        listed,
        [
            "subscribe",
            "unsubscribe",
            "subscriptions",
            "inbox",
            "ack",
            "yield",
            "timer",
            "process",
            "terminal",
            "cancel_wake",
            "wakes",
            "blocked"
        ]
        .map(|n| format!("chariox.events.{n}"))
    );

    let scheduler = {
        let runtime = f.runtime.clone();
        tokio::spawn(async move { runtime.run_agent_wake_scheduler().await })
    };
    let timer = call(
        &f,
        &token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"check in","delay_ms":1_000}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(timer["scheduler_confirmed"], true, "{timer}");
    let registration = timer["registration_id"].as_str().unwrap().to_string();
    let wake_id = timer["wake_id"].as_str().unwrap().to_string();

    let now = crate::session::unix_epoch_ms();
    for bad in [
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"deadline_ms":now - 1,"reason":"late"}),
        serde_json::json!({"registration_ids":["no-such-registration"],"inbox_cursor":0,"deadline_ms":now + 60_000,"reason":"unknown"}),
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"reason":"no deadline"}),
    ] {
        assert!(
            call(&f, &token, "chariox.events.yield", &task, bad)
                .await
                .is_err(),
            "an invalid wait is refused"
        );
    }
    // A12-S02: a duration passed as deadline_ms is refused with what to send instead.
    let duration = call(
        &f,
        &token,
        "chariox.events.yield",
        &task,
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"deadline_ms":3_600_000,"reason":"one hour"}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        duration.contains("absolute Unix time in milliseconds") && duration.contains("now_ms"),
        "{duration}"
    );
    let mut stale = task.clone();
    stale.prompt_id = "an-older-turn".into();
    assert!(call(
        &f,
        &token,
        "chariox.events.wakes",
        &stale,
        serde_json::json!({})
    )
    .await
    .is_err());

    let listing = call(
        &f,
        &token,
        "chariox.events.wakes",
        &task,
        serde_json::json!({}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(listing["wakes"][0]["id"], wake_id.as_str());

    call(
        &f,
        &token,
        "chariox.events.yield",
        &task,
        serde_json::json!({"registration_ids":[registration],"inbox_cursor":0,"deadline_ms":now + 60_000,"reason":"waiting for the clock"}),
    )
    .await
    .unwrap();
    assert!(
        call(
            &f,
            &token,
            "chariox.events.wakes",
            &task,
            serde_json::json!({})
        )
        .await
        .is_err(),
        "once the wait is accepted the turn must end"
    );

    f.store()
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: task.prompt_id.clone(),
            run: f.run.clone(),
            has_answer: true,
            cancelled: false,
            now,
        })
        .unwrap();
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Waiting);
    scheduler.abort();
}

/// A10-S02: a message turn that reuses the ids of the waiting task it
/// interrupted is told that the wait continues and given its own ids, not a
/// bare refusal it escalates to the owner.
#[tokio::test]
async fn a_turn_reusing_an_earlier_waiting_tasks_ids_is_told_its_own() {
    let f = fixture("earlier-turn-origin", true).await;
    let earlier = f.begin("earlier-turn");
    f.arm(&earlier, "earlier-timer").unwrap();
    f.wait_on(&earlier, "completion-earlier-timer");
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let now = crate::session::unix_epoch_ms();
    let refusal = call(
        &f,
        &token,
        "chariox.events.yield",
        &earlier,
        serde_json::json!({"registration_ids":["completion-earlier-timer"],"inbox_cursor":0,"deadline_ms":now + 60_000,"reason":"keep waiting"}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        refusal.contains("still waiting")
            && refusal.contains(&format!("`{}`", task.task_id))
            && refusal.contains(&format!("`{}`", f.prompt_id)),
        "{refusal}"
    );
    assert_eq!(f.task(&earlier.task_id).state, ExecutionState::Waiting);
}

#[tokio::test]
async fn event_tools_cancel_a_timer_and_block_for_the_owner() {
    let f = fixture("event-tools-cancel", true).await;
    let task = f.begin(&f.prompt_id);
    let token = {
        let app = f.app.lock().await;
        app.providers
            .get_run(&f.run)
            .unwrap()
            .runtime_mcp_auth_token()
            .unwrap()
            .to_string()
    };
    let timer = call(
        &f,
        &token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"never needed","delay_ms":3_600_000}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = timer["wake_id"].as_str().unwrap();
    let cancelled = call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id": wake_id}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(cancelled["settled"], true, "{cancelled}");
    assert_eq!(f.wake_state(wake_id), "cancelled");
    assert!(call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id": "wake-of-nobody"}),
    )
    .await
    .is_err());

    call(
        &f,
        &token,
        "chariox.events.blocked",
        &task,
        serde_json::json!({"reason": "Need the owner to pick a branch"}),
    )
    .await
    .unwrap();
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Blocked);
}

/// A03-S04: an agent that cancels a wake that already ended (here: lost) is
/// told it is settled and how it ended, so it does not arm a replacement.
#[tokio::test]
async fn cancelling_a_wake_that_already_ended_answers_settled_with_its_final_state() {
    let f = fixture("event-tools-cancel-settled", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let wake = f.arm(&task, "wake-lost").unwrap();
    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence(&wake.id, "gone", Some(WakeEnd::Lost)))
        .unwrap();
    assert_eq!(f.wake_state("wake-lost"), "lost");

    let cancelled = call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id": "wake-lost"}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(cancelled["settled"], true, "{cancelled}");
    assert_eq!(cancelled["final_state"], "lost", "{cancelled}");
    assert_eq!(f.wake_state("wake-lost"), "lost", "a lost wake stays lost");
}

/// A02-S03: an urgent message to an agent its owner stopped does not start a
/// turn; it waits, shown once to the room, until the owner prompts it again.
#[tokio::test]
async fn urgent_message_to_an_owner_stopped_agent_waits_for_the_owner() {
    let f = fixture("a02-stopped-receiver", false).await;
    let stopped = f.begin("stopped-turn");
    f.store()
        .agent_lifecycle(Operation::CancelTask {
            task: stopped.task_id.clone(),
            owner: "local".into(),
            revision: stopped.revision,
        })
        .unwrap();
    let mut message = crate::durable_state::agent_lifecycle::occurrence(
        &f.room,
        &f.agent,
        "peer-agent",
        "urgent-after-stop",
        "message",
        serde_json::json!({"message": "status now"}),
    );
    message.urgent = true;
    let Outcome::Event(message) = f
        .store()
        .agent_lifecycle(Operation::Occur(message))
        .unwrap()
    else {
        panic!("event")
    };
    let inbox_state = |f: &Fixture| {
        f.store()
            .agent_inbox(&f.room, &f.agent, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.sequence == message.sequence)
            .unwrap()
    };

    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();

    let held = inbox_state(&f);
    assert_eq!(held.state, "pending", "{held:?}");
    assert!(
        held.prompt_id.is_none(),
        "no turn started for a stopped agent: {held:?}"
    );
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(f
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &f.agent)
        .is_none());
    let notices = f.notices();
    assert_eq!(
        notices.matches("which you stopped").count(),
        1,
        "the waiting message is shown once: {notices}"
    );

    // The owner's next prompt lifts the stop; the message follows it.
    let next = f.begin("owner-next");
    f.store()
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: next.prompt_id.clone(),
            run: f.run.clone(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    assert!(
        inbox_state(&f).prompt_id.is_some(),
        "delivered after the owner's prompt: {:?}",
        inbox_state(&f)
    );
}

/// amsudo r1: a sweep that stalls on its sources commits a `now` older than a
/// newer sweep's. That is not a clock rollback and must not wake a live wait
/// whose deadline has not passed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_sweep_does_not_wake_a_live_wait_early() {
    let f = fixture("stalled-sweep", false).await;
    let task = f.begin("turn-1");
    let wake = f.arm(&task, "wake-a").unwrap();
    f.wait_on(&task, &wake.registration_id);
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    *f.driver.stall.lock().unwrap() = Some((entered_tx, release_rx));
    let runtime = f.runtime.clone();
    let stalled = tokio::spawn(async move { runtime.sweep_agent_lifecycle().await });
    tokio::task::spawn_blocking(move || {
        entered_rx.recv_timeout(std::time::Duration::from_secs(20))
    })
    .await
    .unwrap()
    .expect("the sweep probes the source before it commits");
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.store()
        .agent_lifecycle(Operation::Sweep {
            now: crate::session::unix_epoch_ms(),
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        })
        .unwrap();
    release_tx.send(()).unwrap();
    stalled.await.unwrap().unwrap();
    let early = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "deadline_reached")
        .count();
    assert_eq!(early, 0, "no deadline has passed");
}

async fn runtime_token(f: &Fixture) -> String {
    let app = f.app.lock().await;
    app.providers
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .expect("runtime MCP auth")
        .to_string()
}

fn open_terminal(f: &Fixture, command: &str) -> String {
    f.runtime
        .owned
        .room_terminals
        .open(crate::room_terminal::OpenRoomTerminal {
            session_id: f.room.clone(),
            working_directory: f.runtime.room_project_directory(&f.room).unwrap(),
            cols: 100,
            rows: 30,
            title: None,
            opened_by: crate::room_terminal::TerminalActor::Agent(f.agent.clone()),
            command: Some(vec!["sh".into(), "-c".into(), command.into()]),
        })
        .unwrap()
        .terminal_id
}

/// TERM-AGENT-WAIT notify half: a command that finishes while the agent
/// waits on the event reaches it once, with the exit code and the screen.
/// Delivering the wake turn is the ledger's and is covered by the driver tests above.
#[tokio::test]
async fn terminal_finish_reports_once_to_the_waiting_agent_with_exit_code_and_screen() {
    let f = fixture("terminal-wake-finish", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let terminal = open_terminal(&f, "sleep 1; echo SCREEN-MARK; exit 4");
    let armed = call(
        &f,
        &token,
        "chariox.events.terminal",
        &task,
        serde_json::json!({"label":"build","terminal_id":terminal,"until":["finished"]}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = armed["wake_id"].as_str().unwrap().to_string();
    assert_eq!(f.wake_state(&wake_id), "running");
    f.wait_on(&task, armed["registration_id"].as_str().unwrap());

    let events = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if !events.is_empty() {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the finished terminal reports to the waiting agent");
    assert_eq!(events.len(), 1, "reported once");
    let answer = &events[0].payload["public_answer"];
    assert_eq!(answer["exit_code"], 4, "{answer}");
    assert!(
        answer["screen_text"]
            .as_str()
            .unwrap()
            .contains("SCREEN-MARK"),
        "{answer}"
    );
    assert_eq!(f.wake_state(&wake_id), "exited");
}

/// A fired watch is already removed from the terminal, so a rejected ledger
/// write must not discard the occurrence: it is retried until it commits.
#[tokio::test]
async fn a_rejected_terminal_occurrence_is_retried_until_the_ledger_commits() {
    let f = fixture("terminal-wake-retry", true).await;
    let received = f._worktree.path().join("provider-input");
    {
        let mut app = f.app.lock().await;
        app.pty_mut().remove_process(&f.run).unwrap();
        app.pty_mut()
            .spawn(crate::pty::PtySpawnRequest {
                process_key: f.run.clone(),
                provider_run_id: f.run.clone(),
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "stty raw -echo; exec cat > \"$1\"".into(),
                    "wake-provider".into(),
                    received.display().to_string(),
                ],
                env: Default::default(),
                env_remove: Vec::new(),
                working_directory: None,
                cols: 80,
                rows: 24,
            })
            .unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !received.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the provider transport is ready to receive the wake turn");
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    f.runtime
        .terminal_wake_driver
        .as_ref()
        .expect("the runtime registers the terminal wake driver")
        .sink()
        .reject_next_occurrences(2);
    let terminal = open_terminal(&f, "sleep 1; echo RETRY-MARK; exit 0");
    let armed = call(
        &f,
        &token,
        "chariox.events.terminal",
        &task,
        serde_json::json!({"label":"build","terminal_id":terminal,"until":["finished"]}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = armed["wake_id"].as_str().unwrap().to_string();
    f.wait_on(&task, armed["registration_id"].as_str().unwrap());
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    f.runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &f.agent)
        .expect("the yielding provider turn ends before its terminal finishes");

    let events = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if !events.is_empty() {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the rejected occurrence is delivered once the writer accepts it again");
    assert_eq!(events.len(), 1, "exactly one occurrence");
    assert_eq!(f.wake_state(&wake_id), "exited");
    // No lifecycle sweep runs here: the retry itself must schedule delivery,
    // and the provider process must read the wake turn from its transport.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let input = std::fs::read_to_string(&received).unwrap();
            let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
            if input.contains("RETRY-MARK") && events[0].state == "accepted" {
                assert_eq!(events.len(), 1, "exactly one delivered occurrence");
                let woken = f.task(&task.task_id);
                assert_eq!(woken.state, ExecutionState::Working);
                assert_eq!(woken.prompt_id, events[0].prompt_id.clone().unwrap());
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the retry delivers the wake turn to the provider without a sweep");
}

#[tokio::test]
async fn terminal_wake_refuses_a_terminal_that_is_not_running_here() {
    let f = fixture("terminal-wake-unknown", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let error = call(
        &f,
        &token,
        "chariox.events.terminal",
        &task,
        serde_json::json!({"label":"x","terminal_id":"term-nope","until":["finished"]}),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("no running terminal"), "{error}");
    assert!(f
        .store()
        .agent_wakes(Some(&f.room), Some(&f.agent))
        .unwrap()
        .iter()
        .all(|w| w.kind != "terminal" || w.state != "running"));
}

/// The terminal wake driver holds the runtime and the runtime owns the Room terminal service
/// and the wake registry the driver is registered in. Dropping the runtime must free all of
/// it, or a kernel rebuilt in the same process can never take over the durable state lock.
#[tokio::test]
async fn dropping_the_runtime_frees_the_terminal_wake_driver() {
    let _worktree = crate::test_support::TestWorktree::new("terminal-driver-drop");
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap(),
    ));
    let runtime = owned_runtime_state(&app).await;
    let driver = Arc::downgrade(
        runtime
            .terminal_wake_driver
            .as_ref()
            .expect("the runtime registers the terminal wake driver"),
    );
    let kernel = Arc::downgrade(&app);
    drop(runtime);
    drop(app);
    assert!(
        driver.upgrade().is_none(),
        "the driver outlived its runtime"
    );
    assert!(
        kernel.upgrade().is_none(),
        "the kernel outlived its runtime"
    );
}

#[path = "async_popup.rs"]
mod async_popup;
