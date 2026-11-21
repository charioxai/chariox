//! Local 1013: an explicit job machine can name an enrolled kernel.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn job_machine_1013_snapshot_and_hash() {
    let bytes = include_bytes!("job-machines-1013.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "3c49d129359b80dba27858e168419dd3ca989d77cf835a818ce57d6f3c207605"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["run_jobs"].as_array().unwrap() {
        let parsed: crate::local::RunJobRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    let wire = &fixture["run_process"];
    let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
}

/// A request that names no machine is the one every job had before: it is placed by policy.
#[test]
fn a_job_request_without_a_machine_is_placed_by_policy() {
    let request: crate::local::RunJobRequest =
        serde_json::from_value(serde_json::json!({"command":["true"]})).unwrap();
    assert_eq!(request.machine, None);
}
