//! MP-08 / MP-09 / MP-10 / MP-11: real turn admission and settlement producers.
use super::*;

#[test]
fn outside_runtime_turns_publish_working_and_idle_without_legacy_state_setters() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let worktree =
                        crate::test_support::TestWorktree::new("outside-runtime-activity");
                    let app =
                        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                            .unwrap();
                    let session = app
                        .sessions_mut()
                        .create_session(worktree.session_request())
                        .unwrap();
                    let agent = crate::agent::AgentInstance::new(
                        "outside-live-agent",
                        "outside-live-agent",
                        session.id(),
                        None,
                        "dev-stub",
                        None,
                        None,
                        Some(session.worktree_id().into()),
                        crate::agent::GridPosition::new(0, 0, 1, 1),
                    );
                    app.agents_mut().restore_agent(agent.clone());
                    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
                        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                        4,
                    );
                    let runtime = router.runtime_state();
                    let target = EventTarget {
                        kernel: runtime.owned.config_projection.snapshot().daemon_id,
                        session_id: session.id().into(),
                        agent_id: Some(agent.id().into()),
                    };
                    let subscribe = LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
                        targets: vec![target],
                    });
                    runtime
                        .outside_part_request(
                            "first",
                            true,
                            Some(crate::session::DEFAULT_LOCAL_USER_ID),
                            &subscribe,
                        )
                        .unwrap();
                    runtime.start_active_turn_with_trace_id(
                        session.id(),
                        agent.id(),
                        "prompt",
                        "provider-run",
                        "trace",
                    );
                    // A second admission notification is not a second state transition.
                    runtime.start_active_turn_with_trace_id(
                        session.id(),
                        agent.id(),
                        "prompt",
                        "provider-run",
                        "trace",
                    );
                    // Following an already-working agent must still notice its later finish.
                    runtime
                        .outside_part_request(
                            "late",
                            true,
                            Some(crate::session::DEFAULT_LOCAL_USER_ID),
                            &subscribe,
                        )
                        .unwrap();
                    runtime.owned.clear_prompt_activity("provider-run");
                    runtime.owned.clear_prompt_activity("provider-run");
                    let states = |id: &str| {
                        runtime
                            .outside_store()
                            .outside_list(id, None, 100)
                            .unwrap()
                            .unwrap()
                            .events
                            .into_iter()
                            .filter_map(|event| match event.value {
                                OutsideEventValue::Agent { run_state, .. } => run_state,
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        states("first"),
                        [EventRunState::Working, EventRunState::Idle]
                    );
                    assert_eq!(states("late"), [EventRunState::Idle]);
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
