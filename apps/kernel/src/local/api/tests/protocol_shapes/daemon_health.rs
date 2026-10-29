//! Health diagnostics use the shared protocol on every kernel placement.
use super::*;
use crate::runtime::projection::ProjectionInvariantHealthSnapshot;
use crate::runtime::request_metrics::RequestKindTiming;
use sha2::{Digest, Sha256};

#[test]
fn daemon_health_timing_and_sample_protocol_snapshot_and_hash() {
    let fixture = include_str!("daemon-health-890.json");
    let expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let invariants: ProjectionInvariantHealthSnapshot =
        serde_json::from_value(expected["projection_invariants"].clone()).unwrap();
    let timing: RequestKindTiming =
        serde_json::from_value(expected["request_timing"].clone()).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(expected["request"].clone()).unwrap();
    let durable_writer: crate::durable_state::DurableWriterHealthSnapshot =
        serde_json::from_value(expected["durable_writer"].clone()).unwrap();
    let actual = serde_json::json!({
        "durable_writer": durable_writer,
        "request": request,
        "projection_invariants": invariants,
        "request_timing": timing,
    });
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "2f6ec7e43f2a5b4f836537ff41fbb4b7e8328e1663319621ef8b39f44563ac81"
    );
}
