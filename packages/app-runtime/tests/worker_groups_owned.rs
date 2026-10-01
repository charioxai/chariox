//! Privileged execution requires an explicitly owned private helper namespace.
//! This does not enroll a production release or grant authority to an App.
#![cfg(target_os = "linux")]
use chariox_app_package::{verify, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, VerifiedInstallCandidate,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision},
    release_store::{ReleaseStore, StageBudget},
    runtime_enrollment::EnrolledRuntime,
    worker_process::{PreparedWorker, WorkerLimits, WorkerProcess},
};
use ed25519_dalek::VerifyingKey;
use std::{fs, path::PathBuf, time::Duration};

#[test]
#[ignore = "owned private helper namespace and retained DEV inventory required"]
fn owned_multigroup_launch_keeps_app_authority_empty() {
    assert_eq!(unsafe { libc::geteuid() }, 981);
    let root = PathBuf::from(std::env::var("CHARIOX_GROUPS_OWNED_ROOT").unwrap());
    assert_eq!(
        root.parent().unwrap(),
        std::path::Path::new("/var/lib/p1lk/9/worker-groups-")
    );
    assert_eq!(fs::canonicalize(&root).unwrap(), root);
    let enrollment: serde_json::Value =
        serde_json::from_slice(&fs::read("/etc/chariox/apps/runtime-enrollment.json").unwrap())
            .unwrap();
    assert!(enrollment["runtimeRoot"]
        .as_str()
        .unwrap()
        .starts_with("/var/lib/chariox-worker-groups-fixture/"));
    let groups = fs::read_to_string("/proc/self/status").unwrap();
    if std::env::var("CHARIOX_GROUPS_PARENT_PRIMARY_ONLY").as_deref() != Ok("1") {
        assert!(groups
            .lines()
            .find(|l| l.starts_with("Groups:"))
            .unwrap()
            .split_whitespace()
            .any(|s| s == "107"));
    }
    let public: [u8; 32] = [
        69, 129, 132, 5, 198, 231, 32, 67, 195, 76, 134, 66, 174, 122, 149, 148, 105, 248, 182,
        237, 12, 204, 72, 145, 102, 186, 217, 116, 101, 178, 195, 22,
    ];
    let publisher = TrustedPublisher {
        publisher_id: "com.chariox".into(),
        key_id: "dev-387f43602267b693373898be0be5ec8bcecb6ab5c49eb534ee6b936ab520056c".into(),
        public_key: VerifyingKey::from_bytes(&public).unwrap(),
    };
    let bytes = fs::read(root.join("package.cxapp")).unwrap();
    let policy = VerificationPolicy::new(385, vec![publisher.clone()]);
    let package = verify(&bytes, &policy).unwrap();
    let database = root.join("kernel.db");
    let mut db = rusqlite::Connection::open(&database).unwrap();
    let mut publishers = PublisherTrustRegistry::new(&mut db);
    publishers.initialize().unwrap();
    publishers
        .enroll(
            "owned-group-fixture",
            &publisher,
            0,
            &TrustDecision {
                decision_id: "owned-fixture-publisher".into(),
                authority_ref: "owned-validation".into(),
            },
            1,
        )
        .unwrap();
    let trust = publishers
        .trusted_publisher(
            "owned-group-fixture",
            &publisher.publisher_id,
            &publisher.key_id,
        )
        .unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&package, &trust).unwrap();
    let mut registry = InstallationRegistry::new(&mut db);
    registry.initialize().unwrap();
    let staged = registry
        .create_and_stage_verified("owned_group_fixture", "owned-group-fixture", &candidate, 1)
        .unwrap();
    registry
        .decide(
            &staged.token,
            CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "owned-fixture-approval".into(),
                    authority_ref: "owned-validation".into(),
                },
            },
            2,
        )
        .unwrap();
    registry.quiesce(&staged.token, 3).unwrap();
    let binding = registry
        .staged_trust("owned-group-fixture", &staged.token)
        .unwrap();
    let store = ReleaseStore::open_or_create(&database).unwrap();
    store
        .stage(
            &package,
            &bytes,
            StageBudget {
                max_stage_bytes: 128 * 1024 * 1024,
                reserved_bytes: 128 * 1024 * 1024,
                host_reserve_bytes: 10 * 1024 * 1024 * 1024,
            },
        )
        .unwrap();
    let lease = store.lease_verified(&package, &bytes).unwrap();
    let preparation = PreparedWorker::prepare_linux(
        EnrolledRuntime::open_installed().unwrap(),
        lease,
        &binding,
        None,
        0,
    );
    if std::env::var("CHARIOX_GROUPS_EXPECT_PREPARE_REFUSAL").as_deref() == Ok("1") {
        assert!(
            preparation.is_err(),
            "unenrolled code unexpectedly prepared"
        );
        fs::write(
            root.join("public-refusal-proof.json"),
            b"{\"preparationRefused\":true,\"appStarted\":false}\n",
        )
        .unwrap();
        return;
    }
    let prepared = preparation.unwrap();
    let started = WorkerProcess::spawn_blocking(prepared, WorkerLimits::default());
    if std::env::var("CHARIOX_GROUPS_EXPECT_REFUSAL").as_deref() == Ok("1") {
        assert!(
            started.is_err(),
            "authority-negative fixture unexpectedly launched"
        );
        fs::write(
            root.join("public-refusal-proof.json"),
            b"{\"launchRefused\":true,\"appStarted\":false}\n",
        )
        .unwrap();
        return;
    }
    let worker = started.unwrap();
    // The production observer already enforces caps, namespaces and exact child.
    // Independently retain only public process metadata while the App is alive.
    let cgroup = PathBuf::from(std::env::var("CHARIOX_GROUPS_APPS_CGROUP").unwrap());
    let mut app_status = Vec::new();
    for leaf in fs::read_dir(cgroup).unwrap().flatten() {
        if !leaf.file_name().to_string_lossy().starts_with("app-") {
            continue;
        }
        for pid in fs::read_to_string(leaf.path().join("cgroup.procs"))
            .unwrap()
            .split_whitespace()
        {
            let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            if !status
                .lines()
                .any(|l| l.starts_with("NSpid:") && l.split_whitespace().last() == Some("1"))
            {
                continue;
            }
            assert_eq!(
                status
                    .lines()
                    .find(|l| l.starts_with("Groups:"))
                    .unwrap()
                    .split_whitespace()
                    .count(),
                1
            );
            for field in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
                assert_eq!(
                    status
                        .lines()
                        .find(|l| l.starts_with(field))
                        .unwrap()
                        .split_whitespace()
                        .nth(1),
                    Some("0000000000000000")
                );
            }
            assert!(!PathBuf::from(format!("/proc/{pid}/fd/7")).exists());
            app_status.push(serde_json::json!({"pid":pid,"supplementaryGroups":0,"capabilities":"0","fd7Present":false}));
        }
    }
    assert_eq!(app_status.len(), 1);
    fs::write(
        root.join("public-worker-proof.json"),
        serde_json::to_vec_pretty(&app_status).unwrap(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    worker.shutdown_blocking().unwrap();
}
