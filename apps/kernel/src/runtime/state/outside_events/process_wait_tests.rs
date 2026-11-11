//! A command that waits for room reaches the agent's followers as an outside event.
use super::*;
use crate::runtime::process_resources::{
    push_wait_event, take_wait_events, PreparedDomain, SessionProcessOwner, SessionProcessState,
    WaitChange, WaitEvent, Waiting,
};

#[cfg(target_os = "linux")]
#[test]
fn a_wait_and_its_end_reach_a_follower_of_the_agent() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let worktree = crate::test_support::TestWorktree::new("outside-process-wait");
                    let app =
                        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                            .unwrap();
                    let session = app
                        .sessions_mut()
                        .create_session(worktree.session_request())
                        .unwrap();
                    let agent = crate::agent::AgentInstance::new(
                        "builder",
                        "builder",
                        session.id(),
                        None,
                        "dev-stub",
                        None,
                        None,
                        Some(session.worktree_id().into()),
                        crate::agent::GridPosition::new(0, 0, 1, 1),
                    );
                    app.agents_mut().restore_agent(agent);
                    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
                        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                        4,
                    );
                    let runtime = router.runtime_state();
                    runtime
                        .outside_part_request(
                            "follower",
                            true,
                            Some(crate::session::DEFAULT_LOCAL_USER_ID),
                            &LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
                                targets: vec![EventTarget {
                                    kernel: runtime.owned.config_projection.snapshot().daemon_id,
                                    session_id: session.id().into(),
                                    agent_id: Some("builder".into()),
                                }], sources: Vec::new(), }),
                        )
                        .unwrap();
                    // A workload of the agent, listed under it in the session.
                    let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
                    let domain = PreparedDomain::for_tests().start(child.id()).unwrap();
                    domain.bind_key("process-wait-test-key");
                    domain.list_as(session.id(), SessionProcessOwner::Agent("builder".into()));
                    take_wait_events();
                    let row = Waiting {
                        reason: crate::runtime::process_resources::WaitReason::Memory,
                        position: 1,
                        need_bytes: 6 << 30,
                        free_bytes: 1 << 30,
                        command_line: "cargo build".into(),
                        cwd: Some("/work".into()),
                        since_ms: 1,
                    };
                    for (change, state) in [
                        (WaitChange::Began, SessionProcessState::Paused(row)),
                        (
                            WaitChange::Resumed { waited_ms: 4000 },
                            SessionProcessState::Running,
                        ),
                    ] {
                        push_wait_event(WaitEvent {
                            keys: vec!["process-wait-test-key".into()],
                            process_id: "process-issued-7".into(),
                            change,
                            state,
                        });
                    }
                    runtime.deliver_process_waits();
                    let events = runtime
                        .outside_store()
                        .outside_list("follower", None, 100)
                        .unwrap()
                        .unwrap()
                        .events
                        .into_iter()
                        .filter_map(|event| match event.value {
                            OutsideEventValue::Process {
                                process_id,
                                state,
                                waited_ms,
                                ..
                            } => Some((event.agent_id, process_id, state, waited_ms)),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(events.len(), 2, "{events:?}");
                    assert_eq!(events[0].0.as_deref(), Some("builder"));
                    assert_eq!(events[0].1, "process-issued-7");
                    assert!(matches!(&events[0].2, SessionProcessState::Paused(row) if row.command_line == "cargo build"));
                    assert_eq!((&events[1].2, events[1].3), (&SessionProcessState::Running, Some(4000)));
                    drop(domain);
                    let _ = child.kill();
                    let _ = child.wait();
                });
        })
        .unwrap()
        .join()
        .unwrap();
}

/// MP-08 / MP-09 / MP-10 / MP-11: the agent that owns a paused command finds
/// one short message for the pause and one for the resume in its inbox, and
/// neither starts or steers a turn.
#[cfg(target_os = "linux")]
#[test]
fn an_agent_finds_a_notice_in_its_inbox_when_its_command_pauses_and_when_it_continues() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let worktree = crate::test_support::TestWorktree::new("inbox-process-wait");
                    let app =
                        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                            .unwrap();
                    let session = app
                        .sessions_mut()
                        .create_session(worktree.session_request())
                        .unwrap();
                    let agent = crate::agent::AgentInstance::new(
                        "builder",
                        "builder",
                        session.id(),
                        None,
                        "dev-stub",
                        None,
                        None,
                        Some(session.worktree_id().into()),
                        crate::agent::GridPosition::new(0, 0, 1, 1),
                    );
                    app.agents_mut().restore_agent(agent);
                    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
                        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                        4,
                    );
                    let runtime = router.runtime_state();
                    let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
                    let domain = PreparedDomain::for_tests().start(child.id()).unwrap();
                    domain.bind_key("inbox-wait-test-key");
                    domain.list_as(session.id(), SessionProcessOwner::Agent("builder".into()));
                    take_wait_events();
                    let row = Waiting {
                        reason: crate::runtime::process_resources::WaitReason::Memory,
                        position: 1,
                        need_bytes: 6 << 30,
                        free_bytes: 1 << 30,
                        command_line: "cargo test".into(),
                        cwd: Some("/work".into()),
                        since_ms: 1,
                    };
                    let push = |change, state| {
                        push_wait_event(WaitEvent {
                            keys: vec!["inbox-wait-test-key".into()],
                            process_id: "process-issued-8".into(),
                            change,
                            state,
                        })
                    };
                    let notices = || -> Vec<String> {
                        runtime
                            .read_agent_inbox(session.id(), "builder", &serde_json::json!({}))
                            .unwrap()["events"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|event| event["kind"] == "resource_notice")
                            .map(|event| event["payload"]["message"].as_str().unwrap().to_string())
                            .collect()
                    };
                    let t0 = std::time::Instant::now();
                    let at = |seconds| t0 + std::time::Duration::from_secs(seconds);

                    push(WaitChange::Began, SessionProcessState::Paused(row));
                    runtime.deliver_process_waits_at(t0);
                    assert!(notices().is_empty(), "told before the pause settled");
                    runtime.deliver_process_waits_at(at(4));
                    assert_eq!(
                        notices(),
                        vec!["`cargo test` is paused: the machine is short of memory. It continues when memory frees."]
                    );

                    push(
                        WaitChange::Resumed { waited_ms: 5000 },
                        SessionProcessState::Running,
                    );
                    runtime.deliver_process_waits_at(at(5));
                    runtime.deliver_process_waits_at(at(9));
                    assert_eq!(notices().len(), 2, "{:?}", notices());
                    assert_eq!(notices()[1], "`cargo test` continues.");
                    assert!(
                        runtime
                            .owned
                            .durable_state_store
                            .agent_delivery_front(session.id(), "builder")
                            .unwrap()
                            .is_none(),
                        "a notice was queued as a delivery that would start or steer a turn"
                    );
                    drop(domain);
                    let _ = child.kill();
                    let _ = child.wait();
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
