use super::*;
use crate::local::RoomEnvironmentScreenshotChunk;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RemoteExtensionInvocationContext,
    RemoteRoomBrowserRuntimeToolCall, RemoteRoomBrowserRuntimeToolResult,
    RELAY_PEER_PROTOCOL_VERSION,
};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
use crate::transport::room_browser_controller::{
    RoomComputerInputAction, RoomComputerPointerButton,
};

#[test]
fn room_screenshot_peer_protocol_is_bounded_and_versioned() {
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 35);

    let request = RelayPeerRequest::ReadRoomScreenshotChunk {
        session_id: "session-1".to_string(),
        slice_id: "slice-1".to_string(),
        artifact_id: "artifact-1".to_string(),
        offset: 131_072,
        max_bytes: 131_072,
    };
    assert_eq!(
        serde_json::to_value(&request).expect("Room screenshot peer request should encode"),
        serde_json::json!({
            "kind": "read_room_screenshot_chunk",
            "session_id": "session-1",
            "slice_id": "slice-1",
            "artifact_id": "artifact-1",
            "offset": 131072,
            "max_bytes": 131072
        })
    );

    let response = RelayPeerResponse::RoomScreenshotChunk {
        session_id: "session-1".to_string(),
        slice_id: "slice-1".to_string(),
        chunk: RoomEnvironmentScreenshotChunk {
            artifact_id: "artifact-1".to_string(),
            offset: 0,
            data_base64: "YWJj".to_string(),
            eof: false,
        },
    };
    let response_value =
        serde_json::to_value(&response).expect("Room screenshot peer response should encode");
    assert_eq!(
        serde_json::from_value::<RelayPeerResponse>(response_value)
            .expect("Room screenshot peer response should decode"),
        response
    );
}

#[test]
fn room_controller_protocol_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 299);
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 35);
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
                    expected_document_url: Some("https://example.test/login".into()),
                },
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"action","execution_id":"11111111111111111111111111111111","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","action":{"kind":"fill","text":"sensitive-fill-fixture",
                "append":false,"submit":false,"expected_document_url":"https://example.test/login"},"timeout_ms":500}),
        ),
        (
            RoomBrowserControllerCommand::RecoverAction {
                execution_id: "11111111111111111111111111111111".into(),
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                action: crate::runtime::browser_controller_action::BrowserLocatorAction::Fill {
                    text: "sensitive-fill-fixture".into(),
                    append: false,
                    submit: false,
                    expected_document_url: None,
                },
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"recover_action","execution_id":"11111111111111111111111111111111","target_id":"target-1","document_id":"doc-1",
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
            RoomBrowserControllerCommand::ComputerInput {
                action_id: "action-7".into(),
                actor_id: "user:owner-1".into(),
                runtime_generation: 4,
                viewport_revision: 9,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action: RoomComputerInputAction::PointerClick {
                    x: 320,
                    y: 180,
                    button: RoomComputerPointerButton::Right,
                    click_count: 2,
                },
            },
            serde_json::json!({
                "kind":"computer_input",
                "action_id":"action-7",
                "actor_id":"user:owner-1",
                "runtime_generation":4,
                "viewport_revision":9,
                "desktop_pixel_width":1280,
                "desktop_pixel_height":800,
                "action":{"kind":"pointer_click","x":320,"y":180,"button":"right","click_count":2}
            }),
        ),
        (
            RoomBrowserControllerCommand::Snapshot {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
            },
            serde_json::json!({"kind":"snapshot","target_id":"target-1","document_id":"doc-1"}),
        ),
        (
            RoomBrowserControllerCommand::Navigate {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                url: crate::runtime::browser_controller_compatibility::BrowserNavigationUrl::new(
                    "https://example.test/path?sensitive-navigation-fixture",
                )
                .unwrap(),
            },
            serde_json::json!({"kind":"navigate","target_id":"target-1","document_id":"doc-1",
                "url":"https://example.test/path?sensitive-navigation-fixture"}),
        ),
        (
            RoomBrowserControllerCommand::Wait {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                wait: crate::runtime::browser_controller_compatibility::BrowserCompatibilityWait::Selector(
                    "sensitive-selector-fixture".into(),
                ),
                timeout_ms: 500,
            },
            serde_json::json!({"kind":"wait","target_id":"target-1","document_id":"doc-1",
                "wait":{"kind":"selector","selector":"sensitive-selector-fixture"},"timeout_ms":500}),
        ),
        (
            RoomBrowserControllerCommand::Dialog {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                action: crate::runtime::browser_controller_action::BrowserDialogAction::Accept {
                    prompt_text: Some("sensitive-dialog-fixture".into()),
                },
            },
            serde_json::json!({"kind":"dialog","target_id":"target-1","document_id":"doc-1",
                "action":{"kind":"accept","prompt_text":"sensitive-dialog-fixture"}}),
        ),
        (
            RoomBrowserControllerCommand::ConfigureDownloads {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
            },
            serde_json::json!({"kind":"configure_downloads","target_id":"target-1","document_id":"doc-1"}),
        ),
        (
            RoomBrowserControllerCommand::Upload {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                node_ref: "backend:1".into(),
                files: crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(
                    vec!["/workspace/sensitive-upload-fixture".into()],
                )
                .unwrap(),
            },
            serde_json::json!({"kind":"upload","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","files":["/workspace/sensitive-upload-fixture"]}),
        ),
        (
            RoomBrowserControllerCommand::Permission {
                target_id: "target-1".into(),
                document_id: "doc-1".into(),
                permission: crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
                setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
            },
            serde_json::json!({"kind":"permission","target_id":"target-1","document_id":"doc-1",
                "permission":"geolocation","setting":"denied"}),
        ),
        (
            RoomBrowserControllerCommand::PollEvents {
                browser_generation: 3,
                cursor: 4,
                limit: 20,
            },
            serde_json::json!({"kind":"poll_events","browser_generation":3,"cursor":4,"limit":20}),
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
        assert!(
            !format!("{request:?}").contains("sensitive-dialog-fixture"),
            "relay diagnostics must not print dialog prompt payloads"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-upload-fixture"),
            "relay diagnostics must not print upload paths"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-navigation-fixture"),
            "relay diagnostics must not print navigation URLs"
        );
        assert!(
            !format!("{request:?}").contains("sensitive-selector-fixture"),
            "relay diagnostics must not print compatibility selectors"
        );
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(wire).unwrap(),
            request
        );
    }
    for result in [
        serde_json::json!({"kind":"recovery_required","process":{
            "state":"ready","process_id":124,"diagnostic_code":null,
            "runtime_generation":3,"restart_count":2
        }}),
        serde_json::json!({"kind":"cancellation_requested","accepted":true}),
        serde_json::json!({"kind":"cancellation_requested","accepted":false}),
        serde_json::json!({"kind":"action_cancelled","controller_fenced":false}),
        serde_json::json!({"kind":"action_cancelled","controller_fenced":true}),
        serde_json::json!({"kind":"action","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "action_kind":"click","dialog_opened":false,"attempts":2,"elapsed_ms":50
        }}),
        serde_json::json!({"kind":"computer_input_applied","action_id":"action-7"}),
        serde_json::json!({"kind":"snapshot","snapshot":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "snapshot_revision":2,"accessibility_nodes":[],"dom_documents":[{
                "document_index":0,"url":"https://example.test/","owner_node_ref":null
            }],"shadow_roots":[],
            "dom_nodes":[{"node_ref":"backend:1","parent_ref":null,"document_index":0,"node_type":1,"node_name":"BUTTON",
                "text":"","attributes":{},"bounds":{"x":1.5,"y":2.0,"width":3.0,"height":4.0}}]
        }}),
        serde_json::json!({"kind":"navigation","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-2",
            "url":"https://example.test/path?sensitive-navigation-result"
        }}),
        serde_json::json!({"kind":"wait","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "kind":"selector","ok":true,"elapsed_ms":7
        }}),
        serde_json::json!({"kind":"dialog","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1","action":"dismiss"
        }}),
        serde_json::json!({"kind":"downloads","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1","enabled":true
        }}),
        serde_json::json!({"kind":"upload","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1","file_count":1,"total_bytes":12
        }}),
        serde_json::json!({"kind":"permission","result":{
            "browser_generation":1,"target_id":"target-1","document_id":"doc-1",
            "permission":"geolocation","setting":"denied"
        }}),
        serde_json::json!({"kind":"events","batch":{
            "browser_generation":3,
            "events":[{"event_id":5,"browser_generation":3,"kind":"network_request",
                "target_id":"target-1","document_id":"doc-1","data":{
                    "request_id":"sensitive-event-fixture","method":"GET","url":"https://example.test/path",
                    "resource_type":"Document"
                }}],
            "next_cursor":5,"replay_gap":false
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
        assert!(
            !format!("{response:?}").contains("sensitive-event-fixture"),
            "relay diagnostics must not print browser event data values"
        );
        assert!(
            !format!("{response:?}").contains("sensitive-navigation-result"),
            "relay diagnostics must not print navigation result URLs"
        );
        assert_eq!(serde_json::to_value(response).unwrap(), wire);
    }
    assert!(
        serde_json::from_value::<RelayPeerRequest>(serde_json::json!({
            "kind":"room_browser_controller", "session_id":"room-1", "slice_id":"slice-1",
            "command":{"kind":"upload","target_id":"target-1","document_id":"doc-1",
                "node_ref":"backend:1","files":["relative-path"]}
        }))
        .is_err()
    );

    let context = RemoteExtensionInvocationContext {
        home_kernel_id: "home-kernel".into(),
        home_session_id: "room-1".into(),
        home_agent_id: "agent-1".into(),
        leased_agent_id: "leased-agent-1".into(),
        worker_provider_run_id: "worker-run-1".into(),
        worker_kernel_id: Some("worker-kernel".into()),
        worker_machine_id: Some("worker-machine".into()),
    };
    let forwarded = RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
        context: context.clone(),
        call: RemoteRoomBrowserRuntimeToolCall {
            tool_name: "slice_open_url".into(),
            arguments: serde_json::json!({
                "url":"https://sensitive-worker-forward.test/path?token=secret"
            }),
        },
    };
    let forwarded_wire = serde_json::json!({
        "kind":"forward_room_browser_runtime_tool",
        "context":{
            "home_kernel_id":"home-kernel",
            "home_session_id":"room-1",
            "home_agent_id":"agent-1",
            "leased_agent_id":"leased-agent-1",
            "worker_provider_run_id":"worker-run-1",
            "worker_kernel_id":"worker-kernel",
            "worker_machine_id":"worker-machine"
        },
        "call":{
            "tool_name":"slice_open_url",
            "arguments":{"url":"https://sensitive-worker-forward.test/path?token=secret"}
        }
    });
    assert_eq!(serde_json::to_value(&forwarded).unwrap(), forwarded_wire);
    assert_eq!(
        serde_json::from_value::<RelayPeerRequest>(forwarded_wire).unwrap(),
        forwarded
    );
    assert!(!format!("{forwarded:?}").contains("sensitive-worker-forward"));

    let handled = RelayPeerResponse::RoomBrowserRuntimeToolHandled {
        result: RemoteRoomBrowserRuntimeToolResult(
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "url":"https://sensitive-worker-result.test/path?token=secret"
                }),
            },
        ),
    };
    let handled_wire = serde_json::json!({
        "kind":"room_browser_runtime_tool_handled",
        "result":{
            "ok":true,
            "payload":{"url":"https://sensitive-worker-result.test/path?token=secret"}
        }
    });
    assert_eq!(serde_json::to_value(&handled).unwrap(), handled_wire);
    assert_eq!(
        serde_json::from_value::<RelayPeerResponse>(handled_wire).unwrap(),
        handled
    );
    assert!(!format!("{handled:?}").contains("sensitive-worker-result"));
}
