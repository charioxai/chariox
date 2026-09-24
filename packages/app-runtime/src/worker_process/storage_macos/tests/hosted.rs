//! Ignored by every ordinary test command. Real images are allowed only on the
//! dedicated disposable GitHub macOS runner, never on a developer workstation.
use super::*;
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn root() -> PathBuf {
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").as_deref(),
        Ok("github-hosted")
    );
    assert_eq!(
        std::env::var("GITHUB_REPOSITORY").as_deref(),
        Ok("charioxai/chariox")
    );
    let temp = fs::canonicalize(std::env::var_os("RUNNER_TEMP").unwrap()).unwrap();
    let supplied = PathBuf::from(std::env::var_os("CHARIOX_STORAGE_DRILL_ROOT").unwrap());
    let path = fs::canonicalize(&supplied).unwrap();
    assert_eq!(path, supplied);
    assert_eq!(path.parent(), Some(temp.as_path()));
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("chariox-app-storage."));
    path
}

fn prepare(root: &StorageRoot, generation: u64) -> MountedStorage {
    root.prepare_with_capacities(
        "fixture-owner",
        "fixture-app",
        generation,
        [64 * 1024 * 1024; 2],
    )
    .unwrap()
}

fn save(path: &Path, bytes: &[u8]) {
    let mut file = File::create(path).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn quota(path: &Path, capacity: u64) -> u64 {
    let mut file = File::create(path).unwrap();
    let block = vec![0x5a; 1024 * 1024];
    let mut full = false;
    for _ in 0..=capacity / block.len() as u64 {
        if let Err(error) = file.write_all(&block) {
            assert!(
                matches!(error.raw_os_error(), Some(libc::ENOSPC | libc::EDQUOT)),
                "{error}"
            );
            full = true;
            break;
        }
    }
    assert!(
        full,
        "fixed image must stop a write at its filesystem capacity"
    );
    let written = file.metadata().unwrap().len();
    assert!(written > 0 && written <= capacity);
    drop(file);
    fs::remove_file(path).unwrap();
    written
}

#[test]
#[ignore = "dedicated disposable hosted macOS runner only"]
fn hosted_storage_create_quota_restart_and_crash_recovery() {
    let path = root();
    let store = StorageRoot::open(&path).unwrap();
    let mut storage = prepare(&store, 1);
    assert!(matches!(
        store.prepare_with_capacities("fixture-owner", "fixture-app", 2, [64 * 1024 * 1024; 2]),
        Err(Error::Busy)
    ));
    let [data, tmp] = storage.paths();
    save(&data.join("persistent.txt"), b"persistent before restart");
    save(&tmp.join("temporary.txt"), b"discard after restart");
    fs::copy("/usr/bin/true", data.join("noexec-probe")).unwrap();
    fs::set_permissions(data.join("noexec-probe"), fs::Permissions::from_mode(0o700)).unwrap();
    match Command::new(data.join("noexec-probe"))
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Err(error) => assert_eq!(error.raw_os_error(), Some(libc::EACCES)),
        Ok(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("private image allowed executable launch");
        }
    }
    let filled = [
        quota(&data.join("fill.bin"), 64 * 1024 * 1024),
        quota(&tmp.join("fill.bin"), 64 * 1024 * 1024),
    ];
    let original_uuid = storage.journal.images[0].volume_uuid.clone();
    let metadata = commands::plist(
        commands::Tool::Hdiutil,
        &commands::strings(&["info", "-plist"]),
        storage.command_context(),
    )
    .unwrap();
    let selected: Vec<_> = metadata["images"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry["image-path"]
                .as_str()
                .is_some_and(|p| Path::new(p).parent() == Some(storage.path.as_path()))
        })
        .cloned()
        .collect();
    assert_eq!(selected.len(), 2);
    let observation = json!({
        "scope":"Actual fixed APFS storage only; no App, Node or signed worker execution",
        "reservedBytes":storage.reserved_bytes(), "bytesBeforeNoSpace":filled, "appleImageMetadata":selected,
        "images":storage.journal.images.iter().enumerate().map(|(index, image)| json!({
            "role":image.role,"capacity":image.capacity,"uuid":image.volume_uuid,
            "fileBytes":storage.images[index].as_ref().unwrap().metadata().unwrap().len(),
            "allocatedBytes":storage.images[index].as_ref().unwrap().metadata().unwrap().blocks()*512,
        })).collect::<Vec<_>>()
    });
    println!("{observation}");
    storage.release_blocking().unwrap();
    drop(storage);
    let mut storage = prepare(&store, 2);
    assert_eq!(storage.journal.images[0].volume_uuid, original_uuid);
    assert_eq!(
        fs::read(storage.paths()[0].join("persistent.txt")).unwrap(),
        b"persistent before restart"
    );
    assert!(!storage.paths()[1].join("temporary.txt").exists());
    storage.release_blocking().unwrap();
    drop(storage);

    // A separate trusted test process exits without Rust Drop after real mount
    // and fsync. Kernel startup then uses the same production journal recovery.
    let log_name = path.file_name().unwrap().to_str().unwrap();
    let log = path.with_file_name(format!("{log_name}.child.log"));
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "worker_process::storage_macos::tests::hosted::hosted_storage_crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(File::create(&log).unwrap())
        .stderr(File::create(path.with_file_name(format!("{log_name}.child-stderr.log"))).unwrap())
        .process_group(0);
    for key in [
        "GITHUB_ACTIONS",
        "RUNNER_ENVIRONMENT",
        "GITHUB_REPOSITORY",
        "RUNNER_TEMP",
        "CHARIOX_STORAGE_DRILL_ROOT",
    ] {
        command.env(key, std::env::var_os(key).unwrap());
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(150);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            // The leader is still owned/unreaped; no reused PID is signalled.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            panic!("hosted crash fixture deadline; dedicated recovery step follows");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
    store.recover_all_blocking().unwrap();
    let mut recovered = prepare(&store, 4);
    assert_eq!(
        fs::read(recovered.paths()[0].join("crash-saved.txt")).unwrap(),
        b"fsync before process exit"
    );
    assert_eq!(recovered.journal.images[0].volume_uuid, original_uuid);
    recovered.release_blocking().unwrap();
    drop(recovered);
    store.recover_all_blocking().unwrap();
    println!("actual_storage_create_quota_noexec_restart_and_process_crash_recovery_passed");
}

#[test]
#[ignore = "called only by the dedicated hosted storage test"]
fn hosted_storage_crash_child() {
    let store = StorageRoot::open(&root()).unwrap();
    let storage = prepare(&store, 3);
    save(
        &storage.paths()[0].join("crash-saved.txt"),
        b"fsync before process exit",
    );
    // Deliberately skips the production Drop/detach path; no App is running.
    std::process::exit(0);
}

#[test]
#[ignore = "dedicated hosted cleanup; never force-detaches or deletes an image"]
fn hosted_storage_cleanup() {
    StorageRoot::open(&root())
        .unwrap()
        .recover_all_blocking()
        .unwrap();
}
