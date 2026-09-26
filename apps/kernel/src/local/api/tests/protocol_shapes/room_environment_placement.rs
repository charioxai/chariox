use super::*;
use crate::local::{
    BindRoomEnvironmentSliceRequest, GetRoomEnvironmentResourceInventoryRequest,
    RoomEnvironmentResourceInventory, RoomEnvironmentSliceBinding,
};

#[test]
fn room_environment_placement_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 349);
    let request = LocalDaemonRequest::BindRoomEnvironmentSlice(BindRoomEnvironmentSliceRequest {
        session_id: "session-1".into(),
        slice_ref: "desktop".into(),
    });
    let wire = serde_json::json!({"BindRoomEnvironmentSlice": {
        "session_id": "session-1", "slice_ref": "desktop"
    }});
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(wire).unwrap(),
        request
    );
    let response = LocalDaemonResponse::RoomEnvironmentSlice {
        binding: Some(RoomEnvironmentSliceBinding {
            session_id: "session-1".into(),
            slice_id: "slice-1".into(),
            owner_kernel_id: "home".into(),
            worker_kernel_ref: "slice:desktop".into(),
        }),
    };
    let wire = serde_json::json!({"RoomEnvironmentSlice":{"binding":{
        "session_id":"session-1", "slice_id":"slice-1",
        "owner_kernel_id":"home", "worker_kernel_ref":"slice:desktop"
    }}});
    assert_eq!(serde_json::to_value(&response).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(wire).unwrap(),
        response
    );
    assert_eq!(
        serde_json::to_value(LocalDaemonResponse::RoomEnvironmentSlice { binding: None }).unwrap(),
        serde_json::json!({"RoomEnvironmentSlice":{"binding":null}})
    );

    let inventory_request = LocalDaemonRequest::GetRoomEnvironmentResourceInventory(
        GetRoomEnvironmentResourceInventoryRequest {
            session_id: "session-1".into(),
            slice_id: "slice-1".into(),
        },
    );
    let inventory_request_wire = serde_json::json!({"GetRoomEnvironmentResourceInventory": {
        "session_id": "session-1", "slice_id": "slice-1"
    }});
    assert_eq!(
        serde_json::to_value(&inventory_request).unwrap(),
        inventory_request_wire
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(inventory_request_wire).unwrap(),
        inventory_request
    );

    let inventory_response = LocalDaemonResponse::RoomEnvironmentResourceInventory {
        inventory: RoomEnvironmentResourceInventory {
            session_id: "session-1".into(),
            environment_id: "environment-1".into(),
            slice_id: "slice-1".into(),
            browser_ids: vec!["browser-pid-7".into()],
            profile_ids: vec!["profile-sha256-41".into()],
        },
    };
    let inventory_response_wire = serde_json::json!({"RoomEnvironmentResourceInventory": {
        "inventory": {
            "session_id": "session-1", "environment_id": "environment-1",
            "slice_id": "slice-1",
            "browser_ids": ["browser-pid-7"],
            "profile_ids": ["profile-sha256-41"]
        }
    }});
    assert_eq!(
        serde_json::to_value(&inventory_response).unwrap(),
        inventory_response_wire
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(inventory_response_wire).unwrap(),
        inventory_response
    );
}
