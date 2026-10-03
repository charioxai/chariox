//! Opt-in, process-kill component drill. No kernel, root helper or provider is
//! launched here. See scripts/drill-linux-durable-faults.sh for bounded disks.
#![cfg(target_os = "linux")]
#[path = "durable_faults/checkpoints.rs"]
mod checkpoints;
#[path = "durable_faults/disk.rs"]
mod disk;
#[path = "durable_faults/fixture.rs"]
mod fixture;
use fixture::*;

#[test]
#[ignore = "explicit private scratch and ordinary UID required"]
fn process_kill_matrix() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    for point in [
        "raw_acknowledged",
        "download",
        "verify",
        "extract",
        "approval",
        "preparation",
        "switch_before_commit",
        "switch_after_commit",
        "state_before_commit",
        "state_after_commit",
        "snapshot_fence",
        "migration",
        "migration_before_commit",
        "migration_after_commit",
        "upload_archive_synced",
        "upload_state_renamed",
        "upload_state_committed",
        "extract_published",
        "extract_parent_synced",
    ] {
        let root = case(point);
        let mut db = open(&root);
        seed(&mut db);
        receipt(&root, "pre", &mut db);
        drop(db);
        kill_at(&root, point);
        checkpoints::recover(&root, point);
        cleanup(&root);
    }
}

#[test]
#[ignore = "child entrypoint; only the parent drill supplies its checkpoint"]
fn crash_child() {
    let root = std::path::PathBuf::from(std::env::var_os("CHARIOX_FAULT_CHILD_ROOT").unwrap());
    let point = std::env::var("CHARIOX_FAULT_POINT").unwrap();
    checkpoints::child(&root, &point);
    panic!("checkpoint was not reached: {point}");
}

#[test]
#[ignore = "bounded private filesystem must be mounted by the drill runner"]
fn enospc_without_restart() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    disk::run();
}

#[test]
#[ignore = "base comparison; accepted work must survive route removal"]
fn removed_route_keeps_accepted_occurrence() {
    let root = case("removed-route");
    let mut db = open(&root);
    seed(&mut db);
    let route = route();
    let tx = db.transaction().unwrap();
    let accepted = chariox_app_runtime::app_inbox::accept_in(
        &tx,
        &route,
        "retained",
        &serde_json::json!({"text":"saved"}),
        1,
        100,
    )
    .unwrap();
    tx.commit().unwrap();
    receipt(&root, "accepted_before_removal", &mut db);
    chariox_app_runtime::app_inbox::remove_route_in(&db, "owner", "installed", "route").unwrap();
    receipt(&root, "after_removal", &mut db);
    let due = chariox_app_runtime::app_inbox::due(&db, 100, 10).unwrap();
    let ok = due.len() == 1
        && accepted == chariox_app_runtime::app_inbox::Accepted::New(due[0].sequence);
    drop(db);
    cleanup(&root);
    assert!(ok, "accepted inbox occurrence was lost on route removal");
}
