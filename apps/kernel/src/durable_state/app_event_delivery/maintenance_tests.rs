use super::*;
use crate::durable_state::app_event_maintenance::{
    AppEventMaintenanceOperation, AppEventMaintenanceOutcome,
};

fn queued_fixture(fixture: &Fixture) -> (DurableKernelStateStore, SessionService, String, Receipt) {
    let (store, mut sessions, session, receipt) = setup(fixture);
    let prepared = prepare(&store, &mut sessions, fixture, &receipt);
    let receipt = store
        .commit_app_event_queue(prepared.clone(), budget())
        .unwrap();
    sessions.restore_session(prepared.session().clone());
    (store, sessions, session, receipt)
}
fn status(store: &DurableKernelStateStore, fixture: &Fixture, id: &str) -> Receipt {
    match store
        .execute_app_state(
            "local",
            fixture.catalog.clone(),
            AppStateOperation::Status {
                receipt_id: id.into(),
            },
            budget(),
        )
        .unwrap()
    {
        AppStateOutcome::Receipt(value) => value,
        _ => panic!("expected receipt"),
    }
}
#[test]
fn real_workflow_run_persistence_and_receipt_delivery_share_one_transaction() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = queued_fixture(&fixture);
    assert_eq!(receipt.queued_session_id.as_deref(), Some(session.as_str()));
    let queued = sessions
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .front()
        .unwrap()
        .clone();
    let run = sessions
        .invoke_queued_workflow_endpoint(&session, &queued)
        .unwrap();
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_delivered BEFORE UPDATE OF state ON app_outbox WHEN NEW.state='delivered' BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END;").unwrap();
    assert!(store
        .persist_workflow_runtime_transition(
            &sessions.get_session(&session).unwrap(),
            "run_started"
        )
        .is_err());
    assert_eq!(status(&store, &fixture, &receipt.receipt_id), receipt);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM durable_workflow_runs WHERE run_id=?1",
            [run.id()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER reject_delivered").unwrap();
    store
        .persist_workflow_runtime_transition(
            &sessions.get_session(&session).unwrap(),
            "run_started",
        )
        .unwrap();
    let delivered = status(&store, &fixture, &receipt.receipt_id);
    assert_eq!(delivered.state, ReceiptState::Delivered);
    assert_eq!(delivered.revision, receipt.revision + 1);
    store
        .persist_workflow_runtime_transition(&sessions.get_session(&session).unwrap(), "repeat")
        .unwrap();
    assert_eq!(status(&store, &fixture, &receipt.receipt_id), delivered);
    drop(db);
    drop(store);
    let store = fixture.open();
    assert_eq!(status(&store, &fixture, &receipt.receipt_id), delivered);
}
#[test]
fn actual_queue_remove_atomically_fails_receipt_including_multi_session_writer() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = queued_fixture(&fixture);
    sessions
        .remove_queued_workflow_prompt(&session, receipt.queued_prompt_id.as_deref().unwrap())
        .unwrap();
    let before = sessions.get_session(&session).unwrap();
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_cancel BEFORE UPDATE OF state ON app_outbox WHEN NEW.state='failed' BEGIN SELECT RAISE(ABORT,'fixture cancelled receipt failure'); END;").unwrap();
    assert!(store
        .persist_workflow_runtime_sessions_transition(&[before.clone()], "queue_removed")
        .is_err());
    assert_eq!(status(&store, &fixture, &receipt.receipt_id), receipt);
    assert_eq!(queued_count(&db), 1);
    db.execute_batch("DROP TRIGGER reject_cancel").unwrap();
    store
        .persist_workflow_runtime_sessions_transition(&[before], "queue_removed")
        .unwrap();
    assert_eq!(
        status(&store, &fixture, &receipt.receipt_id).state,
        ReceiptState::Failed
    );
    assert_eq!(queued_count(&db), 0);
}
#[test]
fn restart_recovers_exact_legacy_queue_session_but_not_a_mismatched_association() {
    let fixture = Fixture::new();
    let (store, sessions, session, receipt) = queued_fixture(&fixture);
    let durable_owner = sessions
        .get_session(&session)
        .unwrap()
        .host_daemon_id()
        .to_owned();
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute(
        "UPDATE app_outbox SET queued_session_id=NULL WHERE receipt_id=?1",
        [&receipt.receipt_id],
    )
    .unwrap();
    drop(store);
    let store = fixture.open();
    let sweep = || AppEventMaintenanceOperation::Sweep {
        owner: "local".into(),
        installation: "installed".into(),
        durable_owner: durable_owner.clone(),
    };
    let AppEventMaintenanceOutcome::Swept {
        queued_sessions, ..
    } = store.maintain_app_events(sweep(), budget()).unwrap()
    else {
        panic!("expected sweep");
    };
    assert_eq!(queued_sessions, vec![session]);
    assert_eq!(status(&store, &fixture, &receipt.receipt_id), receipt);
    db.execute(
        "UPDATE app_outbox SET queued_session_id='different-session' WHERE receipt_id=?1",
        [&receipt.receipt_id],
    )
    .unwrap();
    let AppEventMaintenanceOutcome::Swept {
        queued_sessions, ..
    } = store.maintain_app_events(sweep(), budget()).unwrap()
    else {
        panic!("expected sweep");
    };
    assert!(queued_sessions.is_empty());
    assert_eq!(
        status(&store, &fixture, &receipt.receipt_id).state,
        ReceiptState::Queued
    );
}
