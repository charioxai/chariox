use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use super::*;

const MACHINE_CREDENTIAL: &str = "mcred_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN";
const ACTIVITY_CHANGED_AT: &str = "1970-01-01T00:00:01.000Z";

fn signed_activity_contract() -> (String, String) {
    let fields: BTreeMap<&str, Value> = BTreeMap::from([
        ("accountId", json!("acct-1")),
        ("activityChangedAt", json!(ACTIVITY_CHANGED_AT)),
        ("environmentId", json!("env-1")),
        ("kernelId", json!("kernel-1")),
        ("machineId", json!("machine-1")),
        ("sequence", json!(7)),
        ("workingAgentCount", json!(1)),
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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

    let (managed, managed_signature) = signed_activity_contract();
    assert_eq!(
        managed,
        "{\"accountId\":\"acct-1\",\"activityChangedAt\":\"1970-01-01T00:00:01.000Z\",\"environmentId\":\"env-1\",\"kernelId\":\"kernel-1\",\"machineId\":\"machine-1\",\"sequence\":7,\"workingAgentCount\":1}"
    );
    assert_eq!(
        managed_signature,
        "sha256:9138aec2ee27abc792554b6a7480804f232e4cac12bcfed7bb1f01d15409afb4"
    );
}

#[test]
fn managed_activity_http_producer_keeps_timestamp_and_the_one_receiver_route() {
    let producer = include_str!("../../../../runtime/managed_kernel_activity.rs");
    for required in [
        "activity_changed_at: &'a str",
        "canonical_activity_timestamp",
        "chrono::SecondsFormat::Millis",
        "ACTIVITY_ENDPOINT: &str = \"/v1/kernels/activity\"",
    ] {
        assert!(
            producer.contains(required),
            "managed activity producer lost protocol-340 marker {required:?}"
        );
    }
    assert!(
        !producer.contains("disposable-workers"),
        "the disposable-worker activity route was removed"
    );
}
