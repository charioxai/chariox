//! MP-08/MP-10/MP-11: independent subscriptions share a source, not owner disposition.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome, Registration};

#[tokio::test]
async fn a02_r8_blocked_subscriber_does_not_suppress_waiting_subscriber() {
    let worktree = crate::test_support::TestWorktree::new("am2-r8-shared-source");
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
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    for id in ["blocked-subscriber", "waiting-subscriber"] {
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: session.id().into(),
                agent: agent.id().into(),
                prompt: id.into(),
                run: Some(run.id().into()),
                now,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Subscribe {
                task: id.into(),
                prompt: id.into(),
                registration: Registration {
                    id: format!("reg-{id}"),
                    task_id: id.into(),
                    source_id: "shared-completion".into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true,
                },
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Yield {
                task: id.into(),
                prompt: id.into(),
                registrations: vec![format!("reg-{id}")],
                cursor: 0,
                deadline: now + 60_000,
                reason: "Wait for independent review".into(),
                now,
            })
            .unwrap();
    }
    store
        .agent_lifecycle(Operation::Block {
            task: "blocked-subscriber".into(),
            prompt: "blocked-subscriber".into(),
            reason: "Owner must reconcile this review".into(),
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: "waiting-subscriber".into(),
            run: run.id().into(),
            has_answer: false,
            cancelled: false,
            now,
        })
        .unwrap();
    assert_eq!(
        store
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == "waiting-subscriber")
            .unwrap()
            .state,
        ExecutionState::Waiting
    );
    store
        .agent_lifecycle(Operation::SourceOutcome {
            room: session.id().into(),
            source: "shared-completion".into(),
            occurrence: "finished".into(),
            success: true,
            public_answer: Some(serde_json::json!({"excerpt":"Real source result"})),
            now,
        })
        .unwrap();
    let blocked = store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == "blocked-subscriber")
        .unwrap();
    Box::pin(runtime.deliver_agent_inbox(session.id(), agent.id()))
        .await
        .unwrap();
    let event = store
        .agent_inbox(session.id(), agent.id(), 0)
        .unwrap()
        .remove(0);
    assert_eq!(
        event.state, "accepted",
        "MP-11: an independent waiting task must wake"
    );
    let tasks = store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap();
    let waiting = tasks
        .iter()
        .find(|t| t.task_id == "waiting-subscriber")
        .unwrap();
    assert_eq!(waiting.state, ExecutionState::Working);
    assert_eq!(waiting.prompt_id, event.prompt_id.as_deref().unwrap());
    let Outcome::Event(handled) = store
        .agent_lifecycle(Operation::Ack {
            room: session.id().into(),
            agent: agent.id().into(),
            sequence: event.sequence,
            handled: true,
            now,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(handled.state, "handled");
    assert_eq!(
        handled.payload["public_answer"]["excerpt"],
        "Real source result"
    );
    assert_eq!(
        store
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == blocked.task_id)
            .unwrap(),
        blocked,
        "MP-11: handling another task's wake preserves blocked owner reconciliation"
    );
}

#[tokio::test]
async fn a02_r9_cancelled_newest_task_does_not_suppress_older_completion() {
    Box::pin(older_wait_after_newer_cancellation(false)).await;
}

#[tokio::test]
async fn a02_r9_cancelled_newest_task_does_not_suppress_older_deadline() {
    Box::pin(older_wait_after_newer_cancellation(true)).await;
}

async fn older_wait_after_newer_cancellation(deadline: bool) {
    let worktree = crate::test_support::TestWorktree::new("am2-r9-cancelled-history");
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
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let id = "waiting-subscriber";
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: id.into(),
            run: Some(run.id().into()),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Subscribe {
            task: id.into(),
            prompt: id.into(),
            registration: Registration {
                id: format!("reg-{id}"),
                task_id: id.into(),
                source_id: "shared-completion".into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
            },
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Yield {
            task: id.into(),
            prompt: id.into(),
            registrations: vec![format!("reg-{id}")],
            cursor: 0,
            deadline: now + 60_000,
            reason: "Wait for independent review".into(),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: "waiting-subscriber".into(),
            run: run.id().into(),
            has_answer: false,
            cancelled: false,
            now,
        })
        .unwrap();
    assert_eq!(
        store
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == "waiting-subscriber")
            .unwrap()
            .state,
        ExecutionState::Waiting
    );
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: "cancelled-newer".into(),
            run: Some(run.id().into()),
            now,
        })
        .unwrap();
    let Outcome::Task(blocked) = store
        .agent_lifecycle(Operation::Block {
            task: "cancelled-newer".into(),
            prompt: "cancelled-newer".into(),
            reason: "Owner reconciliation for an independent task".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Task(cancelled) = store
        .agent_lifecycle(Operation::OwnerResponse {
            task: blocked.task_id,
            revision: blocked.blocked_revision,
            resume: false,
            now,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(cancelled.state, ExecutionState::Cancelled);
    if deadline {
        store
            .agent_lifecycle(Operation::Sweep {
                now: now + 60_001,
                busy_recipients: vec![],
                held_work: Vec::new(),
            })
            .unwrap();
    } else {
        store
            .agent_lifecycle(Operation::SourceOutcome {
                room: session.id().into(),
                source: "shared-completion".into(),
                occurrence: "finished".into(),
                success: true,
                public_answer: Some(serde_json::json!({"excerpt":"Real source result"})),
                now,
            })
            .unwrap();
    }
    Box::pin(runtime.deliver_agent_inbox(session.id(), agent.id()))
        .await
        .unwrap();
    let event = store
        .agent_inbox(session.id(), agent.id(), 0)
        .unwrap()
        .remove(0);
    assert_eq!(
        event.state, "accepted",
        "MP-11: an independent waiting task must wake"
    );
    let tasks = store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap();
    let waiting = tasks
        .iter()
        .find(|t| t.task_id == "waiting-subscriber")
        .unwrap();
    assert_eq!(waiting.state, ExecutionState::Working);
    assert_eq!(waiting.prompt_id, event.prompt_id.as_deref().unwrap());
    let Outcome::Event(handled) = store
        .agent_lifecycle(Operation::Ack {
            room: session.id().into(),
            agent: agent.id().into(),
            sequence: event.sequence,
            handled: true,
            now,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(handled.state, "handled");
    assert_eq!(
        handled.kind,
        if deadline {
            "deadline_reached"
        } else {
            "source_completed"
        }
    );
    assert_eq!(
        store
            .agent_tasks(Some(session.id()), Some(agent.id()))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == cancelled.task_id)
            .unwrap(),
        cancelled,
        "MP-11: handling another task's wake preserves independent task cancellation"
    );
}
