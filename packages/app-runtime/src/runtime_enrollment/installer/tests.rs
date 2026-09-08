use super::*;
use crate::runtime_enrollment::EnrolledRuntime;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    roots: Roots,
    key: SigningKey,
}
struct Source {
    path: PathBuf,
    digest: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".chariox/dev");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "runtime-installer-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self::at(root)
    }
    fn at(root: PathBuf) -> Self {
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let roots = Roots {
            runtimes: root.join("runtimes"),
            enrollment: root.join("configuration"),
            uid: unsafe { libc::geteuid() },
            target: manifest::target().unwrap().into(),
        };
        fs::create_dir(&roots.runtimes).unwrap();
        fs::create_dir(&roots.enrollment).unwrap();
        Self {
            root,
            roots,
            key: SigningKey::from_bytes(&[71; 32]),
        }
    }
    fn source(&self, version: &str) -> Source {
        let path = self.root.join(format!("source-{version}"));
        fs::create_dir(&path).unwrap();
        let entries: Vec<_> = manifest::expected_paths(&self.roots.target).unwrap().into_iter().map(|name| {
            let bytes=format!("signed fixture {version} {name}\n"); let executable=manifest::executable(&name);
            write(&path.join(&name),bytes.as_bytes(),if executable {0o555}else{0o444});
            json!({"path":name,"size":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes.as_bytes())),"executable":executable})
        }).collect();
        let bytes=serde_json::to_vec(&json!({"schema":"chariox.app-runtime-inventory.v1","target":self.roots.target,
            "runtimeVersion":"0.1.0","workerAbi":1,"nodeVersion":"24.20.0","nodeModuleAbi":137,
            "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"sourceCommit":"a".repeat(40),"files":entries})).unwrap();
        write(&path.join(INVENTORY), &bytes, 0o444);
        let signature = self
            .key
            .sign(&bytes)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        write(&path.join(SIGNATURE), signature.as_bytes(), 0o444);
        write(&path.join(LEASE), b"", 0o444);
        Source {
            path,
            digest: format!("{:x}", Sha256::digest(&bytes)),
        }
    }
    fn install(&self, source: &Source) -> Result<InstallReceipt> {
        self.roots.install(
            &source.path,
            self.key.verifying_key().to_bytes(),
            &source.digest,
            &mut |_| Ok(()),
        )
    }
    fn cleanup(&self, source: &Source) -> Result<()> {
        self.roots.cleanup(&source.digest, &mut |_| Ok(()))
    }
    fn open(&self) -> EnrolledRuntime {
        EnrolledRuntime::open(&self.roots.enrollment.join(ENROLLMENT), self.roots.uid).unwrap()
    }
    fn current(&self) -> Enrollment {
        self.roots
            .current(&self.roots.context().unwrap())
            .unwrap()
            .unwrap()
    }
    fn fail(&self, source: &Source, checkpoint: Checkpoint) -> Result<InstallReceipt> {
        self.roots.install(
            &source.path,
            self.key.verifying_key().to_bytes(),
            &source.digest,
            &mut |at| {
                if at == checkpoint {
                    Err(EnrollmentError::Io(std::io::Error::other(
                        "injected interruption",
                    )))
                } else {
                    Ok(())
                }
            },
        )
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "dedicated hosted Linux root setup: installs tiny signed bytes, never executes them"]
fn hosted_install_signed_graph_for_storage_views() {
    assert_eq!(std::env::var("GITHUB_ACTIONS").unwrap(), "true");
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").unwrap(),
        "github-hosted"
    );
    assert_eq!(
        std::env::var("GITHUB_REPOSITORY").unwrap(),
        "charioxai/chariox"
    );
    assert_eq!(
        std::env::var("CHARIOX_STORAGE_HOSTED").unwrap(),
        "fixed-production-helper"
    );
    assert_eq!(unsafe { libc::getuid() }, 0);
    assert_eq!(unsafe { libc::geteuid() }, 0);
    // Refuse to overwrite any existing trust authority, even in a marked runner.
    assert!(!Path::new("/etc/chariox/apps/runtime-enrollment.json")
        .try_exists()
        .unwrap());
    let fixture = Fixture::at("/var/lib/chariox-runtime-fixture".into());
    let source = fixture.source("hosted-storage-views-only");
    let receipt = RuntimeInstaller::install(
        &source.path,
        fixture.key.verifying_key().to_bytes(),
        &source.digest,
    )
    .unwrap();
    assert_eq!(receipt.revision, 1);
    let enrolled = EnrolledRuntime::open_installed().unwrap();
    assert_eq!(enrolled.inventory_digest(), source.digest);
    assert!(enrolled.file("platform/libc.so.6").is_some());
    println!("Installed tiny signed graph {} for storage mount verification only; no App/native code executed", receipt.inventory_sha256);
    // The copied production graph and external enrollment remain for the next
    // nonroot storage drill. Only this private source fixture is removed here.
}
fn write(path: &Path, bytes: &[u8], mode: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    if path.exists() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if fs::symlink_metadata(path).unwrap().is_dir() {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                for entry in fs::read_dir(path).unwrap() {
                    writable(&entry.unwrap().path());
                }
            }
        }
        writable(&self.root);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn copied_signed_graph_is_enrolled_sealed_and_retry_is_revision_idempotent() {
    let fixture = Fixture::new();
    let source = fixture.source("first");
    assert_eq!(fixture.install(&source).unwrap().revision, 1);
    let runtime = fixture.open();
    assert_eq!(runtime.inventory_digest(), source.digest);
    assert_eq!(runtime.revision(), 1);
    assert!(runtime.file("sdk/src/index.js").is_some());
    let root = fixture.roots.runtimes.join(&source.digest);
    assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o555);
    assert_eq!(
        fs::metadata(root.join("sdk/src")).unwrap().mode() & 0o777,
        0o555
    );
    assert_eq!(
        fs::metadata(root.join("chariox-app-worker"))
            .unwrap()
            .mode()
            & 0o777,
        0o555
    );
    assert_eq!(
        fs::metadata(root.join("sdk/src/index.js")).unwrap().mode() & 0o777,
        0o444
    );
    assert_eq!(fixture.install(&source).unwrap().revision, 1);
    assert_eq!(
        fixture.current().public_key_hex,
        fixture
            .key
            .verifying_key()
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
}

#[test]
fn externally_selected_key_digest_and_exact_graph_fence_publication() {
    let fixture = Fixture::new();
    let old = fixture.source("old");
    fixture.install(&old).unwrap();
    let source = fixture.source("new");
    assert!(matches!(
        fixture.roots.install(
            &source.path,
            SigningKey::from_bytes(&[99; 32]).verifying_key().to_bytes(),
            &source.digest,
            &mut |_| Ok(())
        ),
        Err(EnrollmentError::Signature)
    ));
    assert!(matches!(
        fixture.roots.install(
            &source.path,
            fixture.key.verifying_key().to_bytes(),
            &"f".repeat(64),
            &mut |_| Ok(())
        ),
        Err(EnrollmentError::Identity)
    ));
    let payload = source.path.join("sdk/src/index.js");
    let original = fs::read(&payload).unwrap();
    write(&payload, b"substituted", 0o444);
    assert!(fixture.install(&source).is_err());
    write(&payload, &original, 0o444);
    write(&source.path.join("extra"), b"unsigned", 0o444);
    assert!(fixture.install(&source).is_err());
    fs::remove_file(source.path.join("extra")).unwrap();
    let link = fixture.root.join("hardlink");
    fs::hard_link(&payload, &link).unwrap();
    assert!(fixture.install(&source).is_err());
    fs::remove_file(link).unwrap();
    fs::remove_file(&payload).unwrap();
    symlink("../index.js", &payload).unwrap();
    assert!(fixture.install(&source).is_err());
    fs::remove_file(&payload).unwrap();
    write(&payload, &original, 0o444);
    assert_eq!(fixture.current().inventory_sha256, old.digest);
    assert!(!fixture.roots.runtimes.join(&source.digest).exists());
    assert_eq!(fixture.install(&source).unwrap().revision, 2);
}

#[test]
fn readers_retain_previous_generation_and_current_generation_cannot_be_retired() {
    let fixture = Fixture::new();
    let old = fixture.source("old");
    fixture.install(&old).unwrap();
    let reader = fixture.open();
    let new = fixture.source("new");
    assert_eq!(fixture.install(&new).unwrap().revision, 2);
    assert!(matches!(fixture.cleanup(&old), Err(EnrollmentError::Busy)));
    assert!(fixture.roots.runtimes.join(&old.digest).exists());
    assert!(matches!(fixture.cleanup(&new), Err(EnrollmentError::Busy)));
    drop(reader);
    fixture.cleanup(&old).unwrap();
    assert!(!fixture.roots.runtimes.join(&old.digest).exists());
    fixture.cleanup(&old).unwrap();
    assert_eq!(fixture.open().inventory_digest(), new.digest);
}

#[test]
fn interrupted_copy_or_publish_preserves_enrollment_and_retries_seal_then_publish() {
    for checkpoint in [Checkpoint::StageCopied, Checkpoint::GenerationPublished] {
        let fixture = Fixture::new();
        let old = fixture.source("old");
        fixture.install(&old).unwrap();
        let new = fixture.source("new");
        assert!(fixture.fail(&new, checkpoint).is_err());
        assert_eq!(fixture.current().inventory_sha256, old.digest);
        assert_eq!(fixture.install(&new).unwrap().revision, 2);
        assert_eq!(fixture.open().inventory_digest(), new.digest);
        assert_eq!(
            fs::metadata(fixture.roots.runtimes.join(&new.digest))
                .unwrap()
                .mode()
                & 0o777,
            0o555
        );
        assert!(!fixture.roots.runtimes.join(STAGING).exists());
    }
}

#[test]
fn enrollment_rename_lost_ack_is_recovered_without_double_increment_and_revision_never_wraps() {
    let fixture = Fixture::new();
    let old = fixture.source("old");
    fixture.install(&old).unwrap();
    let new = fixture.source("new");
    assert!(fixture.fail(&new, Checkpoint::EnrollmentRenamed).is_err());
    assert_eq!(fixture.install(&new).unwrap().revision, 2);
    assert_eq!(fixture.open().revision(), 2);
    let mut current = fixture.current();
    current.revision = i64::MAX as u64;
    write(
        &fixture.roots.enrollment.join(ENROLLMENT),
        &serde_json::to_vec(&current).unwrap(),
        0o444,
    );
    assert_eq!(fixture.install(&new).unwrap().revision, i64::MAX as u64);
    assert!(matches!(fixture.install(&old), Err(EnrollmentError::Limit)));
    assert_eq!(fixture.current().inventory_sha256, new.digest);
}

#[test]
fn interrupted_retirement_keeps_lease_until_remaining_payload_and_directory_are_removed() {
    let fixture = Fixture::new();
    let old = fixture.source("old");
    fixture.install(&old).unwrap();
    let new = fixture.source("new");
    fixture.install(&new).unwrap();
    let mut injected = false;
    assert!(fixture
        .roots
        .cleanup(&old.digest, &mut |at| {
            if at == Checkpoint::RetiringPayload && !injected {
                injected = true;
                Err(EnrollmentError::Io(std::io::Error::other(
                    "cleanup interruption",
                )))
            } else {
                Ok(())
            }
        })
        .is_err());
    assert!(injected);
    let retiring = fixture
        .roots
        .runtimes
        .join(format!("{RETIRING}{}", old.digest));
    assert!(retiring.join(LEASE).exists());
    assert!(!fixture.roots.runtimes.join(&old.digest).exists());
    let lease = File::open(retiring.join(LEASE)).unwrap();
    filesystem::shared_lease(&lease).unwrap();
    assert!(matches!(fixture.cleanup(&old), Err(EnrollmentError::Busy)));
    assert!(retiring.join(LEASE).exists());
    drop(lease);
    fixture.cleanup(&old).unwrap();
    assert!(!retiring.exists());
    assert_eq!(fixture.open().inventory_digest(), new.digest);
}

#[test]
fn global_installer_lock_and_retained_generation_capacity_bound_mutation() {
    let fixture = Fixture::new();
    let source = fixture.source("first");
    let context = fixture.roots.context().unwrap();
    assert!(matches!(
        fixture.install(&source),
        Err(EnrollmentError::Busy)
    ));
    drop(context);
    fixture.install(&source).unwrap();
    for index in 1..MAX_GENERATIONS {
        fixture
            .install(&fixture.source(&format!("generation-{index}")))
            .unwrap();
    }
    let extra = fixture.source("extra");
    assert!(matches!(
        fixture.install(&extra),
        Err(EnrollmentError::Limit)
    ));
    fixture.cleanup(&source).unwrap();
    assert_eq!(
        fixture.install(&extra).unwrap().revision,
        (MAX_GENERATIONS + 1) as u64
    );
}
