//! Test-only admission for the destructive storage fixture. Builder execution
//! must be inside its own disposable QEMU guest, never marked as hosted CI.
use std::{fs, os::unix::fs::MetadataExt, process::Command};

pub(crate) fn require_dedicated_machine() {
    if std::env::var("CHARIOX_STORAGE_PRIVATE_VM").as_deref() == Ok("1") {
        assert!(std::env::var_os("GITHUB_ACTIONS").is_none());
        assert_eq!(
            fs::read_to_string("/etc/hostname").unwrap().trim(),
            "chariox-private-storage-drill"
        );
        let output = Command::new("/usr/bin/systemd-detect-virt")
            .arg("--vm")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "qemu");
    } else {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        assert_eq!(
            std::env::var("RUNNER_ENVIRONMENT").as_deref(),
            Ok("github-hosted")
        );
        assert_eq!(
            std::env::var("GITHUB_REPOSITORY").as_deref(),
            Ok("charioxai/chariox")
        );
    }
    assert_eq!(
        std::env::var("CHARIOX_STORAGE_HOSTED").as_deref(),
        Ok("fixed-production-helper")
    );
}

pub(crate) fn signing_key(fallback: [u8; 32]) -> ed25519_dalek::SigningKey {
    let seed = if std::env::var("CHARIOX_STORAGE_PRIVATE_VM").as_deref() == Ok("1") {
        require_dedicated_machine();
        let path = std::env::var_os("CHARIOX_STORAGE_DRILL_KEY").unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.mode() & 0o777, 0o600);
        fs::read(path).unwrap().try_into().unwrap()
    } else {
        fallback
    };
    ed25519_dalek::SigningKey::from_bytes(&seed)
}
