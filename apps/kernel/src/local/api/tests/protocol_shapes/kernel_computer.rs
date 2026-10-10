//! MP-08 / MP-11: coordinator-allocated local497 / peer106 native desktop display shapes.
use super::*;
#[test]
fn mp08_mp11_m2_kernel_computer_497_106_shapes_are_hashed() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 497);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        106
    );
    let commands = serde_json::json!([
            {"op":"display_subscribe","target":{"surface_id":"s","generation":"g"},"codecs":["png"]},
            {"op":"display_next","target":{"surface_id":"s","generation":"g"},"subscription_id":"viewer","after_sequence":1,"repair":false},
            {"op":"display_close","target":{"surface_id":"s","generation":"g"},"subscription_id":"viewer"},
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
        serde_json::json!({"local":497,"peer":106,"requests":requests,"resource":resource});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "7d0895f72c00ada005d74181be2a390edfef121f93a36f5a0d49e4bcf3d515c6"
    );
    for command in [
        serde_json::json!({"op":"input","target":{"surface_id":"s","generation":"g","user_id":"forged"},"input":{"kind":"keycode","keycode":38,"state":"down"}}),
        serde_json::json!({"op":"state","observed_by":"forged"}),
        serde_json::json!({"op":"display_subscribe","target":{"surface_id":"s","generation":"g"},"codecs":["png"],"_attachment_owner":"forged"}),
        serde_json::json!({"op":"display_next","target":{"surface_id":"s","generation":7},"subscription_id":"viewer","after_sequence":0,"repair":false}),
        serde_json::json!({"op":"target_action","target":{"surface_id":"s","generation":"g"},"target_id":"id","action":"click"}),
    ] {
        assert!(serde_json::from_value::<crate::local::KernelComputerCommand>(command).is_err());
    }
    let input = crate::local::KernelComputerInput::Composition {
        text: crate::local::RoomEnvironmentKeyboardInput::new("private-canary".into()),
    };
    assert!(!format!("{input:?}").contains("private-canary"));
}
