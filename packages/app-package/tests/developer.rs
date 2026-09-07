#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    fs,
    os::unix::{
        fs::{symlink, DirBuilderExt, MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::PathBuf,
    process::Command,
};

use chariox_app_package::{
    developer::{self, ManifestOptions},
    ErrorCode, Limits, SUPPORTED_SDK_VERSION,
};
use rand::{rngs::OsRng, RngCore};
use serde_json::Value;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-package-dev-{}-{:016x}",
                std::process::id(),
                OsRng.next_u64()
            ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join("keys"))
            .unwrap();
        fs::create_dir_all(root.join("bundle/runtime")).unwrap();
        fs::create_dir_all(root.join("bundle/ui")).unwrap();
        let sentinel =
            serde_json::to_string(&root.join("should-not-run").to_string_lossy()).unwrap();
        fs::write(root.join("bundle/runtime/main.js"), format!("require('node:fs').writeFileSync({sentinel},'executed');throw new Error('never execute while packing');")).unwrap();
        fs::write(
            root.join("bundle/ui/index.html"),
            "<!doctype html><title>Test</title>",
        )
        .unwrap();
        Self { root }
    }
    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
    fn keygen(&self) -> developer::PublisherFile {
        developer::keygen(
            "com.example",
            "Developer",
            &self.path("keys/private"),
            &self.path("publisher.json"),
        )
        .unwrap();
        developer::read_publisher(&self.path("publisher.json"), &Limits::default()).unwrap()
    }
    fn manifest(&self) -> chariox_app_package::Manifest {
        let publisher = self.keygen();
        developer::generate_manifest(
            ManifestOptions::new(
                "com.example.test".to_owned(),
                "1.0.0".to_owned(),
                publisher.publisher,
                285,
            ),
            &Limits::default(),
        )
        .unwrap()
    }
    fn cli(&self, arguments: &[&str], status: i32) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_chariox-app-package"))
            .current_dir(&self.root)
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(status),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["ok"], status == 0);
        value
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn executable_round_trip_needs_no_handwritten_manifest_or_code_execution() {
    let fixture = Fixture::new();
    let generated = fixture.cli(
        &[
            "keygen",
            "--publisher-id",
            "com.example",
            "--publisher-name",
            "Developer",
            "--key-out",
            "keys/private",
            "--trust-out",
            "publisher.json",
        ],
        0,
    );
    assert_eq!(
        generated["result"]["trustStatus"],
        "enrollment-material-only"
    );
    assert!(generated["result"].get("publicKey").is_none());
    assert!(generated["result"].get("seed").is_none());
    let secret = fs::read(fixture.path("keys/private")).unwrap();
    assert_eq!(secret.len(), 32);
    assert_eq!(
        fs::metadata(fixture.path("keys/private")).unwrap().nlink(),
        1
    );
    assert_eq!(
        fs::metadata(fixture.path("keys/private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let created = fixture.cli(
        &[
            "manifest",
            "--app-id",
            "com.example.test",
            "--version",
            "1.0.0",
            "--publisher",
            "publisher.json",
            "--kernel-protocol",
            "285",
            "--output",
            "app-manifest.json",
            "--network",
            "GET,POST=https://api.example.com",
        ],
        0,
    );
    assert_eq!(
        created["result"]["manifest"]["sdkVersion"],
        SUPPORTED_SDK_VERSION
    );
    assert_eq!(
        created["result"]["manifest"]["capabilities"]["agents"],
        serde_json::json!([])
    );
    let first = fixture.cli(
        &[
            "pack",
            "--bundle",
            "bundle",
            "--manifest",
            "app-manifest.json",
            "--key",
            "keys/private",
            "--output",
            "first.cxapp",
            "--kernel-protocol",
            "285",
        ],
        0,
    );
    let second = fixture.cli(
        &[
            "pack",
            "--bundle",
            "bundle",
            "--manifest",
            "app-manifest.json",
            "--key",
            "keys/private",
            "--output",
            "second.cxapp",
            "--kernel-protocol",
            "285",
        ],
        0,
    );
    assert_eq!(first, second);
    assert_eq!(
        fs::read(fixture.path("first.cxapp")).unwrap(),
        fs::read(fixture.path("second.cxapp")).unwrap()
    );
    let inspected = fixture.cli(&["inspect", "first.cxapp"], 0);
    assert_eq!(inspected["result"]["status"], "untrusted-claims-only");
    let validated = fixture.cli(
        &[
            "validate",
            "first.cxapp",
            "--trust",
            "publisher.json",
            "--kernel-protocol",
            "285",
        ],
        0,
    );
    assert_eq!(
        validated["result"]["status"],
        "verified-against-explicit-publisher-file"
    );
    assert_eq!(validated["result"]["fileCount"], 2);
    assert!(!fixture.path("should-not-run").exists());

    let mut tampered = fs::read(fixture.path("first.cxapp")).unwrap();
    let index = tampered
        .windows(b"<!doctype".len())
        .position(|bytes| bytes == b"<!doctype")
        .unwrap();
    tampered[index] ^= 1;
    fs::write(fixture.path("tampered.cxapp"), tampered).unwrap();
    assert_eq!(
        fixture.cli(&["inspect", "tampered.cxapp"], 0)["result"]["status"],
        "untrusted-claims-only"
    );
    assert_eq!(
        fixture.cli(
            &[
                "validate",
                "tampered.cxapp",
                "--trust",
                "publisher.json",
                "--kernel-protocol",
                "285"
            ],
            3
        )["error"]["code"],
        "INTEGRITY_MISMATCH"
    );
    assert_eq!(
        fixture.cli(
            &[
                "validate",
                "first.cxapp",
                "--trust",
                "publisher.json",
                "--kernel-protocol",
                "284"
            ],
            3
        )["error"]["code"],
        "INCOMPATIBLE_PROTOCOL"
    );
}

#[test]
fn keygen_sets_exact_private_mode_even_under_restrictive_umask() {
    let fixture = Fixture::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_chariox-app-package"));
    command.current_dir(&fixture.root).args([
        "keygen",
        "--publisher-id",
        "com.example",
        "--publisher-name",
        "Developer",
        "--key-out",
        "keys/private",
        "--trust-out",
        "publisher.json",
    ]);
    // Change the child only; parallel Rust tests keep their existing umask.
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o777);
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        fs::metadata(fixture.path("keys/private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read(fixture.path("keys/private")).unwrap().len(), 32);
}

#[test]
fn wrong_key_cannot_pack_or_validate_as_an_enrolled_publisher() {
    let fixture = Fixture::new();
    let manifest = fixture.manifest();
    developer::keygen(
        "com.example",
        "Other",
        &fixture.path("keys/other"),
        &fixture.path("other.json"),
    )
    .unwrap();
    assert_eq!(
        developer::pack_directory(
            &fixture.path("bundle"),
            &manifest,
            &fixture.path("keys/other"),
            &fixture.path("bad.cxapp"),
            285,
            &Limits::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidDeveloperKey
    );
    assert!(!fixture.path("bad.cxapp").exists());
    developer::pack_directory(
        &fixture.path("bundle"),
        &manifest,
        &fixture.path("keys/private"),
        &fixture.path("good.cxapp"),
        285,
        &Limits::default(),
    )
    .unwrap();
    let other = developer::read_publisher(&fixture.path("other.json"), &Limits::default()).unwrap();
    assert_eq!(
        developer::validate_archive(&fixture.path("good.cxapp"), &other, 285, &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::UntrustedPublisher
    );
}

#[test]
fn keygen_preserves_existing_files_and_cleans_unpublished_temporary_secrets() {
    let fixture = Fixture::new();
    fs::write(fixture.path("publisher.json"), "preserve enrollment").unwrap();
    assert!(developer::keygen(
        "com.example",
        "Developer",
        &fixture.path("keys/private"),
        &fixture.path("publisher.json")
    )
    .is_err());
    assert!(!fixture.path("keys/private").exists());
    assert_eq!(
        fs::read(fixture.path("publisher.json")).unwrap(),
        b"preserve enrollment"
    );
    assert_eq!(fs::read_dir(fixture.path("keys")).unwrap().count(), 0);
    fs::remove_file(fixture.path("publisher.json")).unwrap();
    assert_eq!(
        developer::keygen(
            "com.example",
            "Developer",
            &fixture.path("keys/private"),
            &fixture.path("keys/./private")
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidArguments
    );
    assert_eq!(fs::read_dir(fixture.path("keys")).unwrap().count(), 0);
    fixture.keygen();
    let before = fs::read(fixture.path("keys/private")).unwrap();
    assert!(developer::keygen(
        "com.example",
        "Developer",
        &fixture.path("keys/private"),
        &fixture.path("another.json")
    )
    .is_err());
    assert_eq!(fs::read(fixture.path("keys/private")).unwrap(), before);
    assert!(!fixture.path("another.json").exists());
}

#[test]
fn private_key_permissions_and_parent_links_are_rejected() {
    let fixture = Fixture::new();
    let manifest = fixture.manifest();
    fs::set_permissions(
        fixture.path("keys/private"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        developer::pack_directory(
            &fixture.path("bundle"),
            &manifest,
            &fixture.path("keys/private"),
            &fixture.path("bad.cxapp"),
            285,
            &Limits::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidDeveloperKey
    );
    fs::set_permissions(
        fixture.path("keys/private"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    symlink(fixture.path("keys"), fixture.path("linked-keys")).unwrap();
    assert_eq!(
        developer::keygen(
            "com.example",
            "Developer",
            &fixture.path("linked-keys/new"),
            &fixture.path("new.json")
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidPath
    );
    assert!(!fixture.path("keys/new").exists());
    fs::set_permissions(fixture.path("keys"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        developer::keygen(
            "com.example",
            "Developer",
            &fixture.path("keys/new"),
            &fixture.path("new.json")
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidDeveloperKey
    );
}

#[test]
fn bundle_rejects_link_escapes_hardlinks_special_files_and_limits() {
    let fixture = Fixture::new();
    fs::write(fixture.path("outside"), "not in bundle").unwrap();
    let escape = fixture.path("bundle/escape");
    symlink(fixture.path("outside"), &escape).unwrap();
    assert_eq!(
        developer::read_bundle(&fixture.path("bundle"), &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
    fs::remove_file(&escape).unwrap();
    symlink(fixture.path("keys"), &escape).unwrap();
    assert_eq!(
        developer::read_bundle(&fixture.path("bundle"), &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
    fs::remove_file(&escape).unwrap();
    fs::hard_link(fixture.path("outside"), &escape).unwrap();
    assert_eq!(
        developer::read_bundle(&fixture.path("bundle"), &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
    fs::remove_file(&escape).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let fifo = std::ffi::CString::new(escape.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_eq!(
        developer::read_bundle(&fixture.path("bundle"), &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
    fs::remove_file(&escape).unwrap();
    let mut limits = Limits::default();
    limits.max_entries = 2;
    assert_eq!(
        developer::read_bundle(&fixture.path("bundle"), &limits)
            .unwrap_err()
            .code,
        ErrorCode::ArchiveLimit
    );
    limits = Limits::default();
    limits.max_file_bytes = 8;
    assert_eq!(
        developer::read_bundle(&fixture.path("bundle"), &limits)
            .unwrap_err()
            .code,
        ErrorCode::ArchiveLimit
    );
    symlink(fixture.path("bundle"), fixture.path("linked-bundle")).unwrap();
    assert_eq!(
        developer::read_bundle(&fixture.path("linked-bundle"), &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
}

#[test]
fn output_is_atomic_no_replace_and_cannot_include_its_signing_key() {
    let fixture = Fixture::new();
    let manifest = fixture.manifest();
    let output = fixture.path("keep.cxapp");
    fs::write(&output, "retain previous package").unwrap();
    assert_eq!(
        developer::pack_directory(
            &fixture.path("bundle"),
            &manifest,
            &fixture.path("keys/private"),
            &output,
            285,
            &Limits::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::Io
    );
    assert_eq!(fs::read(&output).unwrap(), b"retain previous package");
    assert_eq!(
        developer::pack_directory(
            &fixture.path("bundle"),
            &manifest,
            &fixture.path("keys/private"),
            &fixture.path("bundle/accidental.cxapp"),
            285,
            &Limits::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidPath
    );
    assert_eq!(
        developer::pack_directory(
            &fixture.root,
            &manifest,
            &fixture.path("keys/private"),
            &fixture.path("secret.cxapp"),
            285,
            &Limits::default()
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidPath
    );
    assert!(!fixture.path("bundle/accidental.cxapp").exists());
    assert!(!fixture.path("secret.cxapp").exists());
    assert!(!fs::read_dir(&fixture.root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".cxapp-write-")));
}

#[test]
fn cli_errors_are_stable_json_and_generated_contract_tracks_sdk() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.cli(&["install", "untrusted.cxapp"], 2)["error"]["code"],
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        fixture.cli(&["inspect", "missing.cxapp"], 4)["error"]["code"],
        "IO"
    );
    assert_eq!(
        fixture.cli(
            &[
                "validate",
                "x",
                "--trust",
                "a",
                "--trust",
                "b",
                "--kernel-protocol",
                "285"
            ],
            2
        )["error"]["code"],
        "INVALID_ARGUMENTS"
    );
    let sdk: Value = serde_json::from_str(include_str!("../../app-sdk/package.json")).unwrap();
    assert_eq!(sdk["version"], SUPPORTED_SDK_VERSION);
    let manifest = fixture.manifest();
    assert_eq!(
        manifest.app_contract_version,
        chariox_app_package::APP_CONTRACT_VERSION
    );
    assert_eq!(
        manifest.runtime.engine,
        chariox_app_package::RuntimeEngine::Node
    );
    assert!(manifest.capabilities.network.is_empty());
    let manifest_path = fixture.path("duplicate.json");
    fs::write(
        &manifest_path,
        r#"{"schema":"chariox.app.v1","schema":"overwritten"}"#,
    )
    .unwrap();
    assert_eq!(
        developer::read_manifest(&manifest_path, &Limits::default())
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest
    );
}
