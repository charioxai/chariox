//! Real ENOSPC, not an injected generic I/O error. The caller supplies a
//! disposable <=128 MiB mount; never exhaust the builder's shared filesystem.
use super::fixture::*;
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    package_upload::{PackageUploadStore, UploadError, UploadLimits},
    release_store::ReleaseStoreError,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::ffi::OsStrExt,
};

fn fill(root: &Path) -> u64 {
    let mut ballast = fs::File::create(root.join("browser-cache-ballast")).unwrap();
    let mut written = 0;
    let bytes = [0x73; 65536];
    loop {
        match ballast.write(&bytes) {
            Ok(0) => panic!("zero-length ballast write"),
            Ok(n) => written += n as u64,
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
                break;
            }
        }
        assert!(written <= 128 * 1024 * 1024);
    }
    written
}
fn release(root: &Path) {
    fs::remove_file(root.join("browser-cache-ballast")).unwrap();
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn run() {
    let root = case("enospc");
    let name = std::ffi::CString::new(root.as_os_str().as_bytes()).unwrap();
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    assert_eq!(
        unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) },
        0
    );
    let stat = unsafe { stat.assume_init() };
    let capacity = stat.f_blocks as u64 * stat.f_frsize as u64;
    assert!(
        capacity <= 128 * 1024 * 1024,
        "refuse to fill any unbounded filesystem"
    );
    let mut db = open(&root);
    seed(&mut db);
    fs::write(root.join("neighbour-data"), b"neighbour acknowledged bytes").unwrap();
    // Pre-create an upload, so failure exercises archive/metadata writes rather
    // than merely refusing to create a directory on a full filesystem.
    let upload_root = root.join("uploads");
    fs::create_dir(&upload_root).unwrap();
    fs::set_permissions(
        &upload_root,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let uploads = PackageUploadStore::open(&upload_root, UploadLimits::default(), 1).unwrap();
    let bytes = vec![0x51; 512 * 1024];
    let status = uploads
        .begin(
            "owner",
            "disk-test",
            bytes.len() as u64,
            &digest(&bytes),
            1000,
            1,
        )
        .unwrap();
    let store = ReleaseStore::open_or_create(&root.join("kernel.sqlite")).unwrap();
    let package = Package::new(2, 0);
    let verified = verify(
        &package.bytes,
        &VerificationPolicy::new(367, vec![package.publisher.clone()]),
    )
    .unwrap();
    let refusal = StageBudget {
        host_reserve_bytes: capacity * 2,
        ..budget()
    };
    assert!(matches!(
        store.stage(&verified, &package.bytes, refusal),
        Err(ReleaseStoreError::HostReserve)
    ));
    let written = fill(&root);
    assert_eq!(
        ManagedStateStore::new(&mut db)
            .get(scope(1), "saved")
            .unwrap()
            .unwrap()
            .value,
        json!({"text":"acknowledged"})
    );
    assert_eq!(
        fs::read(root.join("neighbour-data")).unwrap(),
        b"neighbour acknowledged bytes"
    );
    let error = uploads
        .chunk("owner", &status.handle, 0, &bytes, &digest(&bytes), 2)
        .unwrap_err();
    assert!(matches!(&error,UploadError::Io(error) if error.raw_os_error()==Some(libc::ENOSPC)));
    println!(
        "{}",
        json!({"case":"enospc-upload","capacity":capacity,"browser_cache_ballast":written,"write_error":error.to_string(),"errno":libc::ENOSPC,"reads_usable":true,"typed_retryable":"raw_upload_io_on_this_base"})
    );
    release(&root);
    // Same store, connection and process: owned ballast removal is sufficient.
    let retry = uploads
        .chunk("owner", &status.handle, 0, &bytes, &digest(&bytes), 3)
        .unwrap();
    assert_eq!(retry.accepted_bytes, bytes.len() as u64);
    let lease = uploads.finalize("owner", &status.handle, 4).unwrap();
    let mut result = Vec::new();
    lease
        .file()
        .try_clone()
        .unwrap()
        .read_to_end(&mut result)
        .unwrap();
    assert_eq!(result, bytes);
    drop(lease);
    uploads.abort("owner", &status.handle, 5).unwrap();
    // Force SQLITE_FULL on a large managed-state transaction; small writes may
    // fit an existing WAL page and would not be a reliable disk-full boundary.
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    fill(&root);
    let large = StateChanges::new(
        0,
        vec![],
        vec![StateWrite::Put {
            key: "large".into(),
            value: json!("x".repeat(240 * 1024)),
        }],
    )
    .unwrap();
    let error = ManagedStateStore::new(&mut db)
        .transaction(scope(1), &large)
        .unwrap_err();
    assert!(
        matches!(&error,managed_state::StateError::Database(rusqlite::Error::SqliteFailure(code,_)) if code.code==rusqlite::ErrorCode::DiskFull)
    );
    assert_eq!(
        ManagedStateStore::new(&mut db)
            .get(scope(1), "saved")
            .unwrap()
            .unwrap()
            .value,
        json!({"text":"acknowledged"})
    );
    assert!(ManagedStateStore::new(&mut db)
        .get(scope(1), "large")
        .unwrap()
        .is_none());
    release(&root);
    ManagedStateStore::new(&mut db)
        .transaction(scope(1), &large)
        .unwrap();
    // Real extraction ENOSPC cleans its stage and releases the publication lock.
    let error = store
        .stage_with_checkpoint(&verified, &package.bytes, budget(), |checkpoint| {
            if checkpoint == chariox_app_runtime::release_store::StageCheckpoint::Created {
                fill(&root);
            }
            Ok(())
        })
        .unwrap_err();
    println!(
        "{}",
        json!({"case":"enospc-staging","error":error.to_string(),"reads_usable":true})
    );
    assert!(matches!(
        error,
        ReleaseStoreError::Io(ref error) if error.raw_os_error()==Some(libc::ENOSPC)
    ));
    release(&root);
    store.collect_abandoned().unwrap();
    store.stage(&verified, &package.bytes, budget()).unwrap();
    assert_eq!(store.collect_abandoned().unwrap().active, 0);
    assert_eq!(
        fs::read(root.join("neighbour-data")).unwrap(),
        b"neighbour acknowledged bytes"
    );
    println!(
        "{}",
        json!({"case":"enospc-recovery","same_process":true,"same_database_connection":true,"same_upload_store":true,"state_retry":true,"staging_retry":true,"neighbour_intact":true})
    );
    drop(uploads);
    drop(store);
    drop(db);
    cleanup(&root);
}
