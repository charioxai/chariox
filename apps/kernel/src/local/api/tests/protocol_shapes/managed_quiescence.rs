use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use super::*;

const MACHINE_CREDENTIAL: &str = "mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NONCE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const IDLE_DEADLINE_AT: &str = "2026-09-26T00:00:00.000Z";

fn signed_payload(fields: BTreeMap<&'static str, Value>) -> (String, String, String) {
    let canonical = serde_json::to_string(&fields).expect("quiescence fields should serialize");
    let mut mac = Hmac::<Sha256>::new_from_slice(MACHINE_CREDENTIAL.as_bytes())
        .expect("machine credential should initialize HMAC");
    mac.update(canonical.as_bytes());
    let signature = format!("sha256:{:x}", mac.finalize().into_bytes());

    let mut payload = fields;
    payload.insert("machineCredential", json!(MACHINE_CREDENTIAL));
    payload.insert("signature", json!(signature));
    let payload = serde_json::to_string(&payload).expect("signed quiescence payload should serialize");
    (canonical, signature, payload)
}

fn assert_signed_fixture(
    fields: BTreeMap<&'static str, Value>,
    expected_canonical: &str,
    expected_signature: &str,
    expected_payload: &str,
) {
    let (canonical, signature, payload) = signed_payload(fields);
    assert_eq!(canonical, expected_canonical);
    assert_eq!(signature, expected_signature);
    assert_eq!(payload, expected_payload);
}

#[test]
fn managed_quiescence_contract_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 350);
}

#[test]
fn managed_quiescence_poll_request_matches_canonical_v1_json_and_hmac_fixture() {
    assert_signed_fixture(
        BTreeMap::from([
            ("accountId", json!("account-1")),
            ("action", json!("auto_stop_quiescence_poll")),
            ("environmentId", json!("environment-1")),
            ("kernelId", json!("kernel-1")),
            ("machineId", json!("machine-1")),
            ("protocolVersion", json!(1)),
        ]),
        r#"{"accountId":"account-1","action":"auto_stop_quiescence_poll","environmentId":"environment-1","kernelId":"kernel-1","machineId":"machine-1","protocolVersion":1}"#,
        "sha256:2d8160d958ced5fd810ab82baf032d91ff72fff9c1324a04d6acf4aac01b4005",
        r#"{"accountId":"account-1","action":"auto_stop_quiescence_poll","environmentId":"environment-1","kernelId":"kernel-1","machineCredential":"mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","machineId":"machine-1","protocolVersion":1,"signature":"sha256:2d8160d958ced5fd810ab82baf032d91ff72fff9c1324a04d6acf4aac01b4005"}"#,
    );
}

#[test]
fn managed_quiescence_reservation_ack_matches_canonical_v1_json_and_hmac_fixture() {
    assert_signed_fixture(
        BTreeMap::from([
            ("accountId", json!("account-1")),
            ("action", json!("auto_stop_quiescence_ack")),
            ("challengeId", json!("challenge-1")),
            ("decision", json!("fenced")),
            ("desiredRevision", json!(4)),
            ("environmentId", json!("environment-1")),
            ("idleDeadlineAt", json!(IDLE_DEADLINE_AT)),
            ("idleSequence", json!(8)),
            ("kernelId", json!("kernel-1")),
            ("machineId", json!("machine-1")),
            ("nonce", json!(NONCE)),
            ("protocolVersion", json!(1)),
            ("stopOperationId", json!("stop-operation-1")),
        ]),
        r#"{"accountId":"account-1","action":"auto_stop_quiescence_ack","challengeId":"challenge-1","decision":"fenced","desiredRevision":4,"environmentId":"environment-1","idleDeadlineAt":"2026-09-26T00:00:00.000Z","idleSequence":8,"kernelId":"kernel-1","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","protocolVersion":1,"stopOperationId":"stop-operation-1"}"#,
        "sha256:d1200d83260be01c766316b0f661ce7607969fafd409e540ffa68dd2c5253bd2",
        r#"{"accountId":"account-1","action":"auto_stop_quiescence_ack","challengeId":"challenge-1","decision":"fenced","desiredRevision":4,"environmentId":"environment-1","idleDeadlineAt":"2026-09-26T00:00:00.000Z","idleSequence":8,"kernelId":"kernel-1","machineCredential":"mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","protocolVersion":1,"signature":"sha256:d1200d83260be01c766316b0f661ce7607969fafd409e540ffa68dd2c5253bd2","stopOperationId":"stop-operation-1"}"#,
    );
}

#[test]
fn managed_quiescence_release_ack_reuses_canonical_v1_json_and_hmac_fixture() {
    // Keep these exact values in sync with the kernel runtime's existing
    // cloud_release_ack_matches_canonical_v1_payload_and_hmac_fixture.
    assert_signed_fixture(
        BTreeMap::from([
            ("accountId", json!("account-1")),
            ("action", json!("auto_stop_quiescence_release_ack")),
            ("challengeId", json!("challenge-1")),
            ("environmentId", json!("environment-1")),
            ("kernelId", json!("kernel-1")),
            ("machineId", json!("machine-1")),
            ("nonce", json!(NONCE)),
            ("outcome", json!("keep_running")),
            ("protocolVersion", json!(1)),
            ("resultSequence", json!(1)),
            ("stopOperationId", json!("stop-operation-1")),
        ]),
        r#"{"accountId":"account-1","action":"auto_stop_quiescence_release_ack","challengeId":"challenge-1","environmentId":"environment-1","kernelId":"kernel-1","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","outcome":"keep_running","protocolVersion":1,"resultSequence":1,"stopOperationId":"stop-operation-1"}"#,
        "sha256:d2a9120425277132c10100e4c96ab3ee8172ae79ff82f8296ca6cae8c99502ec",
        r#"{"accountId":"account-1","action":"auto_stop_quiescence_release_ack","challengeId":"challenge-1","environmentId":"environment-1","kernelId":"kernel-1","machineCredential":"mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","outcome":"keep_running","protocolVersion":1,"resultSequence":1,"signature":"sha256:d2a9120425277132c10100e4c96ab3ee8172ae79ff82f8296ca6cae8c99502ec","stopOperationId":"stop-operation-1"}"#,
    );
}

#[test]
fn managed_quiescence_poll_commands_pin_reservation_and_release_shapes() {
    let reserve = json!({
        "protocolVersion": 1,
        "command": {
            "kind": "reserve_idle_for_stop",
            "challengeId": "challenge-1",
            "accountId": "account-1",
            "environmentId": "environment-1",
            "machineId": "machine-1",
            "kernelId": "kernel-1",
            "desiredRevision": 4,
            "idleSequence": 8,
            "idleDeadlineAt": IDLE_DEADLINE_AT,
            "stopOperationId": "stop-operation-1",
            "nonce": NONCE,
        }
    });
    assert_eq!(
        serde_json::to_string(&reserve).expect("reservation command should serialize"),
        r#"{"command":{"accountId":"account-1","challengeId":"challenge-1","desiredRevision":4,"environmentId":"environment-1","idleDeadlineAt":"2026-09-26T00:00:00.000Z","idleSequence":8,"kernelId":"kernel-1","kind":"reserve_idle_for_stop","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","stopOperationId":"stop-operation-1"},"protocolVersion":1}"#,
    );

    let release = json!({
        "protocolVersion": 1,
        "command": {
            "kind": "release_admission_fence",
            "challengeId": "challenge-1",
            "accountId": "account-1",
            "environmentId": "environment-1",
            "machineId": "machine-1",
            "kernelId": "kernel-1",
            "desiredRevision": 4,
            "idleSequence": 8,
            "idleDeadlineAt": IDLE_DEADLINE_AT,
            "stopOperationId": "stop-operation-1",
            "nonce": NONCE,
            "outcome": "keep_running",
            "resultSequence": 1,
        }
    });
    assert_eq!(
        serde_json::to_string(&release).expect("release command should serialize"),
        r#"{"command":{"accountId":"account-1","challengeId":"challenge-1","desiredRevision":4,"environmentId":"environment-1","idleDeadlineAt":"2026-09-26T00:00:00.000Z","idleSequence":8,"kernelId":"kernel-1","kind":"release_admission_fence","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","outcome":"keep_running","resultSequence":1,"stopOperationId":"stop-operation-1"},"protocolVersion":1}"#,
    );
}
