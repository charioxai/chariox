use super::*;

#[tokio::test]
async fn unix_listener_is_private_and_refuses_symlink_or_regular_file() {
    let root = std::env::temp_dir().join(format!("chx-ipc-{:016x}", rand::random::<u64>()));
    let path = root.join("private/kernel.sock");
    let listener = LocalIpcListener::bind(path.clone()).unwrap();
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(LocalIpcListener::bind(path.clone()).is_err());
    drop(listener);
    assert!(!path.exists());
    fs::write(&path, "keep").unwrap();
    assert!(LocalIpcListener::bind(path.clone()).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(root.join("private"), root.join("link")).unwrap();
    assert!(LocalIpcListener::bind(root.join("link/kernel.sock")).is_err());
    fs::remove_file(root.join("link")).unwrap();
    fs::remove_dir_all(&root).unwrap();
}
