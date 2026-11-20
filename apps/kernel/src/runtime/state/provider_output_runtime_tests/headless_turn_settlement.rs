//! G-DELIVERY item I: a claude-headless turn ends at the provider's Stop hook, which completes
//! the prompt in the bridge. Its durable task settles at that Stop, so the supervision sweep never
//! finds it Working later and ends it "Interrupted before it finished" (case H).
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, AgentWake, ExecutionState, Operation,
};
use std::io::Write as _;

struct HeadlessTurn {
    _worktree: crate::test_support::TestWorktree,
    _cleanup: InertPtyCleanup,
    runtime: KernelRuntimeState,
    room: String,
    agent: String,
    run: String,
    prompt: crate::session::PromptQueueItem,
    events: std::path::PathBuf,
}

impl HeadlessTurn {
    /// A headless Claude agent whose turn `prompt` is running and remembered by the output pump.
    async fn running(label: &str) -> Self {
        let worktree = crate::test_support::TestWorktree::new(label);
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "headless-turn-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let context = worktree.path().join("context.txt");
        let events = worktree.path().join("events.jsonl");
        std::fs::write(&context, "").unwrap();
        std::fs::write(&events, "").unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "default",
            "haiku",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            format!("{label}-run"),
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: label.into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::from([
                    (
                        "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                        context.display().to_string(),
                    ),
                    (
                        "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                        events.display().to_string(),
                    ),
                ]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.update_provider_run_projection(run.clone());
        spawn_inert_pty_for_run(&mut app, run.id());
        let app = Arc::new(Mutex::new(app));
        let cleanup = InertPtyCleanup {
            app: Arc::clone(&app),
            provider_run_id: run.id().into(),
        };
        let runtime = owned_runtime_state(&app).await;
        let prompt = crate::session::PromptQueueItem::new(
            format!("{label}-prompt"),
            attachment.id(),
            agent.id(),
            "do the work",
            crate::session::PromptStatus::Queued,
        );
        runtime
            .owned
            .submit_local_prepared_prompt_with_queue_policy(
                &crate::app::KernelPreparedPromptSubmission {
                    session_id: session.id().into(),
                    prompt: prompt.clone(),
                    force_queue: false,
                    refresh_projection: true,
                },
                false,
            )
            .unwrap()
            .unwrap();
        runtime
            .owned
            .ensure_agent_task(session.id(), agent.id(), &prompt, Some(run.id()))
            .unwrap();
        let turn = Self {
            _worktree: worktree,
            _cleanup: cleanup,
            runtime,
            room: session.id().into(),
            agent: agent.id().into(),
            run: run.id().into(),
            prompt,
            events,
        };
        turn.pump().await;
        assert_eq!(turn.task().state, ExecutionState::Working);
        turn
    }

    async fn pump(&self) {
        self.runtime
            .pump_owned_provider_output(&self.room, &self.run, vec![], true)
            .await
            .unwrap();
    }

    /// Claude's Stop hook ends the turn, and the next output pass reads it.
    async fn stop(&self) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.events)
            .unwrap();
        writeln!(file, "{}", serde_json::json!({"hook_event_name": "Stop"})).unwrap();
        self.pump().await;
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.room)
            .unwrap();
        assert!(
            self.runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &self.agent)
                .is_none(),
            "the Stop hook completes the turn's prompt"
        );
    }

    fn task(&self) -> AgentTaskExecution {
        self.runtime
            .owned
            .durable_state_store
            .agent_turn_task(&self.room, &self.agent, self.prompt.id())
            .unwrap()
            .expect("the turn's task is recorded")
    }

    /// The turn armed a timer and yielded on it, as `chariox.events.yield` does.
    fn yield_on_a_timer(&self) {
        let task = self.task();
        let now = crate::session::unix_epoch_ms();
        let wake: AgentWake = serde_json::from_value(serde_json::json!({
            "id": "wake-headless", "task_id": task.task_id, "room_id": self.room,
            "agent_id": self.agent, "registration_id": "completion-wake-headless",
            "kind": "timer", "label": "wait", "state": "scheduled",
            "created_at_ms": now, "verified_at_ms": null, "next_due_ms": now + 3_600_000,
            "interval_ms": null, "command": [], "match_text": null, "matched_at_ms": null,
            "pid": null, "exit_code": null, "fire_count": 0, "missed_fires": 0,
            "last_fired_at_ms": null, "last_sequence": null, "last_delivery": null,
            "last_delivered_at_ms": null, "last_acknowledged_at_ms": null, "alerted_sequence": null
        }))
        .unwrap();
        let store = &self.runtime.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::ArmWake {
                task: task.task_id.clone(),
                prompt: self.prompt.id().into(),
                wake: Box::new(wake),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Yield {
                task: task.task_id,
                prompt: self.prompt.id().into(),
                registrations: vec!["completion-wake-headless".into()],
                cursor: 0,
                deadline: Some(now + 3_600_000),
                reason: "Wait for the timer".into(),
                now,
            })
            .unwrap();
    }

    /// A supervision sweep well past the delivery timeout, as on an idle agent.
    async fn sweep_later(&self) {
        self.runtime
            .sweep_agent_lifecycle_at_for_test(
                crate::session::unix_epoch_ms()
                    + crate::durable_state::agent_lifecycle::DELIVERY_TIMEOUT_MS
                    + 60_000,
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_headless_turn_settles_its_task_at_stop() {
    let turn = HeadlessTurn::running("headless-turn-done").await;
    turn.stop().await;
    let task = turn.task();
    assert_eq!(
        task.state,
        ExecutionState::Done,
        "the Stop hook left the turn's task {:?} ({:?}) for the sweep",
        task.state,
        task.reason
    );
    turn.sweep_later().await;
    let task = turn.task();
    assert_eq!(task.state, ExecutionState::Done, "{:?}", task.reason);
}

#[tokio::test]
async fn a_headless_turn_that_yielded_waits_at_stop_and_is_never_interrupted_later() {
    let turn = HeadlessTurn::running("headless-turn-yield").await;
    turn.yield_on_a_timer();
    turn.stop().await;
    let task = turn.task();
    assert_eq!(
        task.state,
        ExecutionState::Waiting,
        "the yield did not commit at Stop: {:?} ({:?})",
        task.state,
        task.reason
    );
    turn.sweep_later().await;
    let task = turn.task();
    assert_eq!(task.state, ExecutionState::Waiting, "{:?}", task.reason);
    assert!(
        !task.reason.contains("Interrupted"),
        "a turn that ended normally got an Interrupted card: {:?}",
        task.reason
    );
}
