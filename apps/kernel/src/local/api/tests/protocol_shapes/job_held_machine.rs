//! Local 1012: a job request may name a machine the agent already holds.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn job_held_machine_1012_snapshot_and_hash() {
    let bytes = include_bytes!("job-held-machine-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "f04014fcfa4c61bd682ae839d55c529d37d8f199c1d51389a7bfcdfd6a932e8b"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["run_jobs"].as_array().unwrap() {
        let parsed: crate::local::RunJobRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
}

/// A request that names no machine is the one every job had before: it is placed by policy.
#[test]
fn a_job_request_without_a_machine_is_placed_by_policy() {
    let request: crate::local::RunJobRequest =
        serde_json::from_value(serde_json::json!({"command":["true"]})).unwrap();
    assert_eq!(request.machine, None);
}
