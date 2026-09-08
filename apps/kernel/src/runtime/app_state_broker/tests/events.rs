use super::integration::{change, receive, send, start, Fixture, WAIT};
use super::*;
use crate::durable_state::{
    app_publishers::AppPublisherMutation, app_state::fixture_event_catalog,
};
use chariox_app_runtime::{
    app_outbox::{AppOutbox, Receipt, ReceiptState, VerifiedAutomation},
    publisher_trust::TrustDecision,
};
use rusqlite::Connection;
use tokio::time::timeout;

pub(super) fn automations(store: &DurableKernelStateStore, catalog: &EventCatalog) {
    // Trusted fixture setup only: actual configuration verifies existing owned
    // workflow targets through its separate kernel permission/writer path.
    let db = Connection::open(store.path()).unwrap();
    for id in ["auto-a", "auto-b"] {
        db.execute("INSERT INTO app_automations(owner_id,installation_id,automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status)
            VALUES('alice','installed',?1,1,'changed',1,?2,'session','publication','endpoint','queue','active')",
            rusqlite::params![id,catalog.schema_digest("changed").unwrap()]).unwrap();
    }
}
pub(super) fn occurrence(automation: &str, id: &str) -> Value {
    let occurred_at = crate::session::unix_epoch_ms();
    let id = chariox_app_runtime::app_outbox::occurrence_id(id, occurred_at).unwrap();
    json!({"automationId":automation,"occurrenceId":id,"eventVersion":1,
        "occurredAtMs":occurred_at,"payload":{"text":"changed"},
        "invocation":{"prompt":"Handle the declared event","artifacts":[]}})
}
pub(super) fn count(store: &DurableKernelStateStore) -> i64 {
    Connection::open(store.path())
        .unwrap()
        .query_row("SELECT count(*) FROM app_outbox", [], |row| row.get(0))
        .unwrap()
}
fn status(store: &DurableKernelStateStore, catalog: &EventCatalog, id: &str) -> Receipt {
    let mut db = Connection::open(store.path()).unwrap();
    let tx = db.transaction().unwrap();
    AppOutbox::status_in(&tx, catalog, "alice", id).unwrap()
}

#[test]
fn event_decoder_rejects_authority_overrides_incomplete_bodies_and_global_overflow() {
    let good = occurrence("auto-a", "event");
    assert!(decode::operation("events.emit", good.clone()).is_ok());
    for key in [
        "owner",
        "installationId",
        "generation",
        "workflowId",
        "automationRevision",
    ] {
        let mut bad = good.clone();
        bad[key] = json!("forged");
        assert_eq!(rejection("events.emit", bad).code, "INVALID_ARGUMENT");
    }
    for key in ["invocation", "payload", "occurredAtMs"] {
        let mut bad = good.clone();
        bad.as_object_mut().unwrap().remove(key);
        assert_eq!(rejection("events.emit", bad).code, "INVALID_ARGUMENT");
    }
    for field in ["sizeBytes", "digest"] {
        let mut bad = good.clone();
        bad["invocation"]["artifacts"] =
            json!([{"name":"context","mediaType":"text/plain","reference":"opaque"}]);
        bad["invocation"]["artifacts"][0][field] = Value::Null;
        assert_eq!(rejection("events.emit", bad).code, "INVALID_ARGUMENT");
    }
    for method in ["events.status", "events.retry"] {
        assert!(decode::operation(method, json!({"receiptId":"receipt"})).is_ok());
        assert_eq!(
            rejection(method, json!({"receiptId":"receipt","owner":"forged"})).code,
            "INVALID_ARGUMENT"
        );
        assert_eq!(
            rejection(method, json!({"receiptId":null})).code,
            "INVALID_ARGUMENT"
        );
    }
    let occurrences: Vec<_> = (0..16)
        .map(|i| {
            let mut event = occurrence(&format!("auto-{i}"), "event");
            event["invocation"]["prompt"] = json!("x".repeat(32768));
            event
        })
        .collect();
    let mut transaction = transaction();
    transaction["occurrences"] = json!(occurrences);
    assert_eq!(
        rejection("state.transaction", transaction).code,
        "LIMIT_EXCEEDED"
    );
}

#[tokio::test]
async fn one_peer_commits_state_and_multiple_automations_before_returning_minimal_receipts() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    automations(&store, &catalog);
    let (peer, mut worker, task) = start(&store, catalog, "alice", Arc::new(Semaphore::new(8)));
    let mut first = occurrence("auto-a", "same-id");
    first["invocation"]["artifacts"] = json!([{
        "name":"Context","mediaType":"text/plain","reference":"file:///not-authorized",
        "sizeBytes":0,"digest":format!("sha256:{}","a".repeat(64))
    }]);
    let mut second = first.clone();
    second["automationId"] = json!("auto-b");
    let mut transaction = change(json!("saved"));
    transaction["occurrences"] = json!([first.clone(), second]);
    send(&mut worker, "atomic", "state.transaction", transaction).await;
    let committed = receive(&mut worker).await.unwrap();
    assert_eq!(committed["revision"], 1);
    let receipts = committed["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 2);
    for receipt in receipts {
        assert_eq!(receipt.as_object().unwrap().len(), 2);
        assert_eq!(receipt["state"], "accepted");
    }
    let first_id = receipts[0]["receiptId"].as_str().unwrap().to_owned();
    assert_ne!(receipts[0]["receiptId"], receipts[1]["receiptId"]);
    assert_eq!(count(&store), 2); // A separate connection sees the committed rows.
    send(&mut worker, "replay", "events.emit", first).await;
    assert_eq!(receive(&mut worker).await.unwrap(), receipts[0]);
    send(
        &mut worker,
        "receipt",
        "events.status",
        json!({"receiptId":first_id}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap(), receipts[0]);
    send(&mut worker, "state", "state.get", json!({"key":"status"})).await;
    assert_eq!(
        receive(&mut worker).await.unwrap(),
        json!({"value":"saved","version":1})
    );
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn failure_in_second_automation_rolls_back_first_receipt_and_state_on_the_real_writer() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    automations(&store, &catalog);
    let (peer, mut worker, task) = start(&store, catalog, "alice", Arc::new(Semaphore::new(8)));
    let existing = occurrence("auto-b", "existing");
    send(&mut worker, "seed", "events.emit", existing.clone()).await;
    receive(&mut worker).await.unwrap();
    let mut conflicting = existing;
    conflicting["invocation"]["prompt"] = json!("different immutable work");
    let mut mutation = change(json!("must roll back"));
    mutation["occurrences"] = json!([occurrence("auto-a", "first"), conflicting]);
    send(&mut worker, "conflict", "state.transaction", mutation).await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CONFLICT");
    assert_eq!(count(&store), 1);
    let db = Connection::open(store.path()).unwrap();
    let fault = occurrence("auto-b", "fault");
    // The generated canonical ID contains only the fixed prefix, digits, dots
    // and lowercase hex. Use this exact occurrence in both the trigger and call.
    let fault_id = fault["occurrenceId"].as_str().unwrap();
    db.execute_batch(&format!("CREATE TRIGGER fixture_event_failure BEFORE INSERT ON app_outbox WHEN NEW.occurrence_id='{fault_id}' BEGIN SELECT RAISE(ABORT,'private failure detail'); END;")).unwrap();
    let mut mutation = change(json!("also must roll back"));
    mutation["occurrences"] = json!([occurrence("auto-a", "first"), fault]);
    send(&mut worker, "fault", "state.transaction", mutation).await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "UNAVAILABLE");
    db.execute_batch("DROP TRIGGER fixture_event_failure;")
        .unwrap();
    assert_eq!(count(&store), 1);
    send(&mut worker, "state", "state.get", json!({"key":"status"})).await;
    assert_eq!(receive(&mut worker).await.unwrap(), Value::Null);
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn retry_reconciles_only_current_due_classifications_without_changing_backoff_or_attempts() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    automations(&store, &catalog);
    let (peer, mut worker, task) = start(
        &store,
        catalog.clone(),
        "alice",
        Arc::new(Semaphore::new(8)),
    );
    let mut ids = Vec::new();
    for id in ["due", "future", "failed"] {
        send(&mut worker, id, "events.emit", occurrence("auto-a", id)).await;
        ids.push(
            receive(&mut worker).await.unwrap()["receiptId"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    send(
        &mut worker,
        "not-classified",
        "events.retry",
        json!({"receiptId":ids[0]}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CONFLICT");
    let mut db = Connection::open(store.path()).unwrap();
    let tx = db.transaction().unwrap();
    let automation =
        VerifiedAutomation::load_in(&tx, catalog.clone(), "alice", "auto-a", 1).unwrap();
    let now = crate::session::unix_epoch_ms();
    let due = AppOutbox::mark_retryable_in(&tx, &automation, &ids[0], 1, now + 1, now).unwrap();
    let future =
        AppOutbox::mark_retryable_in(&tx, &automation, &ids[1], 1, now + 60_000, now).unwrap();
    AppOutbox::settle_in(
        &tx,
        &catalog,
        "alice",
        &ids[2],
        1,
        ReceiptState::Failed,
        now,
    )
    .unwrap();
    tx.commit().unwrap();
    timeout(WAIT, async {
        while crate::session::unix_epoch_ms() < due.next_attempt_at_ms {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    for attempt in 0..2 {
        send(
            &mut worker,
            &format!("retry-{attempt}"),
            "events.retry",
            json!({"receiptId":ids[0]}),
        )
        .await;
        assert_eq!(
            receive(&mut worker).await.unwrap(),
            json!({"receiptId":ids[0],"state":"retryable"})
        );
        assert_eq!(status(&store, &catalog, &ids[0]), due);
    }
    for id in [&ids[1], &ids[2]] {
        send(
            &mut worker,
            "unretryable",
            "events.retry",
            json!({"receiptId":id}),
        )
        .await;
        assert_eq!(receive(&mut worker).await.unwrap_err(), "CONFLICT");
    }
    assert_eq!(status(&store, &catalog, &ids[1]), future);
    db.execute(
        "UPDATE app_automations SET revision=2 WHERE automation_id='auto-a'",
        [],
    )
    .unwrap();
    send(
        &mut worker,
        "changed-binding",
        "events.retry",
        json!({"receiptId":ids[0]}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CONFLICT");
    assert_eq!(status(&store, &catalog, &ids[0]), due);
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn receipt_queries_stay_owner_scoped_and_recheck_same_generation_publisher_revocation() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    automations(&store, &catalog);
    let (peer, mut worker, task) = start(&store, catalog, "alice", Arc::new(Semaphore::new(8)));
    send(
        &mut worker,
        "emit",
        "events.emit",
        occurrence("auto-a", "private"),
    )
    .await;
    let id = receive(&mut worker).await.unwrap()["receiptId"]
        .as_str()
        .unwrap()
        .to_owned();
    let db = Connection::open(store.path()).unwrap();
    // Trusted fixture mutation isolates the SQL owner predicate without granting
    // the App a second installation or access to another owner's event content.
    db.execute(
        "UPDATE app_outbox SET owner_id='bob' WHERE receipt_id=?1",
        [&id],
    )
    .unwrap();
    send(
        &mut worker,
        "other-owner",
        "events.status",
        json!({"receiptId":id}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "NOT_FOUND");
    db.execute(
        "UPDATE app_outbox SET owner_id='alice' WHERE receipt_id=?1",
        [&id],
    )
    .unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke".into(),
                    authority_ref: "kernel-fixture".into(),
                },
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .unwrap();
    for method in ["events.status", "events.retry"] {
        send(&mut worker, "revoked", method, json!({"receiptId":id})).await;
        assert_eq!(receive(&mut worker).await.unwrap_err(), "APP_UNAVAILABLE");
    }
    send(
        &mut worker,
        "revoked-emit",
        "events.emit",
        occurrence("auto-a", "new"),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "APP_UNAVAILABLE");
    assert_eq!(count(&store), 1);
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}
