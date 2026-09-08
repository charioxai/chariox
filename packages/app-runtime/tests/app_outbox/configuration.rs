use super::*;
use chariox_app_runtime::app_outbox::{AutomationStatus, AutomationTarget, MAX_AUTOMATIONS};

fn target(queue: &str) -> AutomationTarget {
    AutomationTarget {
        session_id: "session".into(),
        publication_id: "publication".into(),
        endpoint_id: "endpoint".into(),
        queue_id: queue.into(),
    }
}
#[test]
fn configuration_cas_fences_pending_receipts_and_explicit_reactivation() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let old = automation(&mut db, &catalog);
    let receipt = accept(&mut db, &old, "pending-before-change");
    let tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::configure_in(
            &tx,
            &catalog,
            "owner",
            "automation",
            0,
            "changed",
            &target("new-queue"),
            false
        ),
        Err(OutboxError::Conflict)
    ));
    let changed = AppOutbox::configure_in(
        &tx,
        &catalog,
        "owner",
        "automation",
        1,
        "changed",
        &target("new-queue"),
        false,
    )
    .unwrap();
    assert_eq!(changed.revision, 2);
    tx.commit().unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(AppOutbox::apply_in(&mut tx, &old, &[occurrence("later", "value")], 100).is_err());
    let new = VerifiedAutomation::load_in(&tx, catalog.clone(), "owner", "automation", 2).unwrap();
    assert!(matches!(
        AppOutbox::mark_queued_in(&tx, &new, &receipt.receipt_id, 1, "queue-item", 101),
        Err(OutboxError::Conflict)
    ));
    let paused = AppOutbox::deactivate_in(
        &tx,
        &catalog,
        "owner",
        "automation",
        2,
        AutomationStatus::Paused,
    )
    .unwrap();
    assert_eq!(paused.revision, 3);
    assert!(VerifiedAutomation::load_in(&tx, catalog.clone(), "owner", "automation", 3).is_err());
    assert!(matches!(
        AppOutbox::deactivate_in(
            &tx,
            &catalog,
            "owner",
            "automation",
            3,
            AutomationStatus::Active
        ),
        Err(OutboxError::Invalid)
    ));
    tx.commit().unwrap();
    drop(db);
    let mut db = directory.open();
    let tx = db.transaction().unwrap();
    let current = AppOutbox::configuration_in(&tx, &catalog, "owner", "automation").unwrap();
    assert_eq!(current.status, AutomationStatus::Paused);
    let active = AppOutbox::configure_in(
        &tx,
        &catalog,
        "owner",
        "automation",
        3,
        "changed",
        &target("new-queue"),
        false,
    )
    .unwrap();
    assert_eq!(active.revision, 4);
    assert_eq!(
        AppOutbox::status_in(&tx, &catalog, "owner", &receipt.receipt_id).unwrap(),
        receipt
    );
    tx.commit().unwrap();
}
#[test]
fn configuration_is_atomic_bounded_and_never_grants_cross_owner_or_revoked_access() {
    let directory = Database::new();
    let mut db = directory.open();
    let (package, _, catalog) = setup(&mut db);
    let tx = db.transaction().unwrap();
    assert!(AppOutbox::configure_in(
        &tx,
        &catalog,
        "other",
        "new",
        0,
        "changed",
        &target("queue"),
        false
    )
    .is_err());
    assert!(matches!(
        AppOutbox::configure_in(
            &tx,
            &catalog,
            "owner",
            "new",
            0,
            "undeclared",
            &target("queue"),
            false
        ),
        Err(OutboxError::Schema)
    ));
    AppOutbox::configure_in(
        &tx,
        &catalog,
        "owner",
        "new",
        0,
        "changed",
        &target("queue"),
        false,
    )
    .unwrap();
    drop(tx);
    let tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::configuration_in(&tx, &catalog, "owner", "new"),
        Err(OutboxError::NotFound)
    ));
    for index in 1..MAX_AUTOMATIONS {
        AppOutbox::configure_in(
            &tx,
            &catalog,
            "owner",
            &format!("automation-{index}"),
            0,
            "changed",
            &target("queue"),
            false,
        )
        .unwrap();
    }
    assert!(matches!(
        AppOutbox::configure_in(
            &tx,
            &catalog,
            "owner",
            "overflow",
            0,
            "changed",
            &target("queue"),
            false
        ),
        Err(OutboxError::Limit)
    ));
    assert_eq!(
        AppOutbox::configurations_in(&tx, &catalog, "owner")
            .unwrap()
            .len(),
        MAX_AUTOMATIONS
    );
    AppOutbox::deactivate_in(
        &tx,
        &catalog,
        "owner",
        "automation",
        1,
        AutomationStatus::Disabled,
    )
    .unwrap();
    tx.commit().unwrap();
    PublisherTrustRegistry::new(&mut db)
        .revoke(
            "owner",
            &package.publisher().publisher_id,
            &package.publisher().key_id,
            1,
            &decision("revoke-config"),
            200,
        )
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(AppOutbox::configure_in(
        &tx,
        &catalog,
        "owner",
        "automation",
        2,
        "changed",
        &target("queue"),
        false
    )
    .is_err());
}
#[test]
fn configuration_revision_exhaustion_never_wraps_or_partially_changes_target() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    db.execute("UPDATE app_automations SET revision=?1", [i64::MAX])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::configure_in(
            &tx,
            &catalog,
            "owner",
            "automation",
            i64::MAX as u64,
            "changed",
            &target("changed"),
            true
        ),
        Err(OutboxError::Limit)
    ));
    assert!(matches!(
        AppOutbox::configure_in(
            &tx,
            &catalog,
            "owner",
            "automation",
            u64::MAX,
            "changed",
            &target("changed"),
            true
        ),
        Err(OutboxError::Invalid)
    ));
    assert!(matches!(
        AppOutbox::deactivate_in(
            &tx,
            &catalog,
            "owner",
            "automation",
            i64::MAX as u64,
            AutomationStatus::Disabled
        ),
        Err(OutboxError::Limit)
    ));
    let retained = AppOutbox::configuration_in(&tx, &catalog, "owner", "automation").unwrap();
    assert_eq!(retained.target.queue_id, "default");
    assert_eq!(retained.status, AutomationStatus::Active);
    assert!(!retained.scheduled);
    tx.commit().unwrap();
}
