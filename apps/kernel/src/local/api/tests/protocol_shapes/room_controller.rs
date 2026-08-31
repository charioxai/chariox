use super::*;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;

#[test]
fn room_controller_protocol_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 287);
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 23);
    for (command, wire_command) in [
        (
            RoomBrowserControllerCommand::Action {
                execution_id: "11111111111111111111111111111111".into(),
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                action: crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text: "sensitive-fill-fixture".into(),
                    append: false,
                    submit: false,
                },
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"action","execution_id":"11111111111111111111111111111111","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","action":{"kind":"fill","text":"sensitive-fill-fixture",
                "append":false,"submit":false},"timeout_ms":500}),
        ),
        (
            RoomBrowserControllerCommand::CancelAction {
                execution_id: "11111111111111111111111111111111".into(),
            },
            serde_json::json!({"kind":"cancel_action","execution_id":"11111111111111111111111111111111"}),
        ),
        (
            RoomBrowserControllerCommand::Snapshot {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
            },
            serde_json::json!({"kind":"snapshot","target_id":"target-1","document_id":"doc-1"}),
        ),
        (
            RoomBrowserControllerCommand::Acquire,
            serde_json::json!({"kind":"acquire"}),
        ),
        (
            RoomBrowserControllerCommand::Release,
            serde_json::json!({"kind":"release"}),
        ),
        (
            RoomBrowserControllerCommand::Reconcile {
                viewport: crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
            },
            serde_json::json!({"kind":"reconcile","viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,
                "desktop_pixel_width":1280,"desktop_pixel_height":800,
                "revision":1,"last_actor_id":null
            }}),
        ),
    ] {
        let request = RelayPeerRequest::RoomBrowserController {
            session_id: "room-1".into(),
            slice_id: "slice-1".into(),
            command,
        };
        let wire = serde_json::json!({"kind":"room_browser_controller", "session_id":"room-1",
            "slice_id":"slice-1","command":wire_command});
        assert!(
            !format!("{request:?}").contains("sensitive-fill-fixture"),
            "relay diagnostics must not print fill payloads"
        );
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(wire).unwrap(),
            request
        );
    }
    for result in [
        serde_json::json!({"kind":"cancellation_requested","accepted":true}),
        serde_json::json!({"kind":"cancellation_requested","accepted":false}),
        serde_json::json!({"kind":"action_cancelled","controller_fenced":false,
            "controller_restarted":false}),
        serde_json::json!({"kind":"action_cancelled","controller_fenced":true,
            "controller_restarted":true}),
        serde_json::json!({"kind":"action","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "action_kind":"click","dialog_opened":false,"attempts":2,"elapsed_ms":50
        }}),
        serde_json::json!({"kind":"snapshot","snapshot":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "snapshot_revision":2,"accessibility_nodes":[],"dom_documents":[],"shadow_roots":[],
            "dom_nodes":[{"node_ref":"backend:1","parent_ref":null,"node_type":1,"node_name":"BUTTON",
                "text":"","attributes":{},"bounds":{"x":1.5,"y":2.0,"width":3.0,"height":4.0}}]
        }}),
        serde_json::json!({"kind":"process","snapshot":{
            "state":"ready","process_id":123,"diagnostic_code":null,
            "runtime_generation":2,"restart_count":1
        }}),
        serde_json::json!({"kind":"reconciled","reconciliation":{
            "process":{"state":"ready","process_id":123,"diagnostic_code":null,
                "runtime_generation":2,"restart_count":1},
            "browser":{"browser_generation":3,"event_cursor":4,
                "tabs":[{"target_id":"target-1","document_id":"doc-1",
                    "url":"https://example.test/","title":"Example"}],
                "focused_target_id":"target-1","viewport":{
                    "css_width":1280,"css_height":800,"device_scale_factor":1,
                    "desktop_pixel_width":1280,"desktop_pixel_height":800}}
        }}),
    ] {
        let wire = serde_json::json!({"kind":"room_browser_controller", "session_id":"room-1",
            "slice_id":"slice-1","result":result});
        let response: RelayPeerResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), wire);
    }
}
