//! MP-08/MP-10/MP-11 credfix: a worker's refusal reason reaches the home with the prompt receipt.
use super::*;
use crate::transport::relay_peer::{
    LeasedPromptReceipt, LeasedPromptReceiptPhase, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

#[test]
fn leased_prompt_rejection_reason_is_versioned_hashed_and_optional() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let snapshot = serde_json::json!({
        "local_protocol": LOCAL_DAEMON_PROTOCOL_VERSION,
        "peer_protocol": RELAY_PEER_PROTOCOL_VERSION,
        "responses": [
            {"kind": "leased_prompt_receipt_queried", "receipt": {
                "home_prompt_id": "p1", "worker_provider_run_id": "",
                "phase": "steer_rejected", "execution_lease_id": "l1"}},
            {"kind": "leased_prompt_receipt_queried", "receipt": {
                "home_prompt_id": "p1", "worker_provider_run_id": "",
                "phase": "steer_rejected", "execution_lease_id": "l1",
                "rejection_reason": "selected codex account `x` is not authenticated"}}
        ]
    });
    let mut reasons = Vec::new();
    for response in snapshot["responses"].as_array().unwrap() {
        let typed: RelayPeerResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), *response);
        let RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt:
                Some(LeasedPromptReceipt {
                    phase: LeasedPromptReceiptPhase::SteerRejected,
                    rejection_reason,
                    ..
                }),
        } = typed
        else {
            panic!("expected a rejected prompt receipt");
        };
        reasons.push(rejection_reason);
    }
    assert_eq!(reasons[0], None);
    assert!(reasons[1].as_deref().unwrap().contains("not authenticated"));
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "7dec679dae50ed2f849da382b64e82a47881e6e6aa105f10997fdf8426394ae1"
    );
}
