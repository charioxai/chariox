//! A sub-agent whose turn ends while it holds a live wake source is still working: its parent
//! hears one `child_finished` when no wake source remains and its last turn ended clean.
use super::*;
use crate::durable_state::agent_lifecycle::InboxEvent;

/// A parent parked on its sub-agent `agent`, which has a running turn and no task yet.
async fn child_fixture(name: &str) -> Fixture {
    let worktree = crate::test_support::TestWorktree::new(name);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    // Spawning a child needs the parent's live turn.
    crate::test_support::admit_room_test_turn(&mut app, session.id(), parent.id());
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-watch-open",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let child = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("child".to_owned())
                .with_parent_agent_id(parent.id())
                .with_spawn_key(format!("spawn-{name}")),
        )
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
            .with_agent_id(child.id()),
        )
        .unwrap();
    app.update_provider_run_projection(run.clone());
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(child.id()),
            "wait for the test source",
            Vec::new(),
        )
        .unwrap()
    else {
        panic!("the child starts its prompt");
    };
    let parent_run = app
        .providers()
        .get_run_for_agent(session.id(), parent.id())
        .expect("the admitted turn has a run");
    spawn_inert_pty_for_run(&mut app, run.id());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let driver = Arc::new(TestDriver {
        sink: runtime.wake_sink(KIND),
        armed: Default::default(),
        fail_arm: AtomicBool::new(false),
        cancel_pending: AtomicBool::new(false),
        lost: AtomicBool::new(false),
        lost_ids: Default::default(),
        stall: Default::default(),
    });
    runtime.register_wake_driver(Arc::downgrade(&driver) as std::sync::Weak<dyn WakeDriver>);
    // The parent ended its turn and parked on its child.
    runtime
        .owned
        .complete_local_prompt_without_advance(session.id(), parent.id(), Some(parent_run.id()))
        .unwrap();
    Fixture {
        runtime,
        driver,
        room: session.id().into(),
        agent: child.id().into(),
        parent: Some(parent.id().into()),
        run: run.id().into(),
        prompt_id: prompt.id().into(),
        _pty: InertPtyCleanup {
            app: app.clone(),
            provider_run_id: run.id().into(),
        },
        app,
        _worktree: worktree,
    }
}

impl Fixture {
    fn child_finished(&self) -> Vec<InboxEvent> {
        self.store()
            .agent_inbox(&self.room, self.parent.as_ref().unwrap(), 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "child_finished")
            .collect()
    }

    /// Ends the child's running turn on `prompt` the way the output pump does: the ledger
    /// settles it, the prompt completes, and the task's end is published.
    async fn end_turn(&self, prompt: &str) -> AgentTaskExecution {
        let Outcome::Settled { task, correction } = self
            .store()
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: self.agent.clone(),
                prompt: prompt.into(),
                run: self.run.clone(),
                has_answer: true,
                cancelled: false,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("settled")
        };
        self.runtime
            .owned
            .complete_local_prompt_without_advance(&self.room, &self.agent, Some(&self.run))
            .unwrap();
        let source = crate::session::PromptQueueItem::new(
            prompt,
            "client-watch-open",
            &self.agent,
            "",
            crate::session::PromptStatus::Completed,
        );
        self.runtime
            .finish_agent_task_settlement(Some((task.clone(), correction)), &source)
            .await
            .unwrap();
        task
    }

    /// The wake's source ends; the turn the kernel starts for it is the task's next turn.
    async fn wake_ends(&self, wake: &str, task: &AgentTaskExecution) -> AgentTaskExecution {
        self.runtime
            .wake_sink(KIND)
            .occurred(self.occurrence(wake, "exit", Some(WakeEnd::Succeeded)))
            .unwrap();
        self.next_turn_of(task).await
    }

    async fn next_turn_of(&self, task: &AgentTaskExecution) -> AgentTaskExecution {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let woken = self.task(&task.task_id);
                let delivered = self
                    .store()
                    .agent_inbox(&self.room, &self.agent, 0)
                    .unwrap()
                    .iter()
                    .all(|event| event.state != "pending" && event.state != "submitting");
                if woken.state == ExecutionState::Working
                    && woken.prompt_id != task.prompt_id
                    && delivered
                {
                    return woken;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the ended source starts the task's next turn")
    }

    /// The agent reads and handles every event its turn was woken for.
    fn handle_inbox(&self) {
        for event in self
            .store()
            .agent_inbox(&self.room, &self.agent, 0)
            .unwrap()
        {
            if event.state == "accepted" {
                self.store()
                    .agent_lifecycle(Operation::Ack {
                        room: self.room.clone(),
                        agent: self.agent.clone(),
                        sequence: event.sequence,
                        handled: true,
                        now: crate::session::unix_epoch_ms(),
                    })
                    .unwrap();
            }
        }
    }
}

/// The incident of 2026-11-14: the child's second task waited on a wake whose end the agent read
/// and handled while its first task waited on a live wake. The second task's wait is settled,
/// but the child still holds a live wake: its parent hears both ends once the child holds none.
#[tokio::test]
async fn a_task_whose_wait_the_agent_settled_is_not_finished_while_it_holds_another_live_wake() {
    let f = child_fixture("watch-open-sibling").await;
    let first = f.begin(&f.prompt_id);
    let first_wake = f.arm(&first, "wake-first").unwrap();
    f.wait_on(&first, &first_wake.registration_id);
    let second = f.begin("turn-second");
    let second_wake = f.arm(&second, "wake-second").unwrap();
    f.wait_on(&second, &second_wake.registration_id);
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.room, &f.agent, Some(&f.run))
        .unwrap();

    f.store()
        .agent_lifecycle(Operation::WakeOccurred {
            id: second_wake.id.clone(),
            occurrence: "second-exit".into(),
            kind: "exit".into(),
            payload: serde_json::json!({}),
            summary: None,
            end: Some(WakeEnd::Succeeded),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    // The agent reads the end in some turn and handles it; the first wake still runs.
    let ended = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(ended.len(), 1, "{ended:?}");
    f.store()
        .agent_lifecycle(Operation::MarkShown {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequences: vec![ended[0].sequence],
        })
        .unwrap();
    f.store()
        .agent_lifecycle(Operation::Ack {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequence: ended[0].sequence,
            handled: true,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(f.task(&second.task_id).state, ExecutionState::Waiting);
    assert!(
        f.child_finished().is_empty(),
        "the child still waits on wake-first: {:?}",
        f.child_finished()
    );

    let next = f.wake_ends(&first_wake.id, &first).await;
    f.handle_inbox();
    f.end_turn(&next.prompt_id).await;
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    let mut ended: Vec<String> = f
        .child_finished()
        .iter()
        .map(|event| event.payload["child_task_id"].as_str().unwrap().to_owned())
        .collect();
    ended.sort();
    let mut expected = vec![first.task_id.clone(), second.task_id.clone()];
    expected.sort();
    assert_eq!(ended, expected, "one end for each task, none earlier");
}
