use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::session::RuntimeSession;
use crate::terminal::TerminalOutputKind;

// MP-08 / MP-10 / MP-11: local and relay subscriptions drain this same boundary.
#[tokio::test]
async fn terminal_workflow_updates_obey_current_attachment_trace_visibility() {
    use crate::session::{
        CollaborationLevel, WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus,
    };
    for status in [
        WorkflowRunStatus::Completed,
        WorkflowRunStatus::Failed,
        WorkflowRunStatus::Stopped,
    ] {
        for level in [
            CollaborationLevel::Private,
            CollaborationLevel::Transparent,
            CollaborationLevel::Full,
        ] {
            let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
            let (session, owner) = crate::app::KernelSessionService::new(&mut app)
                .create_session(crate::session::CreateSessionRequest::new(
                    "workspace",
                    "worktree",
                ))
                .unwrap();
            let foreign = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_owner_user_id("foreign-owner"),
                )
                .unwrap();
            let retired_owned = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_owner_user_id(owner.owner_user_id()),
                )
                .unwrap();
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::for_user(
                    session.id(),
                    "workflow-subscriber",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                    owner.owner_user_id(),
                ))
                .unwrap();
            let run = |id: &str, creator: &str, worker: &str| {
                let mut run = WorkflowRun::new(
                    id,
                    "workflow",
                    "endpoint",
                    "node",
                    Some(format!("{id}-private-input")),
                    None,
                    vec![WorkflowNodeRun::new(
                        format!("{id}-node-run"),
                        "node",
                        worker,
                        0,
                        WorkflowNodeRunStatus::Completed,
                    )],
                    vec![],
                )
                .with_creator(Some(creator.into()));
                run.set_status(status);
                run
            };
            let foreign_run = run("foreign-run", foreign.id(), foreign.id());
            let own_run = run("owned-run", owner.id(), retired_owned.id());
            let mixed_run = run("mixed-run", owner.id(), foreign.id());
            // Queue before the collaboration change: delivery must use current policy.
            for run in [foreign_run, own_run, mixed_run] {
                app.terminal().record_workflow_run_update(
                    session.id(),
                    vec![attachment.id().into()],
                    run,
                );
            }
            for id in [foreign.id(), retired_owned.id()] {
                crate::app::KernelSessionService::new(&mut app)
                    .destroy_agent(id)
                    .unwrap();
            }
            let mut wire =
                serde_json::to_value(app.sessions().get_session(session.id()).unwrap()).unwrap();
            wire["members"][0]["collaboration_level"] = serde_json::to_value(level).unwrap();
            app.sessions_mut()
                .restore_session(serde_json::from_value(wire).unwrap());
            let WatchResult::Ok {
                workflow_run_updates,
                ..
            } = watch_subscription_state(&mut app, session.id(), attachment.id(), false, None, 0)
            else {
                panic!("subscription must remain available");
            };
            let ids = workflow_run_updates
                .iter()
                .map(|run| run.id())
                .collect::<Vec<_>>();
            assert_eq!(
                ids,
                if level.can_view_agent_trace() {
                    vec!["foreign-run", "owned-run", "mixed-run"]
                } else {
                    vec!["owned-run"]
                },
                "MP-11: terminal {status:?} updates must use {level:?} trace visibility",
            );
            let WatchResult::Ok {
                workflow_run_updates,
                ..
            } = watch_subscription_state(&mut app, session.id(), attachment.id(), false, None, 0)
            else {
                panic!("subscription must remain available");
            };
            assert!(
                workflow_run_updates.is_empty(),
                "updates drain exactly once"
            );
        }
    }
}

// MP-08 / MP-10: exercise the actual subscription loop, without another producer wakeup.
#[tokio::test]
async fn subscription_delivers_three_mib_burst_in_order_without_another_append() {
    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ))
        .expect("create session");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "burst-subscriber",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attach");
    let attachment_id = attachment.id().to_string();
    let expected: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    for bytes in expected.chunks(16 * 1024) {
        app.fan_out_output(
            &session_id,
            "burst-run",
            TerminalOutputKind::ProviderOutput,
            None,
            vec![attachment_id.clone()],
            bytes,
        );
    }
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    ));
    let runtime = Arc::new(KernelTransportRuntime::default());
    let (priority_tx, _priority_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let (close_tx, mut close_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_subscription_loop(
        router,
        runtime,
        KernelOutgoingSender::new(priority_tx, event_tx),
        close_tx,
        Arc::new(AtomicBool::new(false)),
        KernelSubscription {
            session_id,
            attachment_id,
            subscription_scope: KernelSubscriptionScope::Session,
            connection_class: crate::local::KernelConnectionClass::Terminal,
        },
    ));
    let receipt = tokio::time::timeout(Duration::from_secs(3), async {
        let mut delivered = Vec::new();
        let mut heartbeats = 0;
        let mut snapshot_seen = false;
        while delivered.len() < expected.len() {
            match event_rx.recv().await.expect("subscription remains open") {
                KernelOutgoingFrame::Event { event, .. } => match *event {
                    KernelEvent::TerminalOutput { records } => {
                        assert!(snapshot_seen, "initial session projection must precede bulk output from the same wake");
                        for record in records {
                            delivered.extend(record.bytes);
                        }
                    }
                    KernelEvent::SessionSnapshot { .. } => snapshot_seen = true,
                    KernelEvent::Heartbeat { .. } => heartbeats += 1,
                    _ => {}
                },
                _ => panic!("event queue contains only events"),
            }
        }
        (delivered, heartbeats)
    })
    .await;
    task.abort();
    let _ = task.await;
    let (delivered, heartbeats) =
        receipt.expect("subscriber drains the complete burst before heartbeat timeout");
    assert_eq!(delivered, expected, "every byte arrives once and in order");
    assert!(
        heartbeats > 0,
        "output delivery preserves heartbeat scheduling"
    );
    assert!(
        close_rx.try_recv().is_err(),
        "bounded output must not close a healthy subscriber"
    );
}

#[test]
fn subscription_wait_duration_uses_next_explicit_deadline() {
    let now = Instant::now();
    let wait = next_subscription_wait_duration(
        now + Duration::from_secs(1),
        now + Duration::from_secs(30),
    );

    assert!(
        wait <= Duration::from_secs(1),
        "wait should be bounded by the next heartbeat deadline"
    );
    assert!(
        wait > Duration::from_millis(900),
        "wait should not fall back to a short active-work poll"
    );
}

#[test]
fn subscription_wait_duration_is_ready_for_due_deadlines() {
    let now = Instant::now();
    let wait = next_subscription_wait_duration(
        now - Duration::from_millis(1),
        now + Duration::from_secs(30),
    );

    assert_eq!(wait, Duration::ZERO);
}

#[test]
fn subscription_deadline_intervals_preserve_previous_cadence() {
    assert_eq!(subscription_heartbeat_interval(), Duration::from_secs(5));
    assert_eq!(
        subscription_snapshot_reconciliation_interval(),
        Duration::from_secs(30)
    );
}

#[test]
fn advance_subscription_deadline_skips_missed_intervals() {
    let advanced = advance_subscription_deadline(
        Instant::now() - Duration::from_secs(11),
        Duration::from_secs(5),
    );

    assert!(advanced > Instant::now());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_subscription_work_keeps_emitting_heartbeats_without_restarting_work() {
    let work_starts = Arc::new(AtomicUsize::new(0));
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let work_starts_for_future = Arc::clone(&work_starts);
    let heartbeats_for_callback = Arc::clone(&heartbeats);
    let mut next_heartbeat_at = Instant::now();

    let result = await_subscription_work_with_heartbeats(
        async move {
            work_starts_for_future.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(65)).await;
            "snapshot"
        },
        &mut next_heartbeat_at,
        Duration::from_millis(20),
        move || {
            let heartbeats = Arc::clone(&heartbeats_for_callback);
            async move {
                heartbeats.fetch_add(1, Ordering::SeqCst);
                true
            }
        },
    )
    .await;

    assert_eq!(result, Some("snapshot"));
    assert_eq!(work_starts.load(Ordering::SeqCst), 1);
    assert!(
        heartbeats.load(Ordering::SeqCst) > 0,
        "a pending state read must not starve the subscription heartbeat"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn failed_heartbeat_stops_waiting_for_subscription_work() {
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let heartbeats_for_callback = Arc::clone(&heartbeats);
    let mut next_heartbeat_at = Instant::now();

    let result = await_subscription_work_with_heartbeats(
        std::future::pending::<()>(),
        &mut next_heartbeat_at,
        Duration::from_millis(20),
        move || {
            let heartbeats = Arc::clone(&heartbeats_for_callback);
            async move { heartbeats.fetch_add(1, Ordering::SeqCst) < 1 }
        },
    )
    .await;

    assert!(result.is_none());
    assert_eq!(heartbeats.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn due_heartbeat_precedes_ready_subscription_work() {
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let heartbeats_for_callback = Arc::clone(&heartbeats);
    let mut next_heartbeat_at = Instant::now();

    let result = await_subscription_work_with_heartbeats(
        std::future::ready("snapshot"),
        &mut next_heartbeat_at,
        Duration::from_millis(20),
        move || {
            let heartbeats = Arc::clone(&heartbeats_for_callback);
            async move {
                heartbeats.fetch_add(1, Ordering::SeqCst);
                true
            }
        },
    )
    .await;

    assert_eq!(result, Some("snapshot"));
    assert_eq!(heartbeats.load(Ordering::SeqCst), 1);
}

#[test]
fn compact_replay_events_preserves_heartbeat_output_and_latest_snapshot_only() {
    let events = vec![
        logged_event(
            1,
            KernelEvent::Heartbeat {
                session_id: "session-a".to_string(),
            },
        ),
        logged_event(2, session_snapshot_event("session-a", "snapshot-a")),
        logged_event(3, terminal_output_event("session-a", "first")),
        logged_event(4, session_snapshot_event("session-a", "snapshot-b")),
        logged_event(
            5,
            KernelEvent::TransportResumed {
                session_id: "session-a".to_string(),
                resumed_from_event_id: Some(1),
            },
        ),
        logged_event(6, terminal_output_event("session-a", "second")),
    ];

    let compacted = compact_replay_events(events);

    assert_eq!(
        compacted
            .iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>(),
        vec![1, 3, 4, 6]
    );
    assert!(matches!(compacted[0].event, KernelEvent::Heartbeat { .. }));
    assert!(matches!(
        compacted[1].event,
        KernelEvent::TerminalOutput { .. }
    ));
    assert!(matches!(
        compacted[2].event,
        KernelEvent::SessionSnapshot { .. }
    ));
    assert!(matches!(
        compacted[3].event,
        KernelEvent::TerminalOutput { .. }
    ));
}

fn logged_event(event_id: u64, event: KernelEvent) -> LoggedEvent<KernelEvent> {
    LoggedEvent {
        event_id,
        stream_id: "session:session-a:attachment:attachment-a".to_string(),
        stream_seq: event_id,
        recorded_at_ms: event_id * 1_000,
        event,
    }
}

fn session_snapshot_event(session_id: &str, alias: &str) -> KernelEvent {
    KernelEvent::SessionSnapshot {
        room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
            &RuntimeSession::new(
                session_id,
                Some(alias.into()),
                "workspace-a",
                "worktree-a",
                "machine-a",
                "daemon-a",
            ),
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
        session: Box::new(RuntimeSession::new(
            session_id,
            Some(alias.to_string()),
            "workspace-a",
            "worktree-a",
            "machine-a",
            "daemon-a",
        )),
        provider_run: Box::new(None),
        agent_activity: Box::new(BTreeMap::new()),
        agent_activity_revision: 0,
    }
}

fn terminal_output_event(session_id: &str, marker: &str) -> KernelEvent {
    KernelEvent::TerminalOutput {
        records: vec![TerminalOutputRecord {
            record_id: None,
            sequence: None,
            sequence_end: None,
            sequence_byte_lengths: Vec::new(),
            timestamp_ms: 1_000,
            session_id: session_id.to_string(),
            provider_run_id: "provider-run-a".to_string(),
            agent_id: Some("agent-a".to_string()),
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some(marker.to_string()),
            recipient_attachment_ids: vec!["attachment-a".to_string()],
            pending_recipient_attachment_ids: vec!["attachment-a".to_string()],
            bytes: marker.as_bytes().to_vec(),
            external_observation_metadata: None,
        }],
    }
}

#[tokio::test]
async fn room_workflows_local_drill_initial_inventory_other_client_creation_and_last_deletion() {
    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "room-workflows",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app.clone(),
        1,
    ));
    let runtime = Arc::new(KernelTransportRuntime::default());
    let (priority_tx, _priority_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let (close_tx, _close_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_subscription_loop(
        router,
        runtime,
        KernelOutgoingSender::new(priority_tx, event_tx),
        close_tx,
        Arc::new(AtomicBool::new(false)),
        KernelSubscription {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            subscription_scope: KernelSubscriptionScope::Session,
            connection_class: crate::local::KernelConnectionClass::Terminal,
        },
    ));
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let KernelOutgoingFrame::Event { event, .. } = event_rx.recv().await.unwrap() {
                if let KernelEvent::SessionSnapshot { room_workflows, .. } = *event {
                    assert_eq!(room_workflows.workflow_count, 0);
                    break;
                }
            }
        }
        let workflow_id = {
            let app = app.lock().await;
            let workflow = app
                .sessions_mut()
                .create_workflow(session.id(), Some("other-client".into()))
                .unwrap();
            let room = app.sessions().get_session(session.id()).unwrap();
            app.session_state_projection_store().update(room);
            workflow.id().to_string()
        };
        for count in [1, 0] {
            loop {
                if let KernelOutgoingFrame::Event { event, .. } = event_rx.recv().await.unwrap() {
                    if let KernelEvent::RoomWorkflowsChanged { inventory } = *event {
                        assert_eq!(inventory.workflow_count, count);
                        assert_eq!(inventory.session_id, session.id());
                        break;
                    }
                }
            }
            if count == 1 {
                let app = app.lock().await;
                let mut room = app.sessions().get_session(session.id()).unwrap();
                room.remove_workflow(&workflow_id);
                app.sessions_mut().restore_session(room.clone());
                app.session_state_projection_store().update(room);
            }
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    result.expect("one subscription delivers authoritative inventory changes");
}

// MP-08/MP-10/MP-11: exercise the actual local subscription, not just selectors.
#[tokio::test]
async fn task_delta_local_subscription_preserves_prompt_completion_and_provider_change() {
    let worktree = crate::test_support::TestWorktree::new("task-delta-local");
    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "room-workflows",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let prompt = crate::test_support::task_delta::TaskDeltaPrompt::start(
        &mut app,
        session.id(),
        attachment.id(),
    );
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app.clone(),
        1,
    ));
    let runtime = Arc::new(KernelTransportRuntime::default());
    let (priority_tx, _priority_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let (close_tx, _close_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run_subscription_loop(
        router,
        runtime,
        KernelOutgoingSender::new(priority_tx, event_tx),
        close_tx,
        Arc::new(AtomicBool::new(false)),
        KernelSubscription {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            subscription_scope: KernelSubscriptionScope::Session,
            connection_class: crate::local::KernelConnectionClass::Terminal,
        },
    ));
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let KernelOutgoingFrame::Event { event, .. } = event_rx.recv().await.unwrap() {
                if let KernelEvent::SessionSnapshot { session, .. } = *event {
                    assert_eq!(
                        session.agent_tasks()[0].state,
                        crate::durable_state::agent_lifecycle::ExecutionState::Working
                    );
                    break;
                }
            }
        }
        prompt.finish(&mut *app.lock().await, session.id());
        let mut saw_task = false;
        loop {
            if let KernelOutgoingFrame::Event { event, .. } = event_rx.recv().await.unwrap() {
                match *event {
                    KernelEvent::AgentTaskUpdated { task, .. } => {
                        assert_eq!(
                            task.state,
                            crate::durable_state::agent_lifecycle::ExecutionState::Done
                        );
                        saw_task = true;
                    }
                    KernelEvent::SessionSnapshot {
                        session,
                        provider_run,
                        agent_activity,
                        ..
                    } => {
                        if !saw_task {
                            continue;
                        }
                        assert!(session.active_prompt_for_agent(&prompt.agent).is_none());
                        assert!(!agent_activity[&prompt.agent].busy);
                        assert_eq!(
                            provider_run.as_ref().as_ref().unwrap().model(),
                            "updated-model"
                        );
                        break;
                    }
                    _ => {}
                }
            }
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    result.expect("task delta must not swallow prompt completion or provider changes");
}
