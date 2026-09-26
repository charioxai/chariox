use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use super::*;

const MACHINE_CREDENTIAL: &str = "mcred_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN";
const ACTIVITY_CHANGED_AT: &str = "1970-01-01T00:00:01.000Z";

fn signed_activity_contract(resource_key: &'static str) -> (String, String) {
    let fields: BTreeMap<&str, Value> = BTreeMap::from([
        ("accountId", json!("acct-1")),
        ("activityChangedAt", json!(ACTIVITY_CHANGED_AT)),
        (resource_key, json!("env-1")),
        ("kernelId", json!("kernel-1")),
        ("machineId", json!("machine-1")),
        ("runningAgentCount", json!(1)),
        ("sequence", json!(7)),
    ]);
    let canonical = serde_json::to_string(&fields).expect("activity should serialize");
    let mut mac = Hmac::<Sha256>::new_from_slice(MACHINE_CREDENTIAL.as_bytes())
        .expect("machine credential should initialize HMAC");
    mac.update(canonical.as_bytes());
    let signature = format!("sha256:{:x}", mac.finalize().into_bytes());
    (canonical, signature)
}

#[test]
fn managed_activity_http_signed_contract_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 355);

    let (managed, managed_signature) = signed_activity_contract("environmentId");
    assert_eq!(
        managed,
        "{\"accountId\":\"acct-1\",\"activityChangedAt\":\"1970-01-01T00:00:01.000Z\",\"environmentId\":\"env-1\",\"kernelId\":\"kernel-1\",\"machineId\":\"machine-1\",\"runningAgentCount\":1,\"sequence\":7}"
    );
    assert_eq!(
        managed_signature,
        "sha256:2e4cc0ff504b4222281a8bc3e08349cc78b9591fec4d0abd08ff14f1a1598351"
    );

    let (worker, worker_signature) = signed_activity_contract("allocationId");
    assert_eq!(
        worker,
        "{\"accountId\":\"acct-1\",\"activityChangedAt\":\"1970-01-01T00:00:01.000Z\",\"allocationId\":\"env-1\",\"kernelId\":\"kernel-1\",\"machineId\":\"machine-1\",\"runningAgentCount\":1,\"sequence\":7}"
    );
    assert_eq!(
        worker_signature,
        "sha256:42b9ef2e9414baf065431cd5fc7df85e3cef537a6ba697ba43fe752348dbf1b9"
    );
}

#[test]
fn managed_activity_http_producer_keeps_timestamp_and_both_receiver_routes() {
    let producer = include_str!("../../../../runtime/managed_kernel_activity.rs");
    for required in [
        "activity_changed_at: &'a str",
        "canonical_activity_timestamp",
        "chrono::SecondsFormat::Millis",
        "ACTIVITY_ENDPOINT: &str = \"/v1/managed-kernels/activity\"",
        "\"/v1/disposable-workers/activity\"",
    ] {
        assert!(
            producer.contains(required),
            "managed activity producer lost protocol-340 marker {required:?}"
        );
    }
}
