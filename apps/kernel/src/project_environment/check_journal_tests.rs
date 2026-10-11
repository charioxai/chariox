//! MP-08 / MP-10 / MP-11: Check acknowledgement journal durability and recovery.
use super::*;
#[test]
fn deleting_a_project_removes_only_its_private_check_journal() {
    let root = crate::test_support::TestWorktree::new("envp04-check-journal-delete");
    let store = ProjectEnvironmentStore::new(root.path());
    store
        .save_checks("deleted", &EnvironmentChecks::default())
        .unwrap();
    store
        .save_checks("retained", &EnvironmentChecks::default())
        .unwrap();
    let deleted = store.path("deleted").with_extension("checks.wal");
    let retained = store.path("retained").with_extension("checks.wal");
    assert!(deleted.exists() && retained.exists());
    store.remove("deleted").unwrap();
    assert!(
        !deleted.exists(),
        "Project deletion must remove its private observation journal"
    );
    assert!(
        retained.exists(),
        "other Projects' journals must survive deletion"
    );
    store.load_checks("retained").unwrap();
}
#[test]
fn check_projection_waits_for_the_durable_writer() {
    use std::sync::mpsc;
    let root = crate::test_support::TestWorktree::new("envp04-check-read-fence");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = crate::session::RuntimeProject::new(
        "check",
        "owner",
        root.path().to_str().unwrap(),
        "Check",
        crate::session::RuntimeProjectKind::Named,
    );
    let mut env = store.snapshot(&project).unwrap();
    let held = store.check_journal_lock(project.id()).unwrap();
    let reader = store.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let task = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = reader.attach_checks(
            &mut env,
            &EnvironmentTargetBinding {
                machine_id: "machine".into(),
                target_instance_generation: "generation".into(),
                slice_ref: None,
            },
            1,
        );
        done_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    let early = done_rx.recv_timeout(std::time::Duration::from_millis(50));
    drop(held);
    task.join().unwrap();
    assert!(
        matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
        "public projection must wait for a writer's durable fence"
    );
    done_rx.recv().unwrap().unwrap();
}
#[test]
fn check_journal_updates_do_not_grow_the_durable_inode() {
    let root = crate::test_support::TestWorktree::new("envp04-check-fixed-size");
    let store = ProjectEnvironmentStore::new(root.path());
    let mut checks = EnvironmentChecks::default();
    checks.folder_binding_digest = "first".into();
    store.save_checks("project", &checks).unwrap();
    let file = store.path("project").with_extension("checks.wal");
    let before = std::fs::metadata(&file).unwrap().len();
    checks.folder_binding_digest = "second".into();
    store.save_checks("project", &checks).unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().len(), before);
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "second"
    );
}
#[test]
fn check_journal_retains_last_complete_durable_snapshot() {
    use std::io::{Seek, SeekFrom, Write};
    let root = crate::test_support::TestWorktree::new("envp04-check-journal");
    let store = ProjectEnvironmentStore::new(root.path());
    let mut checks = EnvironmentChecks::default();
    checks.folder_binding_digest = "first".into();
    store.save_checks("project", &checks).unwrap();
    checks.folder_binding_digest = "second".into();
    store.save_checks("project", &checks).unwrap();
    let path = store.path("project").with_extension("checks.wal");
    let image = std::fs::read(&path).unwrap();
    let capacity = u64::from_le_bytes(image[8..16].try_into().unwrap());
    // Tear the newest slot's payload. The previously synced slot survives.
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(4096 + 4096 + capacity + 4096))
        .unwrap();
    file.write_all(b"!").unwrap();
    file.sync_data().unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "first"
    );
    checks.folder_binding_digest = "third".into();
    store.save_checks("project", &checks).unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "third"
    );
    // A complete checksummed record with an invalid schema is never silently
    // replaced by an older valid state.
    super::check_journal::append(&path, b"{}").unwrap();
    assert!(store.load_checks("project").is_err());
}

#[test]
fn check_journal_grows_atomically_and_imports_append_snapshots() {
    let root = crate::test_support::TestWorktree::new("envp04-check-resize");
    let store = ProjectEnvironmentStore::new(root.path());
    let path = store.path("project").with_extension("checks.wal");
    let mut checks = EnvironmentChecks::default();
    checks.folder_binding_digest = "legacy append".into();
    let mut legacy = serde_json::to_vec(&checks).unwrap();
    legacy.push(b'\n');
    crate::config::write_private_file(&path, &legacy).unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "legacy append"
    );
    store.save_checks("project", &checks).unwrap();
    let before = std::fs::metadata(&path).unwrap().len();
    checks.folder_binding_digest = "a".repeat(70_000);
    store.save_checks("project", &checks).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > before);
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        checks.folder_binding_digest
    );
    let grown = std::fs::metadata(&path).unwrap().len();
    checks.folder_binding_digest.push('b');
    store.save_checks("project", &checks).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), grown);
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        checks.folder_binding_digest
    );
}
#[test]
fn check_journal_imports_legacy_snapshot_and_rejects_symlinks() {
    let root = crate::test_support::TestWorktree::new("envp04-check-journal");
    let store = ProjectEnvironmentStore::new(root.path());
    let mut checks = EnvironmentChecks::default();
    checks.folder_binding_digest = "legacy".into();
    let legacy = store.path("project").with_extension("checks.json");
    crate::config::write_private_file(&legacy, &serde_json::to_vec(&checks).unwrap()).unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "legacy"
    );
    store.save_checks("project", &checks).unwrap();
    std::fs::remove_file(legacy).unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "legacy"
    );
    #[cfg(unix)]
    {
        let wal = store.path("project").with_extension("checks.wal");
        std::fs::remove_file(&wal).unwrap();
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"unchanged").unwrap();
        std::os::unix::fs::symlink(&outside, &wal).unwrap();
        assert!(store.save_checks("project", &checks).is_err());
        assert!(store.load_checks("project").is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"unchanged");
        std::fs::remove_file(&wal).unwrap();
        std::fs::hard_link(&outside, &wal).unwrap();
        assert!(store.save_checks("project", &checks).is_err());
        assert!(store.load_checks("project").is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"unchanged");
    }
}
