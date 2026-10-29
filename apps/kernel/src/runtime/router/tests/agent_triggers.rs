use super::*;

// Keep large router/scheduler futures out of the async tests' poll frames,
// including their construction before they are moved onto the heap.
#[inline(never)]
fn boxed_schedule_future<F: std::future::Future>(
    make_future: impl FnOnce() -> F,
) -> std::pin::Pin<Box<F>> {
    Box::pin(make_future())
}

#[tokio::test]
async fn wait_schedule_triggers_start_queue_recur_and_delete() {
    let worktree = crate::test_support::TestWorktree::new("agent-prompt-schedules");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let once_request = wait_request(
        &session,
        &agent,
        crate::session::AgentTriggerRepeat::Once,
        None,
    );
    let once_command =
        KernelCommand::from_local_request("create-wait-in", None, None, &once_request);
    let once = boxed_schedule_future(|| router.dispatch(once_command, once_request))
        .await
        .expect("one-shot schedule should create");
    let once_schedule = match once {
        LocalDaemonResponse::AgentTriggerCreated {
            trigger: schedule,
            session,
        } => {
            assert_eq!(schedule.prompt(), "Continue from where you left off.");
            assert_eq!(session.agent_triggers().len(), 1);
            schedule
        }
        response => panic!("unexpected one-shot schedule response: {response:?}"),
    };

    let recurring_request = wait_request(
        &session,
        &agent,
        crate::session::AgentTriggerRepeat::Every,
        Some("Check whether more work remains."),
    );
    let recurring_command =
        KernelCommand::from_local_request("create-wait-every", None, None, &recurring_request);
    let recurring = boxed_schedule_future(|| router.dispatch(recurring_command, recurring_request))
        .await
        .expect("recurring schedule should create");
    let recurring_schedule = match recurring {
        LocalDaemonResponse::AgentTriggerCreated {
            trigger: schedule,
            session,
        } => {
            assert_eq!(session.agent_triggers().len(), 2);
            schedule
        }
        response => panic!("unexpected recurring schedule response: {response:?}"),
    };

    let due_at_ms = once_schedule
        .next_run_at_ms()
        .max(recurring_schedule.next_run_at_ms());
    boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(due_at_ms)).await;

    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("scheduled prompt snapshot should remain readable");
    assert_eq!(
        snapshot
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.prompt()),
        Some("Continue from where you left off.")
    );
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(agent.id())
            .and_then(|prompts| prompts.front())
            .map(|prompt| prompt.prompt()),
        Some("Check whether more work remains.")
    );
    assert!(
        snapshot
            .agent_triggers()
            .iter()
            .all(|trigger| trigger.id() != once_schedule.id()),
        "one-shot schedule should disappear after normal prompt admission"
    );
    let projected_recurring = snapshot
        .agent_triggers()
        .iter()
        .find(|trigger| trigger.id() == recurring_schedule.id())
        .expect("recurring schedule should remain");
    assert_eq!(projected_recurring.fired(), 1);
    assert!(projected_recurring.next_run_at_ms() > due_at_ms);

    for tick in 1..=43 {
        let now_ms = due_at_ms + tick * 60_000;
        boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(now_ms)).await;
        let snapshot = router
            .runtime_state
            .session_snapshot(session.id())
            .await
            .expect("busy schedule snapshot should remain readable");
        assert_eq!(
            snapshot.queued_prompts_for_agent(agent.id()).unwrap().len(),
            1,
            "a busy agent must retain only one occurrence of the recurrence"
        );
        let trigger = &snapshot.agent_triggers()[0];
        assert_eq!(trigger.fired(), 1);
        assert_eq!(trigger.coalesced(), tick);
        assert_eq!(trigger.last_fired_at_ms(), Some(due_at_ms));
        assert_eq!(trigger.next_run_at_ms(), now_ms + 60_000);
    }

    let cancel_request =
        LocalDaemonRequest::DeleteAgentTrigger(crate::local::DeleteAgentTriggerRequest {
            session_id: session.id().to_string(),
            trigger_id: recurring_schedule.id().to_string(),
        });
    let cancel_command =
        KernelCommand::from_local_request("cancel-wait-every", None, None, &cancel_request);
    let cancelled = boxed_schedule_future(|| router.dispatch(cancel_command, cancel_request))
        .await
        .expect("recurring schedule should cancel");
    match cancelled {
        LocalDaemonResponse::AgentTriggerDeleted {
            trigger: schedule,
            session,
        } => {
            assert_eq!(schedule.id(), recurring_schedule.id());
            assert!(session.agent_triggers().is_empty());
            assert_eq!(
                session.queued_prompts_for_agent(agent.id()).unwrap().len(),
                1
            );
            assert_eq!(
                session
                    .active_prompt_for_agent(agent.id())
                    .unwrap()
                    .prompt(),
                "Continue from where you left off."
            );
        }
        response => panic!("unexpected schedule cancellation response: {response:?}"),
    }
}

#[tokio::test]
async fn schedule_triggers_coalesce_active_and_restored_occurrences() {
    let worktree = crate::test_support::TestWorktree::new("schedule-coalescing");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let prompt_owner = app.prompt_state_owner();
    let session_store = app.session_state_store();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let mut schedules = Vec::new();
    for _ in 0..2 {
        let response = router
            .runtime_state
            .create_agent_trigger(
                every_minute(
                    &session,
                    &agent,
                    crate::session::AgentTriggerBusyPolicy::Queue,
                ),
                agent.owner_user_id(),
            )
            .await
            .expect("independent schedule should create");
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = response else {
            panic!("unexpected trigger response");
        };
        schedules.push(trigger);
    }
    let due_at_ms = schedules
        .iter()
        .map(|schedule| schedule.next_run_at_ms())
        .max()
        .unwrap();
    boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(due_at_ms)).await;
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        active.as_ref().unwrap().agent_trigger_id(),
        Some(schedules[0].id())
    );
    assert_eq!(
        queued.len(),
        1,
        "a different schedule with identical text must still queue"
    );
    assert_eq!(queued[0].agent_trigger_id(), Some(schedules[1].id()));

    let manual = crate::session::PromptQueueItem::new(
        "manual-same-text",
        active.as_ref().unwrap().source_attachment_id(),
        agent.id(),
        "The same prompt text",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt: manual } = prompt_owner
        .submit_prepared_prompt(&session, manual, false)
        .expect("same-text manual prompt should queue")
    else {
        panic!("manual prompt should remain queued");
    };
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    let snapshot = session_store
        .mirror_agent_prompt_state(session.id(), agent.id(), active, queued)
        .expect("prompt state should project");

    // Replay the same public session and private prompt records used by restart recovery.
    let mut durable: crate::durable_prompt_state::DurablePromptStateEventPayload =
        serde_json::from_value(
            serde_json::to_value(
                crate::durable_prompt_state::DurablePromptStateEventPayload::capture(
                    &snapshot,
                    agent.id(),
                ),
            )
            .unwrap(),
        )
        .unwrap();
    durable.restore_private_states();
    let mut restored: crate::session::RuntimeSession =
        serde_json::from_value(serde_json::to_value(&snapshot).unwrap()).unwrap();
    restored.mirror_agent_prompt_state(agent.id(), durable.active_prompt, durable.queued_prompts);
    prompt_owner.restore_session_state(&restored);
    session_store.restore_session(restored);

    for tick in 1..=43 {
        let now_ms = due_at_ms + tick * 60_000;
        boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(now_ms)).await;
        let (active, queued) = prompt_owner.state_parts(&session, agent.id());
        assert_eq!(
            active.as_ref().unwrap().agent_trigger_id(),
            Some(schedules[0].id())
        );
        assert_eq!(
            queued.len(),
            2,
            "active and queued occurrences must both coalesce after restart"
        );
        assert_eq!(queued[0].agent_trigger_id(), Some(schedules[1].id()));
        assert_eq!(
            queued[1].id(),
            manual.id(),
            "coalescing must preserve the manual prompt"
        );
        let snapshot = session_store.get_session(session.id()).unwrap();
        for trigger in snapshot.agent_triggers() {
            assert_eq!(trigger.fired(), 1);
            assert_eq!(trigger.next_run_at_ms(), now_ms + 60_000);
        }
    }

    prompt_owner
        .complete_active_prompt_only(&session, agent.id())
        .expect("first occurrence should finish");
    prompt_owner
        .activate_next_queued_prompt_for_dispatch(&session, agent.id(), None)
        .unwrap()
        .expect("second schedule should start");
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(due_at_ms + 44 * 60_000)
    })
    .await;
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        active.as_ref().unwrap().agent_trigger_id(),
        Some(schedules[1].id())
    );
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].id(), manual.id());
    assert_eq!(queued[1].agent_trigger_id(), Some(schedules[0].id()));
    let snapshot = session_store.get_session(session.id()).unwrap();
    assert_eq!(snapshot.agent_triggers()[0].fired(), 2);
    assert_eq!(snapshot.agent_triggers()[1].fired(), 1);

    for schedule in schedules {
        router
            .runtime_state
            .delete_agent_trigger(
                crate::local::DeleteAgentTriggerRequest {
                    session_id: session.id().to_string(),
                    trigger_id: schedule.id().to_string(),
                },
                agent.owner_user_id(),
            )
            .await
            .expect("trigger deletion should stop future ticks");
    }
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(due_at_ms + 100 * 60_000)
    })
    .await;
    assert_eq!(
        prompt_owner.state_parts(&session, agent.id()),
        (active, queued),
        "deletion must preserve already-admitted scheduled and manual prompts"
    );
}

/// What `/wait-in` and `/wait-every` send: a queue-policy schedule trigger.
fn wait_request(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
    repeat: crate::session::AgentTriggerRepeat,
    prompt: Option<&str>,
) -> LocalDaemonRequest {
    LocalDaemonRequest::CreateAgentTrigger(crate::local::CreateAgentTriggerRequest {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        source: crate::local::AgentTriggerSourceRequest::Schedule {
            interval_seconds: 60,
            repeat,
        },
        busy_policy: crate::session::AgentTriggerBusyPolicy::Queue,
        prompt: prompt.map(str::to_string),
    })
}

fn every_minute(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
    busy_policy: crate::session::AgentTriggerBusyPolicy,
) -> crate::local::CreateAgentTriggerRequest {
    crate::local::CreateAgentTriggerRequest {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        source: crate::local::AgentTriggerSourceRequest::Schedule {
            interval_seconds: 60,
            repeat: crate::session::AgentTriggerRepeat::Every,
        },
        busy_policy,
        prompt: Some("The same prompt text".to_string()),
    }
}

#[tokio::test]
async fn busy_policy_steers_or_queues_each_trigger_on_a_running_agent() {
    use crate::session::{AgentTriggerBusyPolicy, AgentTriggerOutcome};
    let worktree = crate::test_support::TestWorktree::new("trigger-busy-policy");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let prompt_owner = app.prompt_state_owner();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let mut triggers = Vec::new();
    // The first firing starts the turn; the next two find the agent running.
    for busy_policy in [
        AgentTriggerBusyPolicy::Queue,
        AgentTriggerBusyPolicy::Steer,
        AgentTriggerBusyPolicy::Queue,
    ] {
        let response = router
            .runtime_state
            .create_agent_trigger(
                every_minute(&session, &agent, busy_policy),
                agent.owner_user_id(),
            )
            .await
            .expect("trigger should create");
        let LocalDaemonResponse::AgentTriggerCreated { trigger, session } = response else {
            panic!("unexpected trigger response");
        };
        assert_eq!(
            session.agent_triggers().last().map(|t| t.busy_policy()),
            Some(busy_policy)
        );
        triggers.push(trigger);
    }
    let due_at_ms = triggers.iter().map(|t| t.next_run_at_ms()).max().unwrap();
    boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(due_at_ms)).await;

    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        active.as_ref().and_then(|prompt| prompt.agent_trigger_id()),
        Some(triggers[0].id())
    );
    assert_eq!(
        queued
            .iter()
            .map(|prompt| prompt.agent_trigger_id())
            .collect::<Vec<_>>(),
        vec![Some(triggers[2].id())],
        "the steer firing joins the running turn; the queue firing waits"
    );
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("snapshot should read");
    let outcomes = snapshot
        .agent_triggers()
        .iter()
        .map(|trigger| (trigger.last_outcome(), trigger.last_error()))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        vec![
            (Some(AgentTriggerOutcome::Started), None),
            (Some(AgentTriggerOutcome::Steered), None),
            (Some(AgentTriggerOutcome::Queued), None),
        ]
    );
}

#[tokio::test]
async fn destroying_an_agent_removes_its_triggers() {
    let worktree = crate::test_support::TestWorktree::new("trigger-agent-destroy");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, kept) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    let destroyed = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "codex"))
        .expect("agent should spawn");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    for agent_id in [kept.id(), destroyed.id()] {
        let request =
            LocalDaemonRequest::CreateAgentTrigger(crate::local::CreateAgentTriggerRequest {
                session_id: session.id().to_string(),
                agent_id: agent_id.to_string(),
                source: crate::local::AgentTriggerSourceRequest::Schedule {
                    interval_seconds: 600,
                    repeat: crate::session::AgentTriggerRepeat::Every,
                },
                busy_policy: crate::session::AgentTriggerBusyPolicy::Queue,
                prompt: None,
            });
        let command = KernelCommand::from_local_request("create-wait-every", None, None, &request);
        boxed_schedule_future(|| router.dispatch(command, request))
            .await
            .expect("schedule should create");
    }
    router
        .runtime_state
        .destroy_agent(destroyed.id(), DEFAULT_LOCAL_USER_ID)
        .await
        .expect("agent should be destroyed");

    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("snapshot should read");
    let snapshot = serde_json::to_value(&snapshot).unwrap();
    let schedule_agents = snapshot["agent_triggers"]
        .as_array()
        .expect("the kept agent's schedule should remain")
        .iter()
        .map(|schedule| schedule["agent_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(schedule_agents, vec![kept.id()]);
}

#[tokio::test]
async fn a_refused_steer_leaves_the_firing_queued_and_says_why() {
    use crate::session::AgentTriggerOutcome;
    let worktree = crate::test_support::TestWorktree::new("trigger-steer-refused");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let prompt_owner = app.prompt_state_owner();
    let session_store = app.session_state_store();
    // The person is running a turn in the provider's own terminal; Chariox
    // cannot steer into a turn it did not start.
    let native_turn = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "thread-1",
        "turn-1",
        agent.id(),
        "typed in the native terminal",
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = prompt_owner
        .submit_prepared_prompt(&session, native_turn, false)
        .expect("the native turn should become active")
    else {
        panic!("the native turn should start on an idle agent");
    };
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    session_store
        .mirror_agent_prompt_state(session.id(), agent.id(), active, queued)
        .expect("prompt state should project");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
        .runtime_state
        .create_agent_trigger(
            every_minute(
                &session,
                &agent,
                crate::session::AgentTriggerBusyPolicy::Steer,
            ),
            agent.owner_user_id(),
        )
        .await
        .expect("trigger should create")
    else {
        panic!("unexpected trigger response");
    };

    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
    })
    .await;

    let (_, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        queued
            .iter()
            .map(|prompt| prompt.agent_trigger_id())
            .collect::<Vec<_>>(),
        vec![Some(trigger.id())],
        "the firing is kept, not dropped"
    );
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("snapshot should read");
    let fired = &snapshot.agent_triggers()[0];
    assert_eq!(fired.last_outcome(), Some(AgentTriggerOutcome::Queued));
    assert!(fired
        .last_error()
        .is_some_and(|error| error.starts_with("steer refused, the firing stays queued")));
    let notices = router
        .app
        .lock()
        .await
        .terminal_stream_store()
        .notice_records();
    assert!(notices.iter().any(|notice| notice
        .message
        .contains("steer refused, the firing stays queued")));
}

async fn archive_test_agent(
    router: &CommandRouter,
    session_id: &str,
    agent_id: &str,
    archived: bool,
) {
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "ArchiveAgent": {"session_id": session_id, "agent_id": agent_id, "archived": archived}
    }))
    .expect("ArchiveAgent must be part of the shared protocol");
    let command = KernelCommand::from_local_request("archive-agent", None, None, &request);
    boxed_schedule_future(|| router.dispatch(command, request))
        .await
        .expect("idle agent archive state should change");
}

#[tokio::test]
async fn archiving_an_agent_removes_its_triggers_and_unarchive_starts_nothing() {
    let worktree = crate::test_support::TestWorktree::new("agent-triggers-archived");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let mut due_at_ms = 0;
    for repeat in [
        crate::session::AgentTriggerRepeat::Once,
        crate::session::AgentTriggerRepeat::Every,
    ] {
        let mut request = every_minute(
            &session,
            &agent,
            crate::session::AgentTriggerBusyPolicy::Queue,
        );
        request.source = crate::local::AgentTriggerSourceRequest::Schedule {
            interval_seconds: 60,
            repeat,
        };
        let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
            .runtime_state
            .create_agent_trigger(request, agent.owner_user_id())
            .await
            .expect("schedule trigger should create")
        else {
            panic!("unexpected trigger response");
        };
        due_at_ms = due_at_ms.max(trigger.next_run_at_ms());
    }

    archive_test_agent(&router, session.id(), agent.id(), true).await;
    boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(due_at_ms)).await;
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("archived agent snapshot should stay readable");
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    assert!(
        snapshot
            .agent_triggers()
            .iter()
            .all(|trigger| trigger.agent_id() != agent.id()),
        "archive removes the agent's triggers"
    );

    archive_test_agent(&router, session.id(), agent.id(), false).await;
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(due_at_ms + 1)
    })
    .await;
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap();
    assert!(
        snapshot.active_prompt_for_agent(agent.id()).is_none(),
        "unarchive must not start a turn"
    );
}

#[tokio::test]
async fn a_firing_seen_while_its_agent_is_archived_releases_its_claim() {
    let worktree = crate::test_support::TestWorktree::new("agent-triggers-archive-window");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
        .runtime_state
        .create_agent_trigger(
            every_minute(
                &session,
                &agent,
                crate::session::AgentTriggerBusyPolicy::Queue,
            ),
            agent.owner_user_id(),
        )
        .await
        .expect("schedule trigger should create")
    else {
        panic!("unexpected trigger response");
    };
    let due_at_ms = trigger.next_run_at_ms();
    // An archive whose durable write fails is rolled back: a tick in between
    // sees the agent archived while its triggers still exist.
    let set_archived = |archived: bool| {
        router
            .runtime_state
            .set_agent_archived_without_cleanup_for_test(agent.id(), archived)
    };
    set_archived(true);
    boxed_schedule_future(|| router.runtime_state.dispatch_due_agent_triggers(due_at_ms)).await;
    set_archived(false);
    let deferred = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap()
        .agent_triggers()
        .iter()
        .find(|candidate| candidate.id() == trigger.id())
        .cloned()
        .expect("the trigger outlives the rolled-back archive");
    assert!(!deferred.dispatch_in_flight(), "the claim is released");
    assert_eq!(deferred.fired(), 0, "no firing reached the archived agent");

    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(deferred.next_run_at_ms())
    })
    .await;
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap();
    assert_eq!(
        snapshot
            .active_prompt_for_agent(agent.id())
            .and_then(|prompt| prompt.agent_trigger_id()),
        Some(trigger.id()),
        "the trigger fires again once the agent is back"
    );
}

#[tokio::test]
async fn destroying_an_agent_settles_its_requested_reply() {
    use crate::durable_state::agent_lifecycle::{occurrence, Operation};
    let worktree = crate::test_support::TestWorktree::new("reply-agent-destroy");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let child = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "codex"))
        .unwrap();
    let store = app.durable_state_store();
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: session.id().into(),
            agent: parent.id().into(),
            prompt: "parent-task".into(),
            run: Some("parent-run".into()),
            now: 1,
        })
        .unwrap();
    let mut request = occurrence(
        session.id(),
        child.id(),
        parent.id(),
        "request",
        "message",
        serde_json::json!({"message": "answer"}),
    );
    request.reply_requested = true;
    store
        .agent_lifecycle(Operation::Send {
            task: "parent-task".into(),
            prompt: "parent-task".into(),
            event: request,
        })
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let request = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: session.id().into(),
        agent_id: child.id().into(),
    });
    let command =
        KernelCommand::from_local_request("destroy-reply-recipient", None, None, &request);
    boxed_schedule_future(|| router.dispatch(command, request))
        .await
        .unwrap();
    let tasks = store
        .agent_tasks(Some(session.id()), Some(parent.id()))
        .unwrap();
    let reply = tasks[0]
        .obligations
        .iter()
        .find(|o| o.kind == "reply")
        .unwrap();
    assert_eq!(reply.status, "failed");
    let inbox = store.agent_inbox(session.id(), parent.id(), 0).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "source_lost");
}

/// Replaces the agent's test run with a structured one whose provider answers
/// a submit on its own schedule: `slow-structured` acknowledges after a delay,
/// an unbound `codex` run refuses the steer.
fn install_structured_run(
    app: &mut DaemonApp,
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
    run_id: &str,
    adapter_key: &str,
    provider: &str,
) {
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        adapter_key,
        provider,
        "default",
        "model",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "trigger-structured-steer".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run);
}

/// Starts a turn the way a person's prompt would, so a trigger firing finds a busy agent.
fn start_running_turn(
    app: &DaemonApp,
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
) -> crate::runtime::prompt_state::PromptStateOwner {
    let prompt_owner = app.prompt_state_owner();
    let turn = crate::session::PromptQueueItem::new(
        app.session_state_store().reserve_prompt_id(),
        "person-attachment",
        agent.id(),
        "a turn in progress",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = prompt_owner
        .submit_prepared_prompt(session, turn, false)
        .expect("the turn should become active")
    else {
        panic!("the turn should start on an idle agent");
    };
    let (active, queued) = prompt_owner.state_parts(session, agent.id());
    app.session_state_store()
        .mirror_agent_prompt_state(session.id(), agent.id(), active, queued)
        .expect("prompt state should project");
    prompt_owner
}

#[tokio::test]
async fn a_steer_the_provider_refuses_after_the_mailbox_accepted_it_leaves_the_firing_queued() {
    use crate::session::AgentTriggerOutcome;
    let worktree = crate::test_support::TestWorktree::new("trigger-steer-provider-refusal");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    install_structured_run(&mut app, &session, &agent, run.id(), "codex", "codex");
    let prompt_owner = start_running_turn(&app, &session, &agent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
        .runtime_state
        .create_agent_trigger(
            every_minute(
                &session,
                &agent,
                crate::session::AgentTriggerBusyPolicy::Steer,
            ),
            agent.owner_user_id(),
        )
        .await
        .expect("trigger should create")
    else {
        panic!("unexpected trigger response");
    };

    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
    })
    .await;

    let (_, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        queued
            .iter()
            .map(|prompt| prompt.agent_trigger_id())
            .collect::<Vec<_>>(),
        vec![Some(trigger.id())],
        "the provider refused the steer, so the firing must still be queued"
    );
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("snapshot should read");
    let fired = &snapshot.agent_triggers()[0];
    assert_eq!(fired.last_outcome(), Some(AgentTriggerOutcome::Queued));
    assert!(fired
        .last_error()
        .is_some_and(|error| error.starts_with("steer refused, the firing stays queued")));
}

#[tokio::test]
async fn a_steer_whose_acknowledgement_is_lost_is_not_replayed_when_the_turn_ends() {
    use crate::session::AgentTriggerOutcome;
    let worktree = crate::test_support::TestWorktree::new("trigger-steer-lost-acknowledgement");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    // The provider receives the steer, then its answer is lost.
    install_structured_run(
        &mut app,
        &session,
        &agent,
        run.id(),
        "dev-stub",
        "lost-ack-structured",
    );
    let prompt_owner = start_running_turn(&app, &session, &agent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
        .runtime_state
        .create_agent_trigger(
            every_minute(
                &session,
                &agent,
                crate::session::AgentTriggerBusyPolicy::Steer,
            ),
            agent.owner_user_id(),
        )
        .await
        .expect("trigger should create")
    else {
        panic!("unexpected trigger response");
    };

    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_triggers(trigger.next_run_at_ms())
    })
    .await;

    let (_, queued) = prompt_owner.state_parts(&session, agent.id());
    assert!(
        queued.is_empty(),
        "the provider may hold the steer, so the firing must not stay queued for a second run"
    );
    prompt_owner
        .complete_active_prompt_only(&session, agent.id())
        .expect("the turn should finish");
    assert!(
        prompt_owner
            .activate_next_queued_prompt_for_dispatch(&session, agent.id(), None)
            .unwrap()
            .is_none(),
        "the unconfirmed firing must not run again as a fresh prompt"
    );
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("snapshot should read");
    assert_eq!(
        snapshot.agent_triggers()[0].last_outcome(),
        Some(AgentTriggerOutcome::Steered)
    );
}

#[tokio::test]
async fn a_steer_pending_across_a_restart_is_not_replayed_when_the_recovered_turn_ends() {
    let worktree = crate::test_support::TestWorktree::new("trigger-steer-restart-fence");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    install_structured_run(
        &mut app,
        &session,
        &agent,
        run.id(),
        "dev-stub",
        "lost-ack-structured",
    );
    let prompt_owner = start_running_turn(&app, &session, &agent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
        .runtime_state
        .create_agent_trigger(
            every_minute(
                &session,
                &agent,
                crate::session::AgentTriggerBusyPolicy::Steer,
            ),
            agent.owner_user_id(),
        )
        .await
        .expect("trigger should create")
    else {
        panic!("unexpected trigger response");
    };
    let dispatching = {
        let runtime_state = router.runtime_state.clone();
        let due = trigger.next_run_at_ms();
        tokio::spawn(async move {
            boxed_schedule_future(|| runtime_state.dispatch_due_agent_triggers(due)).await;
        })
    };

    // The provider holds the steer and its acknowledgement is still pending.
    let in_flight = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (_, queued) = prompt_owner.state_parts(&session, agent.id());
            if let Some(prompt) = queued
                .front()
                .filter(|prompt| prompt.remote_steer_reserved())
            {
                break prompt.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the steer should be in flight");
    let (active, _) = prompt_owner.state_parts(&session, agent.id());

    // A restart keeps only what was persisted: rebuild the queue from the
    // durable private state, with no reservation of this process.
    let private: crate::session::DurablePromptPrivateState = serde_json::from_value(
        serde_json::to_value(
            crate::session::DurablePromptPrivateState::from_prompt(session.id(), &in_flight)
                .expect("the fence must be persisted before the provider write"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut restored_queued: crate::session::PromptQueueItem =
        serde_json::from_value(serde_json::to_value(&in_flight).unwrap()).unwrap();
    restored_queued.restore_durable_private_state(&private);
    let restored_active: crate::session::PromptQueueItem =
        serde_json::from_value(serde_json::to_value(active.as_ref().unwrap()).unwrap()).unwrap();
    let mut restored_session = session.clone();
    restored_session.mirror_agent_prompt_state(
        agent.id(),
        Some(restored_active),
        std::collections::VecDeque::from([restored_queued]),
    );
    let restarted = crate::runtime::prompt_state::PromptStateOwner::default();
    restarted.restore_session_state(&restored_session);

    assert!(
        restarted
            .peek_next_queued_prompt(&restored_session, agent.id())
            .is_none(),
        "the restored firing must stay held while the recovered turn runs"
    );
    restarted
        .complete_active_prompt_only(&restored_session, agent.id())
        .expect("the recovered turn should finish");
    assert!(
        restarted
            .activate_next_queued_prompt_for_dispatch(&restored_session, agent.id(), None)
            .unwrap()
            .is_none(),
        "the firing the provider may hold must not run again as a fresh prompt"
    );
    let (_, queued) = restarted.state_parts(&restored_session, agent.id());
    assert!(
        queued
            .front()
            .is_some_and(|prompt| prompt.remote_steer_reserved()),
        "the firing stays held for reconciliation, never promoted"
    );
    dispatching.await.expect("dispatch task should finish");
}

#[tokio::test]
async fn a_steered_firing_stays_queued_until_the_provider_acknowledges_it() {
    use crate::session::AgentTriggerOutcome;
    let worktree = crate::test_support::TestWorktree::new("trigger-steer-waits-for-provider");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    // The stub provider answers a submit after 750 ms.
    install_structured_run(
        &mut app,
        &session,
        &agent,
        run.id(),
        "dev-stub",
        "slow-structured",
    );
    let prompt_owner = start_running_turn(&app, &session, &agent);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = router
        .runtime_state
        .create_agent_trigger(
            every_minute(
                &session,
                &agent,
                crate::session::AgentTriggerBusyPolicy::Steer,
            ),
            agent.owner_user_id(),
        )
        .await
        .expect("trigger should create")
    else {
        panic!("unexpected trigger response");
    };

    let firing_settled = std::sync::atomic::AtomicBool::new(false);
    let ((), ()) = tokio::join!(
        async {
            boxed_schedule_future(|| {
                router
                    .runtime_state
                    .dispatch_due_agent_triggers(trigger.next_run_at_ms())
            })
            .await;
            firing_settled.store(true, std::sync::atomic::Ordering::SeqCst);
        },
        async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert!(
                !firing_settled.load(std::sync::atomic::Ordering::SeqCst),
                "the firing settled before the provider answered"
            );
            let (_, queued) = prompt_owner.state_parts(&session, agent.id());
            assert_eq!(
                queued.len(),
                1,
                "the firing must stay queued until the provider acknowledges the steer"
            );
        }
    );

    let (_, queued) = prompt_owner.state_parts(&session, agent.id());
    assert!(queued.is_empty(), "the acknowledged steer leaves the queue");
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("snapshot should read");
    assert_eq!(
        snapshot.agent_triggers()[0].last_outcome(),
        Some(AgentTriggerOutcome::Steered)
    );
}
