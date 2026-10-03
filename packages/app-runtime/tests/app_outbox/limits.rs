use super::*;
use chariox_app_runtime::app_outbox::{
    MAX_ATTEMPTS, MAX_BATCH, MAX_BATCH_BYTES, MAX_PAYLOAD_BYTES, MAX_RETAINED_PAYLOAD_BYTES,
};
use serde_json::Value;

fn sized_occurrence(id: &str, bytes: usize) -> Occurrence {
    let overhead = serde_json::to_vec(&json!({"text":""})).unwrap().len()
        + serde_json::to_vec(&occurrence("size", "").invocation)
            .unwrap()
            .len();
    let value = occurrence(id, &"x".repeat(bytes.checked_sub(overhead).unwrap()));
    assert_eq!(
        serde_json::to_vec(&value.payload).unwrap().len()
            + serde_json::to_vec(&value.invocation).unwrap().len(),
        bytes
    );
    value
}

fn retained_bytes(db: &Connection) -> usize {
    let bytes: i64 = db
        .query_row(
            "SELECT coalesce(sum(coalesce(length(CAST(payload_json AS BLOB)),0)+coalesce(length(CAST(invocation_json AS BLOB)),0)),0) FROM app_outbox",
            [],
            |row| row.get(0),
        )
        .unwrap();
    bytes.try_into().unwrap()
}

#[test]
fn batch_item_and_encoded_byte_limits_accept_exactly_the_boundary_without_partial_writes() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let exact: Vec<_> = (0..MAX_BATCH)
        .map(|i| sized_occurrence(&format!("exact-{i}"), MAX_BATCH_BYTES / MAX_BATCH))
        .collect();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::apply_in(&mut tx, &authority, &exact, 100)
            .unwrap()
            .len(),
        MAX_BATCH
    );
    tx.commit().unwrap();
    assert_eq!(retained_bytes(&db), MAX_BATCH_BYTES);

    let too_many: Vec<_> = (0..=MAX_BATCH)
        .map(|i| occurrence(&format!("item-over-{i}"), ""))
        .collect();
    let over_bytes: Vec<_> = (0..MAX_BATCH)
        .map(|i| {
            sized_occurrence(
                &format!("byte-over-{i}"),
                MAX_BATCH_BYTES / MAX_BATCH + usize::from(i == 0),
            )
        })
        .collect();
    let mut tx = db.transaction().unwrap();
    for batch in [&too_many, &over_bytes] {
        assert!(matches!(
            AppOutbox::apply_in(&mut tx, &authority, batch, 100),
            Err(OutboxError::Limit)
        ));
    }
    tx.commit().unwrap();
    assert_eq!(count(&db), MAX_BATCH as i64);
    assert_eq!(retained_bytes(&db), MAX_BATCH_BYTES);
}

#[test]
fn retained_payload_limit_counts_utf8_bytes_and_terminal_cleanup_reopens_capacity() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let overhead = serde_json::to_vec(&json!({"text":""})).unwrap().len()
        + serde_json::to_vec(&occurrence("size", "").invocation)
            .unwrap()
            .len();
    let text_bytes = MAX_PAYLOAD_BYTES - overhead;
    let text = format!(
        "{}{}",
        "é".repeat(text_bytes / 2),
        "x".repeat(text_bytes % 2)
    );
    let full = occurrence("seed", &text);
    assert_eq!(
        serde_json::to_vec(&full.payload).unwrap().len()
            + serde_json::to_vec(&full.invocation).unwrap().len(),
        MAX_PAYLOAD_BYTES
    );
    let mut tx = db.transaction().unwrap();
    let seed = AppOutbox::apply_in(&mut tx, &authority, &[full], 100)
        .unwrap()
        .remove(0);
    tx.commit().unwrap();

    // Seed identical, bounded payloads through trusted fixture SQL so this test
    // exercises byte accounting without hundreds of schema/signature checks.
    let slots = MAX_RETAINED_PAYLOAD_BYTES / MAX_PAYLOAD_BYTES;
    assert_eq!(slots * MAX_PAYLOAD_BYTES, MAX_RETAINED_PAYLOAD_BYTES);
    db.execute(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
         INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision,invocation_json)
         SELECT owner_id,installation_id,'bytes-'||i,automation_id,event_version,'bytes-'||i,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,'accepted',1,0,next_attempt_at_ms,occurred_at_ms,schedule_revision,invocation_json FROM app_outbox,n WHERE receipt_id=?2",
        params![(slots - 2) as i64, seed.receipt_id],
    )
    .unwrap();
    assert_eq!(count(&db), (slots - 1) as i64);
    assert_eq!(
        retained_bytes(&db),
        MAX_RETAINED_PAYLOAD_BYTES - MAX_PAYLOAD_BYTES
    );

    let mut tx = db.transaction().unwrap();
    let last = AppOutbox::apply_in(&mut tx, &authority, &[occurrence("last", &text)], 100)
        .unwrap()
        .remove(0);
    tx.commit().unwrap();
    assert_eq!(retained_bytes(&db), MAX_RETAINED_PAYLOAD_BYTES);
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[occurrence("overflow", "")], 100),
        Err(OutboxError::Limit)
    ));
    assert_eq!(
        AppOutbox::apply_in(&mut tx, &authority, &[occurrence("seed", &text)], 100).unwrap()[0],
        seed
    );
    let settled = AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &last.receipt_id,
        last.revision,
        ReceiptState::Failed,
        101,
    )
    .unwrap();
    assert!(settled.payload.is_none());
    assert_eq!(settled.content_digest, last.content_digest);
    AppOutbox::apply_in(
        &mut tx,
        &authority,
        &[occurrence("replacement", &text)],
        101,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(retained_bytes(&db), MAX_RETAINED_PAYLOAD_BYTES);
    assert_eq!(count(&db), (slots + 1) as i64); // Terminal receipt was retained.
}

#[test]
fn seventh_retry_preserves_one_final_admission_and_cannot_extend_the_retry_budget() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let mut receipt = accept(&mut db, &authority, "retries");
    for attempt in 1..MAX_ATTEMPTS {
        let tx = db.transaction().unwrap();
        let now = receipt.next_attempt_at_ms;
        receipt = AppOutbox::mark_retryable_in(
            &tx,
            &authority,
            &receipt.receipt_id,
            receipt.revision,
            now + 1,
            now,
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(receipt.attempts, attempt);
    }
    let tx = db.transaction().unwrap();
    let now = receipt.next_attempt_at_ms;
    assert!(matches!(
        AppOutbox::mark_retryable_in(
            &tx,
            &authority,
            &receipt.receipt_id,
            receipt.revision,
            now + 1,
            now,
        ),
        Err(OutboxError::Limit)
    ));
    assert_eq!(
        AppOutbox::status_in(&tx, &catalog, "owner", &receipt.receipt_id).unwrap(),
        receipt
    );
    let queued = AppOutbox::mark_queued_in(
        &tx,
        &authority,
        &receipt.receipt_id,
        receipt.revision,
        "final-queue-admission",
        now,
    )
    .unwrap();
    assert_eq!(queued.attempts, MAX_ATTEMPTS);
    assert_eq!(queued.state, ReceiptState::Queued);
    assert!(queued.payload.is_none());
    for revision in [receipt.revision, queued.revision] {
        assert!(AppOutbox::mark_queued_in(
            &tx,
            &authority,
            &receipt.receipt_id,
            revision,
            "again",
            now
        )
        .is_err());
        assert!(AppOutbox::mark_retryable_in(
            &tx,
            &authority,
            &receipt.receipt_id,
            revision,
            now + 1,
            now
        )
        .is_err());
    }
    tx.commit().unwrap();
    assert_eq!(accept(&mut db, &authority, "retries"), queued);
}

#[test]
fn payload_tree_limits_precede_schema_validation_and_encoded_size_includes_escapes() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let mut tx = db.transaction().unwrap();
    // The declared text field is a string. A tree within structural bounds
    // reaches that schema rejection; one extra container/node stops earlier.
    for (array_depth, limited) in [(31, false), (32, true)] {
        let mut nested = Value::Null;
        for _ in 0..array_depth {
            nested = Value::Array(vec![nested]);
        }
        let mut value = occurrence("depth", "");
        value.payload = json!({"text":nested});
        let result = AppOutbox::apply_in(&mut tx, &authority, &[value], 100);
        if limited {
            assert!(matches!(result, Err(OutboxError::Limit)));
        } else {
            assert!(matches!(result, Err(OutboxError::Schema)));
        }
    }
    for (width, limited) in [(16_382, false), (16_383, true)] {
        let mut value = occurrence("breadth", "");
        value.payload = json!({"text":vec![0; width]});
        let result = AppOutbox::apply_in(&mut tx, &authority, &[value], 100);
        if limited {
            assert!(matches!(result, Err(OutboxError::Limit)));
        } else {
            assert!(matches!(result, Err(OutboxError::Schema)));
        }
    }
    let mut oversized_key = occurrence("key", "");
    oversized_key.payload = Value::Object(serde_json::Map::from_iter([(
        "k".repeat(MAX_PAYLOAD_BYTES),
        Value::Null,
    )]));
    for value in [
        oversized_key,
        occurrence("string", &"x".repeat(MAX_PAYLOAD_BYTES)),
        sized_occurrence(
            "encoded-over",
            MAX_PAYLOAD_BYTES
                + 1
                + serde_json::to_vec(&occurrence("size", "").invocation)
                    .unwrap()
                    .len(),
        ),
    ] {
        assert!(matches!(
            AppOutbox::apply_in(&mut tx, &authority, &[value], 100),
            Err(OutboxError::Limit)
        ));
    }
    let overhead = serde_json::to_vec(&json!({"text":""})).unwrap().len();
    let text = format!("{}x", "\n".repeat((MAX_PAYLOAD_BYTES - overhead) / 2));
    let exact = occurrence("escaped-exact", &text);
    assert_eq!(
        serde_json::to_vec(&exact.payload).unwrap().len(),
        MAX_PAYLOAD_BYTES
    );
    AppOutbox::apply_in(&mut tx, &authority, &[exact], 100).unwrap();
    assert!(matches!(
        AppOutbox::apply_in(
            &mut tx,
            &authority,
            &[occurrence("escaped-over", &format!("{text}x"))],
            100,
        ),
        Err(OutboxError::Limit)
    ));
    tx.commit().unwrap();
    assert_eq!(count(&db), 1);
}

#[test]
fn unchanged_automation_resumes_pending_receipt_with_current_catalog_after_update() {
    let directory = Database::new();
    let mut db = directory.open();
    let (mut package, trust, catalog) = setup(&mut db);
    let old = automation(&mut db, &catalog);
    let original = accept(&mut db, &old, "backlog");
    package.manifest.version = "2.0.0".into();
    let (_, current) = package.activate(&mut db, &trust, Some(1));
    assert_eq!(current.generation(), 2);
    drop(db);
    let mut db = directory.open();
    let authority = automation(&mut db, &current);
    let tx = db.transaction().unwrap();
    assert!(AppOutbox::mark_queued_in(
        &tx,
        &old,
        &original.receipt_id,
        original.revision,
        "stale-worker",
        200
    )
    .is_err());
    assert_eq!(
        AppOutbox::pending_in(&tx, &current, "owner", 200, 64).unwrap(),
        vec![original.clone()]
    );
    let queued = AppOutbox::mark_queued_in(
        &tx,
        &authority,
        &original.receipt_id,
        original.revision,
        "current-worker",
        200,
    )
    .unwrap();
    assert_eq!(queued.content_digest, original.content_digest);
    assert_eq!(queued.automation_revision, original.automation_revision);
    // Accepted generation is historical evidence. Current catalog trust and
    // the unchanged automation revision authorize resuming this backlog.
    let accepted_generation: i64 = tx
        .query_row(
            "SELECT accepted_generation FROM app_outbox WHERE receipt_id=?1",
            [&original.receipt_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(accepted_generation, 1);
    tx.commit().unwrap();
    assert_eq!(count(&db), 1);
}
