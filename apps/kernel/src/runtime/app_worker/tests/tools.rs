use super::*;
use crate::{
    durable_state::{app_publishers::AppPublisherMutation, app_tools::AppToolsError},
    runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped},
};
use chariox_app_runtime::{
    app_catalog::{Actor, CallerContext},
    publisher_trust::TrustDecision,
};
use serde_json::json;
use std::sync::atomic::AtomicBool;

fn caller() -> CallerContext {
    CallerContext {
        actor: Actor::Agent("agent".into()),
        room_id: "room".into(),
        operation_id: "fixed-native-tool-call".into(),
        task_id: None,
        turn_id: None,
    }
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}

#[test]
fn real_sdk_tool_roundtrip_is_validated_and_current_signer_revoke_prevents_enqueue() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
    let tool = catalog.app_catalog().tools().next().unwrap().name.clone();
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let (starting, mut events, observed) = start(
        &fixture,
        Mode::ToolEcho,
        &runtime,
        catalog,
        broker(|_| Box::pin(async { Ok(Value::Null) })),
    );
    let (owner, handle) = activate(starting.await_registered_blocking(WAIT).unwrap(), &store);
    event(&runtime, &mut events, "worker.fixture.ready_ack");
    let lease = handle.lease("alice").unwrap();
    let invalid = store.enqueue_app_tool(
        lease.reserve_call(WAIT).unwrap(),
        &tool,
        json!({"unexpected":true}),
        caller(),
        budget(),
    );
    assert!(matches!(
        invalid,
        Err(AppToolsError::Tool(AppToolError::Catalog(
            chariox_app_runtime::app_catalog::CatalogError::Input
        )))
    ));
    assert_eq!(observed.tool_invocations(), 0);
    let response = store
        .enqueue_app_tool(
            lease.reserve_call(WAIT).unwrap(),
            &tool,
            json!({"text":"fixture"}),
            caller(),
            budget(),
        )
        .unwrap();
    let reply = runtime.block_on(response.receive()).unwrap();
    assert_eq!(
        store.accept_app_tool_reply(reply).unwrap(),
        json!({"ok":true})
    );
    assert_eq!(observed.tool_invocations(), 1);
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke-tool".into(),
                    authority_ref: "kernel-fixture".into(),
                },
                now_ms: 10,
            },
        )
        .unwrap();
    let revoked = store.enqueue_app_tool(
        lease.reserve_call(WAIT).unwrap(),
        &tool,
        json!({"text":"must not execute"}),
        caller(),
        budget(),
    );
    assert!(matches!(
        revoked,
        Err(AppToolsError::Tool(AppToolError::Catalog(_)))
    ));
    assert_eq!(observed.tool_invocations(), 1);
    owner.shutdown_blocking();
}

#[test]
fn cancellation_during_real_sqlite_writer_wait_never_reaches_sdk() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
    let tool = catalog.app_catalog().tools().next().unwrap().name.clone();
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let (starting, mut events, observed) = start(
        &fixture,
        Mode::ToolEcho,
        &runtime,
        catalog,
        broker(|_| Box::pin(async { Ok(Value::Null) })),
    );
    let (owner, handle) = activate(starting.await_registered_blocking(WAIT).unwrap(), &store);
    event(&runtime, &mut events, "worker.fixture.ready_ack");
    let slot = handle.lease("alice").unwrap().reserve_call(WAIT).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let observe = cancelled.clone();
    let checks = Arc::new(AtomicUsize::new(0));
    let (started, waiting) = mpsc::channel();
    let budget = AppOperationBudget::from_supervisor(move || observe.load(Ordering::Acquire))
        .fixture_observe_checks(Arc::new(move || {
            if checks.fetch_add(1, Ordering::AcqRel) == 1 {
                let _ = started.send(());
            }
        }));
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let copied = store.clone();
    let (send, response) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = send.send(copied.enqueue_app_tool(
            slot,
            &tool,
            json!({"text":"cancelled"}),
            caller(),
            budget,
        ));
    });
    let waiting_result = waiting.recv_timeout(WAIT);
    cancelled.store(true, Ordering::Release);
    // Release the real lock even if the observation fails, then join the actual
    // admission task before asserting. This is not a mocked SQLite commit.
    connection.execute_batch("ROLLBACK").unwrap();
    let result = response.recv_timeout(WAIT);
    worker.join().unwrap();
    waiting_result.unwrap();
    assert!(matches!(
        result.unwrap(),
        Err(AppToolsError::Stopped(AppOperationStopped::Cancelled))
    ));
    assert_eq!(observed.tool_invocations(), 0);
    owner.shutdown_blocking();
}
