//! GHA-2: an App routes events for itself; the owner can pause a route or an automation.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn app_event_routing_contract_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let requests = [
        json!({"SetAppInboxRoutePaused":{"installation_id":"github","route_id":"o-r-pull_request","paused":true}}),
        json!({"PauseAppAutomation":{"installation_id":"github","automation_id":"wf_1","expected_revision":2}}),
    ];
    for value in &requests {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let responses = [
        json!({"AppInboxRoutes":{"installation_id":"github","routes":[{
            "route_id":"o-r-pull_request","event_name":"pull_request","source_event_type":"pull_request.opened",
            "source_event_version":1,"active":true,"paused":true,
            "connection":{"generator_id":"dev.chariox.github","connection_id":"connection-1","connection_scope":"o/r"},
            "pending":0,"delivered":3,"failed":0,"expired":0}]}}),
        json!({"AppConnections":{"installation_id":"github","connections":[{
            "generator_id":"dev.chariox.github","connection_id":"connection-1","granted_at_ms":5,"actions":[],
            "event_routing":{"max_routes":200,"routes":4,"paused":1}}]}}),
    ];
    for value in &responses {
        let response: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
    let snapshot = json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION,"requests":requests,"responses":responses});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("app-event-routing-1011.json")).unwrap();
    assert_eq!(snapshot, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "1d931c926d279297e714cb1cfc68418c0e62948c5f8a0a43af5c2ea580fa75a2"
    );
    // Neither request names a connection, owner or agent: the kernel decides those.
    for forged in [
        json!({"SetAppInboxRoutePaused":{"installation_id":"github","route_id":"r","paused":true,"owner":"x"}}),
        json!({"PauseAppAutomation":{"installation_id":"github","automation_id":"a","expected_revision":-1}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
