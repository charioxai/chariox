//! Test-only admission for the destructive storage fixture. Builder execution
//! must be inside its own disposable QEMU guest, never marked as hosted CI.
use sha2::{Digest, Sha256};
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
        let seed: [u8; 32] = fs::read(path).unwrap().try_into().unwrap();
        // Retain separate runtime-enrollment and App trust roles, as hosted
        // mode does. The fallback is the existing role-specific seed tag.
        derive_seed(seed, fallback)
    } else {
        fallback
    };
    ed25519_dalek::SigningKey::from_bytes(&seed)
}

fn derive_seed(seed: [u8; 32], role: [u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"chariox.storage-drill.signing-role.v1");
    hash.update(role);
    hash.update(seed);
    hash.finalize().into()
}

#[test]
fn derived_trust_roles_reject_each_others_signatures() {
    use ed25519_dalek::{Signer, SigningKey};
    let runtime = SigningKey::from_bytes(&derive_seed([9; 32], [71; 32]));
    let app = SigningKey::from_bytes(&derive_seed([9; 32], [27; 32]));
    assert_ne!(runtime.verifying_key(), app.verifying_key());
    let bytes = b"test-only role separation";
    assert!(app
        .verifying_key()
        .verify_strict(bytes, &runtime.sign(bytes))
        .is_err());
    assert!(runtime
        .verifying_key()
        .verify_strict(bytes, &app.sign(bytes))
        .is_err());
}
