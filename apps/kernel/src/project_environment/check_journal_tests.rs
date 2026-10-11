//! MP-08 / MP-10 / MP-11: Check acknowledgement journal durability and recovery.
use super::*;
#[test]
fn check_journal_retains_last_complete_durable_snapshot() {
    use std::io::Write;
    let root = crate::test_support::TestWorktree::new("envp04-check-journal");
    let store = ProjectEnvironmentStore::new(root.path());
    let mut checks = EnvironmentChecks::default();
    checks.folder_binding_digest = "first".into();
    store.save_checks("project", &checks).unwrap();
    checks.folder_binding_digest = "second".into();
    store.save_checks("project", &checks).unwrap();
    let file = store.path("project").with_extension("checks.wal");
    let before = std::fs::read(&file).unwrap();
    assert_eq!(before.iter().filter(|b| **b == b'\n').count(), 2);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"{\"operations\":")
        .unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "second"
    );
    checks.folder_binding_digest = "third".into();
    store.save_checks("project", &checks).unwrap();
    assert_eq!(
        store.load_checks("project").unwrap().folder_binding_digest,
        "third"
    );
    let recovered = std::fs::read(&file).unwrap();
    assert_eq!(recovered.iter().filter(|b| **b == b'\n').count(), 3);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"{}\n")
        .unwrap();
    assert!(
        store.load_checks("project").is_err(),
        "corrupt complete records must fail loudly"
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
