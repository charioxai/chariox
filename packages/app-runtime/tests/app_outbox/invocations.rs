use super::*;
use chariox_app_runtime::app_outbox::{
    Artifact, AutomationTarget, MAX_BATCH_BYTES, MAX_INVOCATION_BYTES, MAX_PROMPT_BYTES,
};

#[test]
fn invocation_is_required_strict_metadata_and_part_of_durable_identity() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let mut value = occurrence("canonical-work", "domain payload");
    value.invocation.artifacts = vec![Artifact {
        name: "untrusted.txt".into(),
        media_type: "text/plain".into(),
        reference: "file:///private/kernel-file".into(),
        size_bytes: Some(17),
        digest: None,
    }];
    let original_json = serde_json::to_value(&value).unwrap();
    let mut malformed = original_json.clone();
    malformed.as_object_mut().unwrap().remove("invocation");
    assert!(serde_json::from_value::<Occurrence>(malformed).is_err());
    for key in ["sizeBytes", "digest"] {
        let mut malformed = original_json.clone();
        malformed["invocation"]["artifacts"][0][key] = json!(null);
        assert!(serde_json::from_value::<Occurrence>(malformed).is_err());
    }
    let first = {
        let mut tx = db.transaction().unwrap();
        let receipt =
            AppOutbox::apply_current_in(&mut tx, catalog.clone(), "owner", &[value.clone()], 100)
                .unwrap()
                .remove(0);
        tx.commit().unwrap();
        receipt
    };
    drop(db);
    let mut db = directory.open();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::apply_current_in(&mut tx, catalog.clone(), "owner", &[value.clone()], 101)
            .unwrap()[0],
        first
    );
    let stored: Invocation = serde_json::from_str(first.invocation.as_deref().unwrap()).unwrap();
    assert_eq!(stored.prompt, "Handle the event");
    assert_eq!(stored.artifacts[0].reference, "file:///private/kernel-file");
    value.invocation.prompt.push_str(" differently");
    assert!(matches!(
        AppOutbox::apply_current_in(&mut tx, catalog.clone(), "owner", &[value], 102),
        Err(OutboxError::Conflict)
    ));
}

#[test]
fn shape_budgets_count_canonical_invocation_and_payload_before_schema_admission() {
    let mut value = occurrence("bounds", "x");
    value.invocation.prompt = "x".repeat(MAX_PROMPT_BYTES);
    AppOutbox::validate_occurrences(&[value.clone()]).unwrap();
    value.invocation.prompt.push('x');
    assert!(matches!(
        AppOutbox::validate_occurrences(&[value.clone()]),
        Err(OutboxError::Limit)
    ));
    value.invocation.prompt = "\u{2003}".into();
    assert!(matches!(
        AppOutbox::validate_occurrences(&[value.clone()]),
        Err(OutboxError::Invalid)
    ));
    value.invocation.prompt = "\u{0001}".repeat(MAX_PROMPT_BYTES);
    assert!(serde_json::to_vec(&value.invocation).unwrap().len() > MAX_INVOCATION_BYTES);
    assert!(matches!(
        AppOutbox::validate_occurrences(&[value]),
        Err(OutboxError::Limit)
    ));
    let values: Vec<_> = (0..9)
        .map(|i| {
            let mut v = occurrence(&format!("large-{i}"), "x");
            v.invocation.prompt = "x".repeat(MAX_PROMPT_BYTES);
            v
        })
        .collect();
    assert!(
        values
            .iter()
            .map(|v| serde_json::to_vec(&v.invocation).unwrap().len())
            .sum::<usize>()
            > MAX_BATCH_BYTES
    );
    assert!(matches!(
        AppOutbox::validate_occurrences(&values),
        Err(OutboxError::Limit)
    ));
}

#[test]
fn multiple_current_automations_share_one_atomic_batch_and_global_budget() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let tx = db.transaction().unwrap();
    AppOutbox::configure_in(
        &tx,
        &catalog,
        "owner",
        "second",
        0,
        "changed",
        &AutomationTarget {
            session_id: "session".into(),
            publication_id: "publication".into(),
            endpoint_id: "endpoint".into(),
            queue_id: "default".into(),
        },
        false,
    )
    .unwrap();
    tx.commit().unwrap();
    let mut second = occurrence("same-id", "two");
    second.automation_id = "second".into();
    let mut tx = db.transaction().unwrap();
    let accepted = AppOutbox::apply_current_in(
        &mut tx,
        catalog.clone(),
        "owner",
        &[occurrence("same-id", "one"), second.clone()],
        100,
    )
    .unwrap();
    assert_eq!(accepted.len(), 2);
    assert_ne!(accepted[0].receipt_id, accepted[1].receipt_id);
    tx.commit().unwrap();
    second.invocation.prompt = "changed".into();
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_current_in(
            &mut tx,
            catalog.clone(),
            "owner",
            &[occurrence("must-rollback", "one"), second],
            101
        ),
        Err(OutboxError::Conflict)
    ));
    tx.commit().unwrap();
    assert_eq!(count(&db), 2);
    let mut batch: Vec<_> = (0..16)
        .map(|i| {
            let mut v = occurrence(&format!("budget-{i}"), "x");
            v.automation_id = if i % 2 == 0 { "automation" } else { "second" }.into();
            v.invocation.prompt = "x".repeat(MAX_PROMPT_BYTES);
            v
        })
        .collect();
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_current_in(&mut tx, catalog, "owner", &batch, 102),
        Err(OutboxError::Limit)
    ));
    batch.clear();
    tx.commit().unwrap();
    assert_eq!(count(&db), 2);
}

#[test]
fn retry_acknowledges_only_current_due_kernel_classification_without_mutating_budget() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let accepted = accept(&mut db, &authority, "retry");
    let tx = db.transaction().unwrap();
    assert!(
        AppOutbox::reconcile_retry_in(&tx, &catalog, "owner", &accepted.receipt_id, 100).is_err()
    );
    let retry =
        AppOutbox::mark_retryable_in(&tx, &authority, &accepted.receipt_id, 1, 200, 100).unwrap();
    assert!(
        AppOutbox::reconcile_retry_in(&tx, &catalog, "owner", &accepted.receipt_id, 199).is_err()
    );
    assert_eq!(
        AppOutbox::reconcile_retry_in(&tx, &catalog, "owner", &accepted.receipt_id, 200).unwrap(),
        retry
    );
    assert_eq!(
        AppOutbox::reconcile_retry_in(&tx, &catalog, "owner", &accepted.receipt_id, 201).unwrap(),
        retry
    );
    assert!(
        AppOutbox::reconcile_retry_in(&tx, &catalog, "other", &accepted.receipt_id, 201).is_err()
    );
    tx.execute(
        "UPDATE app_automations SET revision=revision+1 WHERE automation_id='automation'",
        [],
    )
    .unwrap();
    assert!(
        AppOutbox::reconcile_retry_in(&tx, &catalog, "owner", &accepted.receipt_id, 201).is_err()
    );
}

#[test]
fn incoming_only_signed_events_cannot_be_configured_as_outgoing_automations() {
    let directory = Database::new();
    let mut db = directory.open();
    let (mut package, trust, _) = setup(&mut db);
    let mut declarations: serde_json::Value =
        serde_json::from_slice(&package.files["schemas/events.json"]).unwrap();
    declarations["events"][0]["direction"] = json!("incoming");
    package.files.insert(
        "schemas/events.json".into(),
        serde_json::to_vec(&declarations).unwrap(),
    );
    package.manifest.version = "1.1.0".into();
    let (_, catalog) = package.activate(&mut db, &trust, Some(1));
    assert_eq!(catalog.event_version("changed"), None);
    let tx = db.transaction().unwrap();
    assert!(VerifiedAutomation::load_in(&tx, catalog, "owner", "automation", 1).is_err());
}

#[test]
fn unsupported_old_pending_rows_are_terminalized_without_losing_receipts_or_starving_delivery() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let old = accept(&mut db, &authority, "old");
    // Model protocol290 storage with no invocation column and >one pending page.
    db.execute_batch("ALTER TABLE app_outbox DROP COLUMN invocation_json;")
        .unwrap();
    db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<70)
        INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision)
        SELECT owner_id,installation_id,'old-'||i,automation_id,event_version,'old-'||i,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,'accepted',1,0,next_attempt_at_ms,occurred_at_ms,schedule_revision FROM app_outbox,n WHERE receipt_id=?1",[&old.receipt_id]).unwrap();
    drop(db);
    let mut db = directory.open();
    let tx = db.transaction().unwrap();
    let migrated = AppOutbox::status_in(&tx, &catalog, "owner", &old.receipt_id).unwrap();
    assert_eq!(migrated.state, ReceiptState::Failed);
    assert_eq!(migrated.content_digest, old.content_digest);
    assert!(migrated.payload.is_none());
    assert!(migrated.invocation.is_none());
    assert!(AppOutbox::pending_in(&tx, &catalog, "owner", 100, 64)
        .unwrap()
        .is_empty());
    tx.commit().unwrap();
    assert_eq!(count(&db), 71);
    let new = accept(&mut db, &authority, "new");
    let tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::pending_in(&tx, &catalog, "owner", 100, 64).unwrap(),
        vec![new.clone()]
    );
    tx.execute(
        "UPDATE app_outbox SET invocation_json=NULL WHERE receipt_id=?1",
        [&new.receipt_id],
    )
    .unwrap();
    assert!(AppOutbox::mark_queued_in(
        &tx,
        &authority,
        &new.receipt_id,
        new.revision,
        "forbidden",
        100
    )
    .is_err());
}
