//! Machine-scoped, per-creation identities for local Docker slice workers.

use crate::error::DaemonError;
use sha2::{Digest, Sha256};

pub(super) fn new_local_docker_worker_ref(
    machine_id: &str,
    owner_kernel_id: &str,
    local_name: &str,
) -> String {
    worker_ref_with_nonce(
        machine_id,
        owner_kernel_id,
        local_name,
        &format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        ),
    )
}

fn worker_ref_with_nonce(
    machine_id: &str,
    owner_kernel_id: &str,
    local_name: &str,
    creation_nonce: &str,
) -> String {
    let tuple = serde_json::to_vec(&[
        "chariox.slice-worker.v1",
        owner_kernel_id,
        local_name,
        creation_nonce,
    ])
    .expect("serializing slice identity strings cannot fail");
    format!(
        "slice:{:x}:{:x}",
        Sha256::digest(machine_id.as_bytes()),
        Sha256::digest(tuple)
    )
}

pub(super) fn qualified_worker_ref_parts(worker_ref: &str) -> Option<(&str, &str)> {
    let (machine, creation) = worker_ref.strip_prefix("slice:")?.split_once(':')?;
    [machine, creation]
        .into_iter()
        .all(|part| {
            part.len() == 64
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .then_some((machine, creation))
}

pub(crate) fn machine_scoped_slice_worker_ref(worker_ref: &str, machine_id: &str) -> bool {
    if machine_id.trim().is_empty() {
        return false;
    }
    let Some((machine_hash, _)) = qualified_worker_ref_parts(worker_ref) else {
        return false;
    };
    machine_hash == format!("{:x}", Sha256::digest(machine_id.as_bytes()))
}

pub(crate) fn require_hosted_slice_worker_ref(
    worker_ref: &str,
    machine_id: &str,
) -> Result<(), DaemonError> {
    if machine_scoped_slice_worker_ref(worker_ref, machine_id) {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.relay_identity",
        message: "hosted slice worker reference is legacy or belongs to another Machine; create a new slice with a machine-scoped worker identity. Existing slice state was not migrated".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_worker_identity_matches_shared_cross_language_vectors() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/slice-worker-identity.json"
        )))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let field = |name: &str| case[name].as_str().unwrap();
            let actual = worker_ref_with_nonce(
                field("machineId"),
                field("ownerKernelId"),
                field("localName"),
                field("creationNonce"),
            );
            assert_eq!(actual, field("workerKernelRef"), "{}", field("label"));
            assert_eq!(actual.len(), 135);
            assert!(machine_scoped_slice_worker_ref(&actual, field("machineId")));
            assert!(!machine_scoped_slice_worker_ref(&actual, "foreign-machine"));
        }
        assert_ne!(
            worker_ref_with_nonce("m", "a", "bc", "nonce"),
            worker_ref_with_nonce("m", "ab", "c", "nonce")
        );
    }

    #[test]
    fn slice_worker_identity_rejects_legacy_foreign_and_malformed_hosted_refs() {
        let valid = worker_ref_with_nonce("machine-a", "kernel-a", "drill", "nonce");
        require_hosted_slice_worker_ref(&valid, "machine-a").unwrap();
        for invalid in [
            "slice:drill".to_string(),
            valid.to_uppercase(),
            format!("{valid}:extra"),
            valid[..134].to_string(),
        ] {
            assert!(require_hosted_slice_worker_ref(&invalid, "machine-a").is_err());
        }
        assert!(require_hosted_slice_worker_ref(&valid, "machine-b").is_err());
        assert!(require_hosted_slice_worker_ref(&valid, "").is_err());
    }
}
