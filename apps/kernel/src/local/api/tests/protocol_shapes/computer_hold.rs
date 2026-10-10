//! MP-08/MP-10/MP-11: human, history and worker hold wire at current local444.
use super::*;
use crate::local::{RoomEnvironmentHumanAction, SubmitRoomEnvironmentActionRequest};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomComputerInputAction,
};
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp10_mp11_computer_hold_wire_is_bound_to_protocol435() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 480);
    let cases = [
        (
            serde_json::json!({"kind":"keyboard_hold","key":"shift+Left","duration_ms":750}),
            "8d578f35d0565c51ac83c8c525440b234fe98e4cacbb4b62433598c1e6f91e35",
        ),
        (
            serde_json::json!({"kind":"pointer_hold","x":12,"y":24,"button":"right","duration_ms":750}),
            "d62ab12c4e7bdf53f39b70cefa2ad1db9b156a3ab7e70e49f0ea610bea28c1a0",
        ),
    ];
    for (human, expected_hash) in cases {
        let action: RoomEnvironmentHumanAction = serde_json::from_value(human.clone()).unwrap();
        let request =
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: "room".into(),
                runtime_generation: 2,
                viewport_revision: 3,
                idempotency_key: "hold-1".into(),
                action,
            });
        let wire = serde_json::json!({"SubmitRoomEnvironmentAction":{
            "session_id":"room","runtime_generation":2,"viewport_revision":3,"idempotency_key":"hold-1","action":human,
        }});
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(wire.clone()).unwrap(),
            request
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
            expected_hash
        );
        assert!(!format!("{request:?}").contains("shift+Left"));
        let mut worker = human;
        if let Some(key) = worker.as_object_mut().unwrap().remove("key") {
            worker["input"] = key;
        }
        let action: RoomComputerInputAction = serde_json::from_value(worker.clone()).unwrap();
        let command = RoomBrowserControllerCommand::ComputerInput {
            action_id: "hold-1".into(),
            actor_id: "agent:a".into(),
            runtime_generation: 2,
            viewport_revision: 3,
            desktop_pixel_width: 1280,
            desktop_pixel_height: 800,
            action,
        };
        let command_wire = serde_json::json!({"kind":"computer_input", "action_id":"hold-1", "actor_id":"agent:a",
            "runtime_generation":2,"viewport_revision":3,"desktop_pixel_width":1280,"desktop_pixel_height":800,"action":worker});
        assert_eq!(serde_json::to_value(&command).unwrap(), command_wire);
        assert_eq!(
            serde_json::from_value::<RoomBrowserControllerCommand>(command_wire).unwrap(),
            command
        );
    }
}

#[test]
fn mp08_mp10_mp11_protocol411_worker_rejects_hold_instead_of_silently_tapping() {
    // Frozen v411 discriminants reject a hold; it cannot masquerade as KeyboardKey.
    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum LegacyComputerAction {
        KeyboardKey,
        PointerClick,
    }
    for kind in ["keyboard_hold", "pointer_hold"] {
        assert!(serde_json::from_value::<LegacyComputerAction>(
            serde_json::json!({"kind":kind, "duration_ms":750})
        )
        .is_err());
    }
}
