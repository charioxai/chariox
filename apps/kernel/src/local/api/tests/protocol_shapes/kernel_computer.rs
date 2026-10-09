//! MP-08 / MP-11: coordinator-reserved local461 / peer92 native desktop shapes.
use super::*;
#[test]
fn mp08_mp11_kernel_computer_461_92_shapes_are_hashed() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        96
    );
    let commands = serde_json::json!([
            {"op":"start"},{"op":"state"},{"op":"actors"},
            {"op":"snapshot","target":{"surface_id":"s","generation":"g"}},
            {"op":"screenshot","target":{"surface_id":"s","generation":"g"}},
            {"op":"ocr","target":{"surface_id":"s","generation":"g"},"query":"public"},
            {"op":"clipboard_read","target":{"surface_id":"s","generation":"g"}},
            {"op":"target_action","target":{"surface_id":"s","generation":"g"},"tree_revision":2,"target_id":"atspi-id","action":"click"},
            {"op":"takeover","target":{"surface_id":"s","generation":"g"}},
            {"op":"release","target":{"surface_id":"s","generation":"g"}},
            {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"keycode","keycode":38,"state":"down"}},
            {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"keycode","keycode":38,"state":"up"}},
            {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"composition","text":"public"}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"text","text":"public"}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"key","key":"ctrl+a"}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"hold","key":"Right","duration_ms":60}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"pointer_hold","x":20,"y":30,"button":1,"duration_ms":60}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"click","x":20,"y":30,"button":1}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"move","x":20,"y":30}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"drag","x":20,"y":30,"to_x":40,"to_y":50,"button":1}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"scroll","x":20,"y":30,"steps":-2}},
    {"op":"input","target":{"surface_id":"s","generation":"g"},"input":{"kind":"clipboard_write","text":"public"}}
        ]);
    let mut requests = Vec::new();
    for command in commands.as_array().unwrap() {
        let wire =
            serde_json::json!({"KernelBrowser":{"command":{"op":"computer","command":command}}});
        let typed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
        requests.push(wire);
    }
    let resource = serde_json::json!({"kind":"desktop","surface_id":"s"});
    let typed: crate::local::UserDomainResource = serde_json::from_value(resource.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), resource);
    let snapshot =
        serde_json::json!({"local":461,"peer":92,"requests":requests,"resource":resource});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "b1bb646228363ee41be24efdaa50df31ef39f19742eaf667a41c2df1f70c2f82"
    );
    for command in [
        serde_json::json!({"op":"input","target":{"surface_id":"s","generation":"g","user_id":"forged"},"input":{"kind":"keycode","keycode":38,"state":"down"}}),
        serde_json::json!({"op":"state","observed_by":"forged"}),
        serde_json::json!({"op":"target_action","target":{"surface_id":"s","generation":"g"},"target_id":"id","action":"click"}),
    ] {
        assert!(serde_json::from_value::<crate::local::KernelComputerCommand>(command).is_err());
    }
    let input = crate::local::KernelComputerInput::Composition {
        text: crate::local::RoomEnvironmentKeyboardInput::new("private-canary".into()),
    };
    assert!(!format!("{input:?}").contains("private-canary"));
}

// MP-08 / MP-10 / MP-11: PR5 adds only a terminal desktop source selection.
#[test]
fn mp08_mp11_desktop_display_474_shape_hash_and_actor_rejection() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
    let wire = serde_json::json!({"KernelBrowser": {"command": {"op": "computer", "command": {"op": "display_subscribe", "target": {"surface_id": "desktop-one", "generation": "native-one"}, "codecs": ["avc1.420033", "chariox-relay-binary-v96"], "bitrate": 8000000, "device_scale_factor": 2}}}});
    let typed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "aefcf7a68dcc87e519eb8287105de09058d4dbfeb99e76c900a57af31e2be377"
    );
    for field in ["observed_by", "_agent_input", "user_id"] {
        let mut forged = wire.clone();
        forged["KernelBrowser"]["command"]["command"][field] = "forged".into();
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
