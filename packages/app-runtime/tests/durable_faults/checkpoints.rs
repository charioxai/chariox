use super::fixture::*;
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    app_inbox::{Accepted, InboxState},
    package_upload::{PackageUploadStore, UploadCheckpoint, UploadLimits},
    release_store::StageCheckpoint,
};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn upload(root: &Path) -> PackageUploadStore {
    PackageUploadStore::open(&root.join("uploads"), UploadLimits::default(), 1).unwrap()
}

pub fn child(root: &Path, point: &str) {
    if point == "raw_acknowledged" {
        let mut file = fs::File::create(root.join("raw-acknowledged.bin")).unwrap();
        file.write_all(&[0x95; 65536]).unwrap();
        file.sync_all().unwrap();
        fs::File::open(root).unwrap().sync_all().unwrap();
        stop(
            root,
            json!({"bytes":65536,"sha256":digest(&[0x95;65536]),"file_sync":true,"directory_sync":true}),
        );
    }
    if point.starts_with("state_") {
        state_child(root, point);
    }
    let package = Package::new(
        2,
        if point.starts_with("migration") || point == "snapshot_fence" {
            1
        } else {
            0
        },
    );
    if point == "download" || point.starts_with("upload_") {
        fs::create_dir(root.join("uploads")).unwrap();
        fs::set_permissions(
            root.join("uploads"),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let store = upload(root);
        let status = store
            .begin(
                "owner",
                "download",
                package.bytes.len() as u64,
                &digest(&package.bytes),
                1000,
                1,
            )
            .unwrap();
        let half = package.bytes.len() / 2;
        let receipt = store
            .chunk(
                "owner",
                &status.handle,
                0,
                &package.bytes[..half],
                &digest(&package.bytes[..half]),
                2,
            )
            .unwrap();
        if point == "download" {
            stop(
                root,
                json!({"handle":receipt.handle,"accepted_bytes":receipt.accepted_bytes,"sha256":receipt.sha256}),
            );
        }
        let at = match point {
            "upload_archive_synced" => UploadCheckpoint::ArchiveSynced,
            "upload_state_renamed" => UploadCheckpoint::StateRenamed,
            "upload_state_committed" => UploadCheckpoint::StateCommitted,
            _ => unreachable!(),
        };
        store.chunk_with_checkpoint("owner",&status.handle,half as u64,&package.bytes[half..],&digest(&package.bytes[half..]),3,|checkpoint| {
            if checkpoint == at { stop(root,json!({"handle":status.handle,"accepted_bytes":half,"archive_sha256":digest(&package.bytes),"checkpoint":format!("{checkpoint:?}")})); }
            Ok(())
        }).unwrap();
    }
    verify(
        &package.bytes,
        &VerificationPolicy::new(367, vec![package.publisher.clone()]),
    )
    .unwrap();
    if point == "verify" {
        stop(root, json!({"verified_digest":digest(&package.bytes)}));
    }
    package.extract(root, |checkpoint| {
        if (point == "extract" && checkpoint == StageCheckpoint::BeforePublish) || (point == "extract_published" && checkpoint == StageCheckpoint::Published) || (point == "extract_parent_synced" && checkpoint == StageCheckpoint::ParentSynced) {
            stop(root,json!({"checkpoint":format!("{checkpoint:?}"),"verified_digest":digest(&package.bytes)}));
        }
        Ok(())
    });
    let mut db = open(root);
    let trust = trust(&mut db);
    let candidate = package.candidate(&mut db);
    let tx = db.transaction().unwrap();
    let token = candidate
        .stage_update_in(&tx, "owner", "installed", 1, 10)
        .unwrap()
        .token;
    tx.commit().unwrap();
    if point == "approval" {
        stop(root, json!({"token":token,"decision":"pending"}));
    }
    let tx = db.transaction().unwrap();
    let binding = StageTrustBinding::staged_in(&tx, "owner", &token).unwrap();
    binding
        .decide_in(
            &tx,
            "owner",
            &trust,
            CapabilityDecision::Approved {
                approval: approval(),
            },
            11,
        )
        .unwrap();
    binding.quiesce_in(&tx, "owner", &trust, 12).unwrap();
    tx.commit().unwrap();
    if point == "preparation" || point == "snapshot_fence" {
        stop(
            root,
            json!({"token":token,"phase":"quiescing","health":"not_executed"}),
        );
    }
    if point.starts_with("migration") {
        let mut tx = db.transaction().unwrap();
        ManagedStateStore::apply_in(&mut tx, scope(token.generation), &changes(1, "migrated"))
            .unwrap();
        ManagedStateStore::migration_step_in(&tx, scope(token.generation), 1).unwrap();
        tx.commit().unwrap();
        if point == "migration" {
            stop(root, json!({"token":token,"migration_step":1}));
        }
    }
    let tx = db.transaction().unwrap();
    binding
        .commit_in(&tx, "owner", &trust, &approval(), 13)
        .unwrap();
    if point.ends_with("before_commit") {
        stop(root, json!({"token":token,"commit":"provisional"}));
    }
    tx.commit().unwrap();
    stop(root, json!({"token":token,"commit":"acknowledged"}));
}
fn state_child(root: &Path, point: &str) -> ! {
    let mut db = open(root);
    let catalog = Package::new(1, 0).catalog(&mut db);
    let mut tx = db.transaction().unwrap();
    let revision = ManagedStateStore::apply_in(&mut tx, scope(1), &changes(0, "new")).unwrap();
    let outbox =
        AppOutbox::apply_current_in(&mut tx, catalog, "owner", &[occurrence()], 100).unwrap();
    let inbox =
        app_inbox::accept_in(&tx, &route(), "b5-incoming", &json!({"text":"new"}), 1, 100).unwrap();
    ManagedStateStore::apply_wakes_in(
        &mut tx,
        scope(1),
        &[WakeChange::Set(Wake {
            id: "due".into(),
            due_at_ms: 100,
            revision: "revision-1".into(),
        })],
        false,
    )
    .unwrap();
    let checkpoint = json!({"revision":revision,"outbox":format!("{outbox:?}"),"inbox":format!("{inbox:?}"),"wake":{"id":"due","revision":"revision-1"},"acknowledged":point.ends_with("after_commit")});
    if point.ends_with("before_commit") {
        stop(root, checkpoint);
    }
    tx.commit().unwrap();
    stop(root, checkpoint)
}

pub fn recover(root: &Path, point: &str) {
    if point == "raw_acknowledged" {
        let bytes = fs::read(root.join("raw-acknowledged.bin")).unwrap();
        assert_eq!(bytes, vec![0x95; 65536]);
        println!(
            "{}",
            json!({"case":point,"phase":"recovered","bytes":bytes.len(),"sha256":digest(&bytes)})
        );
        return;
    }
    if point == "download" || point.starts_with("upload_") {
        let marker: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("checkpoint.json")).unwrap()).unwrap();
        let package = Package::new(2, 0);
        let handle = marker["handle"].as_str().unwrap();
        let store = upload(root);
        let status = store.status("owner", handle, 4).unwrap();
        let half = package.bytes.len() / 2;
        let expected = if matches!(point, "upload_state_renamed" | "upload_state_committed") {
            package.bytes.len()
        } else {
            half
        };
        assert_eq!(status.accepted_bytes, expected as u64);
        let recovered = fs::read(root.join("uploads").join(format!("{handle}.cxapp"))).unwrap();
        assert_eq!(recovered, &package.bytes[..expected]);
        // Replay the same suffix (whether already acknowledged or interrupted).
        store
            .chunk(
                "owner",
                handle,
                half as u64,
                &package.bytes[half..],
                &digest(&package.bytes[half..]),
                5,
            )
            .unwrap();
        let lease = store.finalize("owner", handle, 6).unwrap();
        let mut contents = Vec::new();
        lease
            .file()
            .try_clone()
            .unwrap()
            .read_to_end(&mut contents)
            .unwrap();
        assert_eq!(contents, package.bytes);
        drop(lease);
        store.abort("owner", handle, 7).unwrap();
        println!(
            "{}",
            json!({"case":point,"phase":"recovered","accepted_bytes":status.accepted_bytes,"exact_prefix":true,"final_digest":digest(&package.bytes)})
        );
        return;
    }
    let mut db = open(root);
    let committed = matches!(point, "switch_after_commit" | "migration_after_commit");
    let installation = InstallationRegistry::new(&mut db).get("installed").unwrap();
    assert_eq!(
        installation.active.as_ref().unwrap().generation,
        if committed { 2 } else { 1 }
    );
    if (point == "snapshot_fence"
        || point.starts_with("migration")
        || point.starts_with("switch_")
        || point == "preparation")
        && !committed
    {
        assert!(ManagedStateStore::new(&mut db)
            .transaction(scope(1), &changes(0, "fenced"))
            .is_err());
        assert_ne!(
            db.query_row(
                "SELECT value_json FROM app_state_values WHERE key='saved'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            json!({"text":"fenced"}).to_string()
        );
    }
    // Supervisor recovery explicitly aborts its pending operation; the registry
    // cannot attest process health or choose the kernel's recovery policy.
    if let Some(pending) = installation.pending_generation {
        let token = InstallationRegistry::new(&mut db)
            .journal("installed")
            .unwrap()
            .into_iter()
            .find(|row| row.token.generation == pending)
            .unwrap()
            .token;
        let tx = db.transaction().unwrap();
        let binding = StageTrustBinding::staged_in(&tx, "owner", &token).unwrap();
        binding
            .abort_in(&tx, "owner", "drill_process_lost", 20)
            .unwrap();
        tx.commit().unwrap();
        let tx = db.transaction().unwrap();
        assert!(binding
            .commit_in(&tx, "owner", &trust(&mut open(root)), &approval(), 21)
            .is_err());
    }
    let recovered = InstallationRegistry::new(&mut db).get("installed").unwrap();
    assert!(recovered.pending_generation.is_none() && !recovered.admission_paused);
    let expected = if point == "state_after_commit" {
        "new"
    } else if point == "migration_after_commit" {
        "migrated"
    } else {
        "acknowledged"
    };
    assert_eq!(
        ManagedStateStore::new(&mut db)
            .get(
                scope(recovered.active.as_ref().unwrap().generation),
                "saved"
            )
            .unwrap()
            .unwrap()
            .value,
        json!({"text":expected})
    );
    if point.starts_with("state_") {
        event_recovery(root, &mut db, point == "state_after_commit");
    }
    let snapshots: u32 = db
        .query_row(
            "SELECT count(*) FROM app_state_snapshot_values",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshots, 0);
    if point.starts_with("extract") {
        let store = ReleaseStore::open_or_create(&root.join("kernel.sqlite")).unwrap();
        assert_eq!(store.collect_abandoned().unwrap().active, 0);
        Package::new(2, 0).extract(root, |_| Ok(()));
        let report = store.collect_abandoned().unwrap();
        assert_eq!(report.removed, 0);
        assert_eq!(report.active, 0);
    }
    receipt(root, "recovered", &mut db);
}
fn event_recovery(root: &Path, db: &mut Connection, committed: bool) {
    let due = app_inbox::due(db, 100, 10).unwrap();
    let catalog = Package::new(1, 0).catalog(db);
    let tx = db.transaction().unwrap();
    let outgoing = AppOutbox::pending_in(&tx, &catalog, "owner", 100, 10).unwrap();
    assert_eq!(outgoing.len(), usize::from(committed));
    assert_eq!(due.len(), usize::from(committed));
    assert_eq!(
        managed_state::due_wakes(&tx, 100, 10).unwrap().len(),
        usize::from(committed)
    );
    tx.commit().unwrap();
    if !committed {
        return;
    }
    let marker: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("checkpoint.json")).unwrap()).unwrap();
    assert_eq!(format!("{outgoing:?}"), marker["outbox"].as_str().unwrap());
    let mut tx = db.transaction().unwrap();
    let retry =
        AppOutbox::apply_current_in(&mut tx, catalog, "owner", &[occurrence()], 101).unwrap();
    assert_eq!(retry, outgoing);
    assert_eq!(
        app_inbox::accept_in(&tx, &route(), "b5-incoming", &json!({"text":"new"}), 1, 101).unwrap(),
        Accepted::Duplicate(due[0].sequence)
    );
    managed_state::complete_wake(&tx, &managed_state::due_wakes(&tx, 100, 10).unwrap()[0]).unwrap();
    let mut now = 100;
    for attempt in 1..=app_inbox::MAX_ATTEMPTS {
        let state = app_inbox::failed_attempt_in(&tx, due[0].sequence, now).unwrap();
        assert_eq!(
            state,
            if attempt == app_inbox::MAX_ATTEMPTS {
                InboxState::Failed
            } else {
                InboxState::Retryable
            }
        );
        now += 60_000;
    }
    tx.commit().unwrap();
    assert!(app_inbox::due(db, now, 10).unwrap().is_empty());
    assert_eq!(app_inbox::counts(db, &route()).unwrap().failed, 1);
    println!(
        "{}",
        json!({"case":"state_after_commit","phase":"handoff","same_outbox_receipt":true,"inbox_duplicate":true,"poison":"failed","owner_query_failed_count":1,"notice_ui":"not_exercised"})
    );
}
