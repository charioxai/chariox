//! MP-08 / MP-09 / MP-10 / MP-11: the Landing 8 shapes at the current local / relay pair, from the
//! fixture the kernel-client test reads too (`landing8-protocol.test.ts`).
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RELAY_PEER_PROTOCOL_VERSION};

#[test]
fn landing8_1003_442_shapes_round_trip() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/landing8-protocol.json"
    ))
    .unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(fixture["relayPeer"], RELAY_PEER_PROTOCOL_VERSION);

    let request: LocalDaemonRequest = serde_json::from_value(fixture["hideAgent"].clone()).unwrap();
    assert!(matches!(&request, LocalDaemonRequest::HideAgent(hide) if hide.hidden));
    assert_eq!(serde_json::to_value(request).unwrap(), fixture["hideAgent"]);

    let trigger: crate::session::AgentTrigger =
        serde_json::from_value(fixture["notificationTrigger"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(trigger).unwrap(),
        fixture["notificationTrigger"]
    );

    let prompt: crate::local::PasskeyPrompt =
        serde_json::from_value(fixture["appInstallPrompt"].clone()).unwrap();
    assert_eq!(prompt.kind, crate::local::PasskeyPromptKind::AppInstall);
    assert_eq!(prompt.expires_at_ms, crate::local::NEVER_EXPIRES_MS);
    assert_eq!(
        serde_json::to_value(prompt).unwrap(),
        fixture["appInstallPrompt"]
    );

    for wire in fixture["relayPeerRequests"].as_array().unwrap() {
        let request: RelayPeerRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *wire);
    }
    for wire in fixture["retiredRequests"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(wire.clone()).is_err(),
            "{wire} is retired"
        );
    }
}
