//! GHA-5: the owner's standing permissions, the count on an installation and the preview rows of a confirmation card.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn app_standing_contract_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let requests = [
        json!({"GrantAppStanding":{"installation_id":"github","account":"work","repo":"charioxai/drill","class":"merge","expires_at_ms":1900000000000u64}}),
        json!({"GrantAppStanding":{"installation_id":"github","account":"work","repo":"charioxai/drill","class":"delete_branch"}}),
        json!({"RevokeAppStanding":{"installation_id":"github","standing_id":"standing-1"}}),
        json!({"ListAppStanding":{"installation_id":"github"}}),
    ];
    for value in &requests {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let responses = [
        json!({"AppStanding":{"installation_id":"github","standing":[
        {"standing_id":"standing-1","account":"work","repo":"charioxai/drill","class":"merge","created_at_ms":5,"expires_at_ms":9,"uses":2,"last_used_ms":7},
        {"standing_id":"standing-2","account":"work","repo":"charioxai/drill","class":"review","created_at_ms":5,"uses":0}]}}),
    ];
    for value in &responses {
        let response: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
    let frame = json!({"installation_id":"github","app_id":"com.chariox.github","app_name":"GitHub","publisher_id":"com.chariox","publisher_name":"Chariox","action_name":"merge_pull_request","input":"{\n  \"number\": 7\n}",
        "preview":[{"label":"Repository","value":"charioxai/drill"},{"label":"Method","value":"squash"}]});
    let parsed: AppConfirmationFrame = serde_json::from_value(frame.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), frame);
    let snapshot = json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION,"requests":requests,"responses":responses,"confirmation_frame":frame});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("app-standing-1011.json")).unwrap();
    assert_eq!(snapshot, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "13ab0ef51505d9ff13beb6a749f554047ebf244c62bd9e825b7b6727e532fec5"
    );
    // The closed classes: nothing for a force push, a default branch or a grant; and no field that names a value.
    for forged in [
        json!({"GrantAppStanding":{"installation_id":"github","account":"work","repo":"o/r","class":"force_push"}}),
        json!({"GrantAppStanding":{"installation_id":"github","account":"work","repo":"o/r","class":"merge","created_by":"agent"}}),
        json!({"RevokeAppStanding":{"installation_id":"github","standing_id":"s","owner":"someone-else"}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
