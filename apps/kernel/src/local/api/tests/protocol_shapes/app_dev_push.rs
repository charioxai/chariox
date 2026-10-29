//! MP-08 / MP-10 / MP-11: D2 has one typed development entry point.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn app_dev_push_contract_requires_the_allocated_protocol() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 647);
    };
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let requests = [
        json!({"BeginAppDevPush":{"session_id":"room","request_id":"save-1","directory":"/work/todo"}}),
        json!({"GetAppDevPush":{"session_id":"room","request_id":"save-1"}}),
    ];
    for value in &requests {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let responses = [
        json!({"AppDevPushBuilding":{"request_id":"save-1"}}),
        json!({"AppDevPushFailed":{"request_id":"save-1","code":"source_unavailable","message":"Use files put to put the source in the kernel workspace."}}),
    ];
    for value in &responses {
        let response: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
    let snapshot = json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION,"requests":requests,"responses":responses});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("app-dev-push-647.json")).unwrap();
    assert_eq!(snapshot, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "ec1409a5c0a15df2d0226e38e37c8cb6d74a42ad6438a05c0273cfb3f928e1c1"
    );
    let forged = json!({"BeginAppDevPush":{"session_id":"room","request_id":"save-1","directory":"/work/todo","owner":"someone-else"}});
    assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
}
