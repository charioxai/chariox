use super::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use std::{
    fs,
    os::{
        fd::AsRawFd,
        unix::fs::{symlink, PermissionsExt},
    },
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    enrollment: PathBuf,
    runtime: PathBuf,
    key: SigningKey,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Tests live in user-owned drill state, outside source trees. Unlike
        // /tmp, every ancestor here must satisfy the production path checks.
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let parent = home.join(".chariox/dev");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "runtime-enrollment-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = root.join("runtime");
        fs::create_dir(&runtime).unwrap();
        let fixture = Self {
            enrollment: root.join("enrollment.json"),
            root,
            runtime,
            key: SigningKey::from_bytes(&[71; 32]),
        };
        let files: Vec<_> = manifest::expected_paths(manifest::target().unwrap()).unwrap().into_iter().map(|path| {
            let bytes = format!("signed fixture {path}\n").into_bytes();
            let executable = manifest::executable(&path);
            fixture.write(&path, &bytes, if executable {0o555} else {0o444});
            json!({"path":path,"size":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes)),"executable":executable})
        }).collect();
        fixture.write(LEASE, b"", 0o444);
        fixture.sign(json!({"schema":"chariox.app-runtime-inventory.v1","target":manifest::target().unwrap(),
            "runtimeVersion":"0.1.0","workerAbi":1,"nodeVersion":"24.20.0","nodeModuleAbi":137,
            "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"sourceCommit":"a".repeat(40),"files":files}));
        fixture
    }
    fn write(&self, name: &str, bytes: &[u8], mode: u32) {
        let path = self.runtime.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if path.exists() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    }
    fn sign(&self, inventory: Value) {
        let bytes = serde_json::to_vec(&inventory).unwrap();
        self.write(INVENTORY, &bytes, 0o444);
        let signature: String = self
            .key
            .sign(&bytes)
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.write(SIGNATURE, signature.as_bytes(), 0o444);
        self.enroll(&bytes, self.key.verifying_key());
    }
    fn enroll(&self, bytes: &[u8], key: VerifyingKey) {
        let key: String = key
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        fs::write(
            &self.enrollment,
            serde_json::to_vec(&json!({"schema":"chariox.app-runtime-enrollment.v1",
            "revision":1,"target":manifest::target().unwrap(),"runtimeRoot":self.runtime,
            "inventorySha256":format!("{:x}",Sha256::digest(bytes)),"publicKeyHex":key}))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&self.enrollment, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn open(&self) -> Result<EnrolledRuntime> {
        EnrolledRuntime::open(&self.enrollment, unsafe { libc::geteuid() })
    }
    fn inventory(&self) -> Value {
        serde_json::from_slice(&fs::read(self.runtime.join(INVENTORY)).unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn signature_pins_exact_inventory_and_keeps_an_installer_lease_until_drop() {
    let fixture = Fixture::new();
    let runtime = fixture.open().unwrap();
    assert_eq!(runtime.revision(), 1);
    assert_eq!(runtime.target(), manifest::target().unwrap());
    assert!(runtime.file("sdk/src/index.js").is_some());
    let lease = File::open(fixture.runtime.join(LEASE)).unwrap();
    assert_ne!(
        unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    // An independent reader may share the same installed generation.
    let second = fixture.open().unwrap();
    drop(runtime);
    assert_ne!(
        unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    drop(second);
    assert_eq!(
        unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert!(matches!(fixture.open(), Err(EnrollmentError::Busy)));
}

#[test]
fn changing_manifest_or_trust_never_accepts_bundle_supplied_authority() {
    let fixture = Fixture::new();
    let mut inventory = fixture.inventory();
    inventory["sourceCommit"] = json!("b".repeat(40));
    let changed = serde_json::to_vec(&inventory).unwrap();
    fixture.write(INVENTORY, &changed, 0o444);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Identity)));
    fixture.enroll(&changed, fixture.key.verifying_key());
    assert!(matches!(fixture.open(), Err(EnrollmentError::Signature)));
    fixture.sign(inventory.clone());
    let bytes = fs::read(fixture.runtime.join(INVENTORY)).unwrap();
    fixture.enroll(&bytes, SigningKey::from_bytes(&[99; 32]).verifying_key());
    assert!(matches!(fixture.open(), Err(EnrollmentError::Signature)));
    inventory["nodeModuleAbi"] = json!(136);
    fixture.sign(inventory);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Contract)));
}

#[test]
fn actual_file_digest_modes_links_extra_entries_and_manifest_duplicates_are_rejected() {
    let fixture = Fixture::new();
    let path = "sdk/src/index.js";
    let original = fs::read(fixture.runtime.join(path)).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fixture.write(path, &changed, 0o444);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Identity)));
    fixture.write(path, &original, 0o644);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Identity)));
    fixture.write(path, &original, 0o444);
    let link = fixture.root.join("outside-link");
    fs::hard_link(fixture.runtime.join(path), &link).unwrap();
    assert!(matches!(fixture.open(), Err(EnrollmentError::Identity)));
    fs::remove_file(link).unwrap();
    fs::remove_file(fixture.runtime.join(path)).unwrap();
    symlink("internal.js", fixture.runtime.join(path)).unwrap();
    assert!(fixture.open().is_err());
    fs::remove_file(fixture.runtime.join(path)).unwrap();
    fixture.write(path, &original, 0o444);
    fixture.write("undeclared.js", b"extra", 0o444);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Identity)));
    fs::remove_file(fixture.runtime.join("undeclared.js")).unwrap();
    let mut inventory = fixture.inventory();
    let duplicate = inventory["files"][0].clone();
    inventory["files"].as_array_mut().unwrap().push(duplicate);
    fixture.sign(inventory);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Contract)));
}

#[test]
fn linux_platform_contract_rejects_missing_foreign_or_executable_libraries() {
    for target in ["linux-x64", "linux-arm64"] {
        let files: Vec<_> = manifest::expected_paths(target)
            .unwrap()
            .into_iter()
            .map(|path| {
                json!({"executable":manifest::executable(&path),"path":path,
                    "size":1,"sha256":"a".repeat(64)})
            })
            .collect();
        assert_eq!(
            files
                .iter()
                .filter(|file| file["path"].as_str().unwrap().starts_with("platform/"))
                .count(),
            8
        );
        assert!(files.len() <= 40);
        let inventory = json!({"schema":"chariox.app-runtime-inventory.v1","target":target,
            "runtimeVersion":"0.1.0","workerAbi":1,"nodeVersion":"24.20.0","nodeModuleAbi":137,
            "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"sourceCommit":"a".repeat(40),"files":files});
        let validate = |value: Value| {
            serde_json::from_value::<manifest::Inventory>(value)
                .unwrap()
                .validate(target)
        };
        assert!(validate(inventory.clone()).is_ok());
        for expected in inventory["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|file| file["path"].as_str().unwrap().starts_with("platform/"))
        {
            let mut missing = inventory.clone();
            missing["files"]
                .as_array_mut()
                .unwrap()
                .retain(|file| file["path"] != expected["path"]);
            assert!(matches!(validate(missing), Err(EnrollmentError::Contract)));
        }
        let mut wrong_loader = inventory.clone();
        let loader = wrong_loader["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| {
                file["path"]
                    .as_str()
                    .unwrap()
                    .starts_with("platform/ld-linux")
            })
            .unwrap();
        loader["path"] = json!(if target == "linux-x64" {
            "platform/ld-linux-aarch64.so.1"
        } else {
            "platform/ld-linux-x86-64.so.2"
        });
        assert!(matches!(
            validate(wrong_loader),
            Err(EnrollmentError::Contract)
        ));
        let mut executable_library = inventory.clone();
        executable_library["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| file["path"] == "platform/libc.so.6")
            .unwrap()["executable"] = json!(true);
        assert!(matches!(
            validate(executable_library),
            Err(EnrollmentError::Contract)
        ));
        let mut host_path = inventory;
        host_path["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| file["path"] == "platform/libc.so.6")
            .unwrap()["path"] = json!("/lib/x86_64-linux-gnu/libc.so.6");
        assert!(matches!(
            validate(host_path),
            Err(EnrollmentError::Contract)
        ));
    }
    for target in ["darwin-x64", "darwin-arm64"] {
        assert!(!manifest::expected_paths(target)
            .unwrap()
            .iter()
            .any(|path| path.starts_with("platform/")));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn signed_platform_library_bytes_and_exact_graph_are_verified_before_admission() {
    let fixture = Fixture::new();
    let path = "platform/libc.so.6";
    assert!(fixture.open().unwrap().file(path).is_some());
    let original = fs::read(fixture.runtime.join(path)).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fixture.write(path, &changed, 0o444);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Identity)));
    fixture.write(path, b"substituted host library", 0o444);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Limit)));
    fixture.write(path, &original, 0o444);
    let mut inventory = fixture.inventory();
    inventory["files"]
        .as_array_mut()
        .unwrap()
        .retain(|file| file["path"] != path);
    fixture.sign(inventory);
    assert!(matches!(fixture.open(), Err(EnrollmentError::Contract)));
}
