// MP-08 / MP-10 / MP-11: coordinator allocation 531/139.
use super::*;
use crate::local::NativeProviderTransportOperation;
use sha2::{Digest, Sha256};
#[test]
fn mp08_native_provider_transport_531_139_snapshot_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 531);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        139
    );
    let snapshot: serde_json::Value = serde_json::from_str(r###"{"local_protocol":531,"peer_protocol":139,"peer_requests":[{"attachment_id":"attachment","kind":"native_provider_transport","leased_agent_id":"leased","operation":{"kind":"open","request_id":"open-request"},"provider_run_id":"worker-run"},{"attachment_id":"attachment","kind":"native_provider_transport","leased_agent_id":"leased","operation":{"connection_id":"connection","frames":["{\"id\":1,\"method\":\"initialize\"}"],"kind":"exchange","sequence":1},"provider_run_id":"worker-run"},{"attachment_id":"attachment","kind":"native_provider_transport","leased_agent_id":"leased","operation":{"connection_id":"connection","kind":"close"},"provider_run_id":"worker-run"}],"requests":[{"NativeProviderTransport":{"agent_id":"agent","attachment_id":"attachment","operation":{"kind":"open","request_id":"open-request"},"provider_run_id":"run","session_id":"session"}},{"NativeProviderTransport":{"agent_id":"agent","attachment_id":"attachment","operation":{"connection_id":"connection","frames":["{\"id\":1,\"method\":\"initialize\"}"],"kind":"exchange","sequence":1},"provider_run_id":"run","session_id":"session"}},{"NativeProviderTransport":{"agent_id":"agent","attachment_id":"attachment","operation":{"connection_id":"connection","kind":"close"},"provider_run_id":"run","session_id":"session"}}],"response":{"NativeProviderTransport":{"result":{"closed":false,"connection_id":"connection","frames":[],"sequence":1}}}}"###).unwrap();
    for value in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    for value in snapshot["peer_requests"].as_array().unwrap() {
        let request: crate::transport::relay_peer::RelayPeerRequest =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "bcc1899953bf49b88101f719d62cd5d5acdbf9e7526c3ee235ddbe1b453d6263"
    );
    let mut arbitrary_endpoint = snapshot["requests"][0].clone();
    arbitrary_endpoint["NativeProviderTransport"]["endpoint"] =
        serde_json::json!("ws://attacker:1234");
    assert!(serde_json::from_value::<LocalDaemonRequest>(arbitrary_endpoint).is_err());
}

// MP-08 / MP-10 / MP-11: old, unreceipted writes must fail before execution.
#[test]
fn mp08_native_provider_transport_refuses_unreceipted_predecessor_operations() {
    for operation in [
        serde_json::json!({"kind":"open"}),
        serde_json::json!({"kind":"exchange","connection_id":"connection","frames":[]}),
    ] {
        assert!(
            serde_json::from_value::<NativeProviderTransportOperation>(operation.clone()).is_err()
        );
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
                "NativeProviderTransport": {"session_id":"session", "attachment_id":"attachment",
                    "agent_id":"agent", "provider_run_id":"run", "operation":operation.clone()}
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<crate::transport::relay_peer::RelayPeerRequest>(serde_json::json!({
            "kind":"native_provider_transport", "leased_agent_id":"leased", "attachment_id":"attachment",
            "provider_run_id":"run", "operation":operation
        })).is_err());
    }
}
