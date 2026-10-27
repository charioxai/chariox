use super::*;
use chariox_app_runtime::app_outbox::occurrence_id;

fn at(key: &str, now: u64) -> Occurrence {
    let mut value = occurrence(key, "value");
    value.occurred_at_ms = now;
    value.occurrence_id = occurrence_id(key, now).unwrap();
    value
}

#[test]
fn immutable_id_time_and_canonical_encoding_are_enforced() {
    assert_eq!(
        occurrence_id("source/é😀", 1000).unwrap(),
        "evt1.1000.21d560d98416f29e501984812520f520d02ac1907e3455d85bf17e219a8cd0cf"
    );
    assert_eq!(
        occurrence_id("a\"\n\\b", 0).unwrap(),
        "evt1.0.65a334110b948868e0415f900bc8df7682229878dcbabf555017ca49e1b8f4df"
    );
    let value = at("service:event/\"😀\"", 100);
    AppOutbox::validate_occurrences(std::slice::from_ref(&value)).unwrap();
    assert_eq!(occurrence_id("source", 0).unwrap().len(), 71);
    assert!(occurrence_id("", 1).is_err());
    assert!(occurrence_id(&"é".repeat(2049), 1).is_err());
    assert!(occurrence_id("source", MAX_SAFE_TIMESTAMP + 1).is_err());
    for bad in [
        value.occurrence_id.replacen(".100.", ".0100.", 1),
        value.occurrence_id.to_uppercase(),
        format!("{}.extra", value.occurrence_id),
        "old-arbitrary-id".into(),
    ] {
        let mut changed = value.clone();
        changed.occurrence_id = bad;
        assert!(matches!(
            AppOutbox::validate_occurrences(&[changed]),
            Err(OutboxError::Invalid)
        ));
    }
    let mut changed = value;
    changed.occurred_at_ms += 1;
    assert!(matches!(
        AppOutbox::validate_occurrences(&[changed]),
        Err(OutboxError::Invalid)
    ));
}

#[test]
fn retained_duplicate_precedes_floor_but_pruned_id_can_never_be_new() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let original = at("late-source", 100);
    let accepted_at = MAX_OCCURRENCE_AGE_MS + 100;
    let mut tx = db.transaction().unwrap();
    let receipt = AppOutbox::apply_in(
        &mut tx,
        &authority,
        std::slice::from_ref(&original),
        accepted_at,
    )
    .unwrap()
    .remove(0);
    let failed = AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &receipt.receipt_id,
        1,
        ReceiptState::Failed,
        accepted_at,
    )
    .unwrap();
    let maintenance =
        AppOutbox::maintain_in(&mut tx, "owner", "installed", accepted_at + 1).unwrap();
    assert_eq!(maintenance.minimum_occurred_at_ms, 101);
    assert_eq!(maintenance.pruned, 0);
    assert_eq!(
        AppOutbox::apply_in(
            &mut tx,
            &authority,
            std::slice::from_ref(&original),
            accepted_at + 1
        )
        .unwrap(),
        vec![failed]
    );
    let mut changed = original.clone();
    changed.payload = json!({"text":"changed"});
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[changed], accepted_at + 1),
        Err(OutboxError::Conflict)
    ));
    tx.commit().unwrap();
    drop(db);
    let mut db = directory.open();
    let mut tx = db.transaction().unwrap();
    let maintenance = AppOutbox::maintain_in(
        &mut tx,
        "owner",
        "installed",
        accepted_at + MAX_OCCURRENCE_AGE_MS,
    )
    .unwrap();
    assert_eq!(maintenance.pruned, 1);
    tx.commit().unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_in(
            &mut tx,
            &authority,
            &[original],
            accepted_at + MAX_OCCURRENCE_AGE_MS
        ),
        Err(OutboxError::TooOld)
    ));
}

#[test]
fn monotonic_floor_survives_reopen_and_clock_rollback_without_cross_owner_access() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let mut tx = db.transaction().unwrap();
    AppOutbox::maintain_in(&mut tx, "owner", "installed", MAX_OCCURRENCE_AGE_MS + 500).unwrap();
    tx.commit().unwrap();
    drop(db);
    let mut db = directory.open();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::maintain_in(&mut tx, "owner", "installed", 400)
            .unwrap()
            .minimum_occurred_at_ms,
        500
    );
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[at("fresh-clock", 400)], 400),
        Err(OutboxError::TooOld)
    ));
    assert!(matches!(
        AppOutbox::maintain_in(
            &mut tx,
            "another-owner",
            "installed",
            MAX_OCCURRENCE_AGE_MS + 999
        ),
        Err(OutboxError::NotFound)
    ));
    assert_eq!(
        AppOutbox::maintain_in(&mut tx, "owner", "installed", 500)
            .unwrap()
            .minimum_occurred_at_ms,
        500
    );
    AppOutbox::apply_in(&mut tx, &authority, &[at("at-floor", 500)], 500).unwrap();
}

#[test]
fn housekeeping_is_atomic_bounded_and_keeps_queued_and_recent_terminal_receipts() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let seed = accept(&mut db, &authority, "seed");
    let queued = accept(&mut db, &authority, "queued");
    let tx = db.transaction().unwrap();
    let queued =
        AppOutbox::mark_queued_in(&tx, &authority, &queued.receipt_id, 1, "queue-item", 101)
            .unwrap();
    assert_eq!(queued.queued_session_id.as_deref(), Some("session"));
    AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &seed.receipt_id,
        1,
        ReceiptState::Failed,
        101,
    )
    .unwrap();
    tx.commit().unwrap();
    db.execute("WITH RECURSIVE n(i) AS(SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<256)
       INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,invocation_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision)
       SELECT owner_id,installation_id,'retention-'||i,automation_id,event_version,'legacy-'||i,event_name,schema_digest,content_digest,automation_revision,accepted_generation,NULL,NULL,accepted_at_ms,expires_at_ms,'failed',1,0,next_attempt_at_ms,occurred_at_ms,schedule_revision FROM app_outbox,n WHERE receipt_id=?1",[&seed.receipt_id]).unwrap();
    let now = MAX_OCCURRENCE_AGE_MS + 102;
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::maintain_in(&mut tx, "owner", "installed", now)
            .unwrap()
            .pruned,
        256
    );
    drop(tx); // neither deletions nor replay floor can escape a failed outer writer transaction
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::maintain_in(&mut tx, "owner", "installed", now)
            .unwrap()
            .pruned,
        256
    );
    assert_eq!(
        AppOutbox::maintain_in(&mut tx, "owner", "installed", now)
            .unwrap()
            .pruned,
        1
    );
    assert_eq!(
        AppOutbox::status_in(&tx, &catalog, "owner", &queued.receipt_id).unwrap(),
        queued
    );
    AppOutbox::apply_in(&mut tx, &authority, &[at("next", now)], now).unwrap();
    tx.commit().unwrap();
}

#[test]
fn unsupported_pending_and_expiry_cannot_starve_live_contracts() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let old = accept(&mut db, &authority, "legacy");
    let current = accept(&mut db, &authority, "live");
    db.execute(
        "UPDATE app_outbox SET occurrence_id='protocol291-id' WHERE receipt_id=?1",
        [&old.receipt_id],
    )
    .unwrap();
    let mut tx = db.transaction().unwrap();
    let result = AppOutbox::maintain_in(&mut tx, "owner", "installed", 101).unwrap();
    assert_eq!(result.failed, 1);
    assert_eq!(result.expired, 0);
    let old = AppOutbox::status_in(&tx, &catalog, "owner", &old.receipt_id).unwrap();
    assert_eq!(old.state, ReceiptState::Failed);
    assert!(old.payload.is_none());
    assert_eq!(old.occurrence_id, "protocol291-id");
    assert_eq!(
        AppOutbox::pending_in(&tx, &catalog, "owner", 101, 16).unwrap(),
        vec![current.clone()]
    );
    assert_eq!(
        AppOutbox::maintain_in(&mut tx, "owner", "installed", current.expires_at_ms)
            .unwrap()
            .expired,
        1
    );
}

#[test]
fn final_failed_queue_attempt_is_counted_without_reactivating_receipt() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let mut receipt = accept(&mut db, &authority, "eight-failures");
    let tx = db.transaction().unwrap();
    for attempt in 0..7 {
        receipt = AppOutbox::mark_retryable_in(
            &tx,
            &authority,
            &receipt.receipt_id,
            receipt.revision,
            102 + attempt,
            101 + attempt,
        )
        .unwrap();
    }
    let failed = AppOutbox::mark_failed_attempt_in(
        &tx,
        &authority,
        &receipt.receipt_id,
        receipt.revision,
        108,
    )
    .unwrap();
    assert_eq!(failed.state, ReceiptState::Failed);
    assert_eq!(failed.attempts, 8);
    assert!(AppOutbox::mark_failed_attempt_in(
        &tx,
        &authority,
        &receipt.receipt_id,
        failed.revision,
        109
    )
    .is_err());
}
