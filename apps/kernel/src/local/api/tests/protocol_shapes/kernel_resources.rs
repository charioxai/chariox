use super::*;
use crate::local::*;
use crate::transport::kernel_protocol::KernelEvent;

// Protocol 1006: the kernel resource feed (docs/PROTOCOL.md §4.1.1).
#[test]
fn kernel_resources_protocol_1006_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let requests = serde_json::json!([{"WatchKernelResources":{"subscription_request_id":"sub-1","watches":[{"resource":"vault_status","params":{},"known_revision":null},{"resource":"app_installations","params":{"session_id":"room"},"known_revision":"r1"}]}}]);
    for request in requests.as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), *request);
    }
    // A watch without params or a known revision is a first visit.
    let bare: WatchKernelResourcesRequest = serde_json::from_value(
        serde_json::json!({"subscription_request_id":"s","watches":[{"resource":"vault_status"}]}),
    )
    .unwrap();
    assert_eq!(bare.watches[0].params, serde_json::json!({}));
    assert_eq!(bare.watches[0].known_revision, None);
    let responses = serde_json::json!([{"KernelResourcesWatched":{"refused":[{"resource":"room_list_v9","params":{},"code":"unknown_resource","message":"this kernel has no resource `room_list_v9`"}]}}]);
    for response in responses.as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *response);
    }
    let events = serde_json::json!([{"event":"kernel_resource_changed","resource":"vault_entries","params":{},"revision":"r2","body":{"snapshot":{"data":{"credentials":[]}}}},{"event":"kernel_resource_changed","resource":"vault_status","params":{},"revision":"r1","body":{"unchanged":{}}}]);
    for event in events.as_array().unwrap() {
        let parsed: KernelEvent = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *event);
    }
    let snapshot = serde_json::json!({"requests":requests,"responses":responses,"events":events});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "ae50764b9a16145a946d1ad8ce811fe73babb35045602493aac03b1684c36c7c"
    );
}
