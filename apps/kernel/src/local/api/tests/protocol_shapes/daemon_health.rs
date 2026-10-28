//! Health diagnostics use the shared protocol on every kernel placement.
use super::*;
use crate::runtime::projection::ProjectionInvariantHealthSnapshot;
use crate::runtime::request_metrics::RequestKindTiming;
use sha2::{Digest, Sha256};

#[test]
fn daemon_health_timing_and_sample_protocol_snapshot_and_hash() {
    let fixture = include_str!("daemon-health-790.json");
    let expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let invariants: ProjectionInvariantHealthSnapshot =
        serde_json::from_value(expected["projection_invariants"].clone()).unwrap();
    let timing: RequestKindTiming =
        serde_json::from_value(expected["request_timing"].clone()).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(expected["request"].clone()).unwrap();
    let actual = serde_json::json!({
        "request": request,
        "projection_invariants": invariants,
        "request_timing": timing,
    });
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "756199efda58f7c9133fbca82985a1f00a5ba6d33255a764d52c8ddf5aa4cceb"
    );
}
