//! Signed tiny bytes exercise preparation and mounts; they are never executed.
use super::*;
use crate::{
    installation::{InstallationRegistry, StageTrustBinding, VerifiedInstallCandidate},
    publisher_trust::{PublisherTrustRegistry, TrustDecision},
    release_store::{ReleaseStore, StageBudget, VerifiedReleaseLease},
    runtime_enrollment::EnrolledRuntime,
    worker_process::PreparedWorker,
};
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

pub(super) fn proofs() -> (VerifiedReleaseLease, EnrolledRuntime, StageTrustBinding) {
    let key = SigningKey::from_bytes(&[27; 32]);
    let publisher = TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "fixture".into(),
        public_key: key.verifying_key(),
    };
    let manifest:Manifest=serde_json::from_value(json!({
        "schema":"chariox.app.v1","appId":"com.example.hosted","version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"fixture","name":"Hosted fixture"},
        "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"appContractVersion":1,"minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"},"capabilities":{}
    })).unwrap();
    let payload = BTreeMap::from([
        (
            "runtime/main.js".into(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>Fixture</title>".to_vec(),
        ),
    ]);
    let bytes = pack(&manifest, &payload, &key, &Limits::default()).unwrap();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(500, vec![publisher.clone()]),
    )
    .unwrap();
    let state = Path::new("/var/lib/chariox/home/state");
    fs::create_dir_all(state).unwrap();
    fs::set_permissions(state, fs::Permissions::from_mode(0o700)).unwrap();
    let store = ReleaseStore::open_or_create(&state.join("kernel.db")).unwrap();
    store
        .stage(
            &package,
            &bytes,
            StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 8 * 1024 * 1024 * 1024,
            },
        )
        .unwrap();
    let lease = store.lease_verified(&package, &bytes).unwrap();
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    let mut trust = PublisherTrustRegistry::new(&mut connection);
    trust.initialize().unwrap();
    trust
        .enroll(
            "hosted-owner",
            &publisher,
            0,
            &TrustDecision {
                decision_id: "hosted-explicit-fixture".into(),
                authority_ref: "dedicated-root-drill".into(),
            },
            1,
        )
        .unwrap();
    let snapshot = trust
        .trusted_publisher("hosted-owner", "com.example", "fixture")
        .unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&package, &snapshot).unwrap();
    let mut installs = InstallationRegistry::new(&mut connection);
    installs.initialize().unwrap();
    let update = installs
        .create_and_stage_verified("factory-only", "hosted-owner", &candidate, 2)
        .unwrap();
    let binding = installs
        .staged_trust("hosted-owner", &update.token)
        .unwrap();
    (lease, EnrolledRuntime::open_installed().unwrap(), binding)
}

#[test]
#[ignore = "dedicated hosted installed helper and signed tiny graph; no native execution"]
fn hosted_readonly_code_views_match_verified_roots_in_kernel_namespace() {
    let context = Context::open("44444444444444444444444444444444");
    let (package, runtime, _binding) = proofs();
    let mut lease = context.lease("code-views", 1);
    lease.attach_code(&package, &runtime).unwrap();
    let [package_path, runtime_path] = lease.code_paths();
    let entry = package_path.join("runtime/main.js");
    assert_eq!(
        fs::metadata(&package_path).unwrap().ino(),
        package.payload().metadata().unwrap().ino()
    );
    assert_eq!(
        fs::metadata(&runtime_path).unwrap().ino(),
        runtime.root().metadata().unwrap().ino()
    );
    assert_eq!(
        fs::read(&entry).unwrap(),
        b"export default function register() {}"
    );
    assert_eq!(
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o600))
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EROFS)
    );
    assert!(fs::read(runtime_path.join("bootstrap.cjs"))
        .unwrap()
        .starts_with(b"signed fixture hosted-storage-views-only"));
    for (view, expected_noexec) in [
        (lease.package.as_ref().unwrap(), true),
        (lease.runtime.as_ref().unwrap(), false),
    ] {
        let flags = super::super::files::mount_flags(&view.0).unwrap();
        assert_eq!(
            flags & (libc::ST_RDONLY | libc::ST_NODEV | libc::ST_NOSUID),
            libc::ST_RDONLY | libc::ST_NODEV | libc::ST_NOSUID
        );
        assert_eq!(flags & libc::ST_NOEXEC != 0, expected_noexec);
        println!("Propagated code view flags={flags}, noexec={expected_noexec}, exact verified device/inode matched");
    }
    lease.release().unwrap();
    let mut next = context.lease("code-views", 2);
    next.attach_code(&package, &runtime).unwrap();
    next.release().unwrap();
}

#[test]
#[ignore = "dedicated hosted factory preparation with signed tiny graph; never spawns it"]
fn hosted_prepared_worker_uses_only_enrolled_sources_and_reclaims_unstarted_domain() {
    let context = Context::open("55555555555555555555555555555555");
    let (package, runtime, binding) = proofs();
    let prepared = PreparedWorker::prepare_linux(runtime, package, &binding).unwrap();
    assert_eq!(prepared.record.installation, "factory-only");
    assert_eq!(prepared.record.generation, "1");
    assert_eq!(
        prepared.record.roots,
        ["/app/package", "/app/data", "/app/tmp", "/runtime"]
    );
    assert!(prepared
        .record
        .bootstrap
        .contains("createRequire('/runtime/bootstrap.cjs')"));
    let program = PathBuf::from(prepared.program.to_str().unwrap());
    assert!(program.starts_with("/var/lib/chariox-app-storage"));
    assert_eq!(program.file_name().unwrap(), "chariox-app-domain-entry");
    let cgroup_root = context.cgroup.parent().unwrap();
    let prepared_leaves: Vec<_> = fs::read_dir(cgroup_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir() && path != &context.cgroup)
        .collect();
    assert_eq!(prepared_leaves.len(), 1);
    drop(prepared);
    assert!(!prepared_leaves[0].exists());
    println!("Actual production factory prepared exact signed roots, platform graph and cgroup, then released without executing fixture code");
}
