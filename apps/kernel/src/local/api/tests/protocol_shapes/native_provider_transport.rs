// MP-08 / MP-10 / MP-11: coordinator allocation 526/134.
use super::*;
use sha2::{Digest, Sha256};
#[test]
fn mp08_native_provider_transport_526_134_snapshot_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 526);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        134
    );
    let snapshot: serde_json::Value = serde_json::from_str(r###"{"local_protocol":526,"peer_protocol":134,"peer_requests":[{"attachment_id":"attachment","kind":"native_provider_transport","leased_agent_id":"leased","operation":{"kind":"open"},"provider_run_id":"worker-run"},{"attachment_id":"attachment","kind":"native_provider_transport","leased_agent_id":"leased","operation":{"connection_id":"connection","frames":["{\"id\":1,\"method\":\"initialize\"}"],"kind":"exchange"},"provider_run_id":"worker-run"},{"attachment_id":"attachment","kind":"native_provider_transport","leased_agent_id":"leased","operation":{"connection_id":"connection","kind":"close"},"provider_run_id":"worker-run"}],"requests":[{"NativeProviderTransport":{"agent_id":"agent","attachment_id":"attachment","operation":{"kind":"open"},"provider_run_id":"run","session_id":"session"}},{"NativeProviderTransport":{"agent_id":"agent","attachment_id":"attachment","operation":{"connection_id":"connection","frames":["{\"id\":1,\"method\":\"initialize\"}"],"kind":"exchange"},"provider_run_id":"run","session_id":"session"}},{"NativeProviderTransport":{"agent_id":"agent","attachment_id":"attachment","operation":{"connection_id":"connection","kind":"close"},"provider_run_id":"run","session_id":"session"}}],"response":{"NativeProviderTransport":{"result":{"closed":false,"connection_id":"connection","frames":[]}}}}"###).unwrap();
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
        "bd149b0ead0a70f99c219509fcf3ed7a4418cfe90e30c302ba6c0660ca64f742"
    );
    let mut arbitrary_endpoint = snapshot["requests"][0].clone();
    arbitrary_endpoint["NativeProviderTransport"]["endpoint"] =
        serde_json::json!("ws://attacker:1234");
    assert!(serde_json::from_value::<LocalDaemonRequest>(arbitrary_endpoint).is_err());
}
