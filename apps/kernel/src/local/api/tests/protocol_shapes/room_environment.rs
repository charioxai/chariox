use super::*;
use crate::session::{
    CanonicalViewport, EnvironmentComponent, EnvironmentComponentHealth,
    EnvironmentComponentHealthState, EnvironmentLifecycle, RoomEnvironmentSnapshot,
};

#[test]
fn room_environment_state_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 269);

    let request = LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
        session_id: "session-1".to_string(),
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment state request should encode"),
        serde_json::json!({
            "GetRoomEnvironmentState": {
                "session_id": "session-1"
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetRoomEnvironmentState": {
                "session_id": "session-1"
            }
        }))
        .expect("Room Environment state request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentState {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Stopped,
            health: vec![EnvironmentComponentHealth {
                component: EnvironmentComponent::BrowserController,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            }],
            viewport: CanonicalViewport::new(1280, 800, 2, 2560, 1600)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            event_cursor: 0,
        },
    };
    assert_eq!(
        serde_json::to_value(response).expect("Room Environment state response should encode"),
        serde_json::json!({
            "RoomEnvironmentState": {
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-1",
                    "runtime_generation": 1,
                    "lifecycle": "stopped",
                    "health": [{
                        "component": "browser_controller",
                        "state": "unavailable",
                        "diagnostic_code": null
                    }],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 2,
                        "desktop_pixel_width": 2560,
                        "desktop_pixel_height": 1600,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [],
                    "tabs": [],
                    "focused_tab_id": null,
                    "actions": [],
                    "input_ownership": [],
                    "event_cursor": 0
                }
            }
        })
    );
}
