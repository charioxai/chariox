//! MP-08/MP-10/MP-11: protocol 447/492/501/503/517 mirroring shape and rejection contract.
use super::*;
use crate::local::{
    KernelBrowserCommand as C, KernelBrowserInput, KernelBrowserMirrorAction as A,
    KernelBrowserRequest,
};
#[test]
fn browser_mirror_protocol_443_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let binding = |action| C::MirrorInput {
        tab_id: "t".into(),
        generation: 2,
        document_id: "d".into(),
        subscription_id: "s".into(),
        sequence: 3,
        action,
    };
    let commands = vec![
        C::MirrorSubscribe {
            tab_id: "t".into(),
            generation: 2,
            device_scale_factor: 2,
            wire: None,
            navigation_preview: None,
            font_requirements: None,
            font_absent: None,
            viewport_first: None,
            visited_links: None,
            layout_width: None,
            layout_height: None,
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec!["n1".into()],
            known_sheets: vec![],
            wait_ms: None,
            resend: None,
            flush: None,
        },
        C::MirrorSubscribe {
            tab_id: "t".into(),
            generation: 2,
            device_scale_factor: 2,
            wire: Some(2),
            navigation_preview: None,
            font_requirements: None,
            font_absent: None,
            viewport_first: None,
            visited_links: None,
            layout_width: None,
            layout_height: None,
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec![],
            known_sheets: vec![],
            wait_ms: Some(1500),
            resend: None,
            flush: None,
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec![],
            known_sheets: vec![],
            wait_ms: Some(1500),
            resend: Some(true),
            flush: None,
        },
        C::MirrorClose {
            subscription_id: "s".into(),
            generation: 2,
        },
        binding(A::Click {
            node_id: "n1".into(),
            x: None,
            y: None,
        }),
        binding(A::Click {
            node_id: "n1".into(),
            x: Some(4.5),
            y: Some(2.0),
        }),
        binding(A::Focus {
            node_id: "n1".into(),
        }),
        binding(A::Text {
            node_id: Some("n1".into()),
            text: "fixture".into(),
        }),
        binding(A::Text {
            node_id: None,
            text: "fixture".into(),
        }),
        binding(A::Scroll {
            node_id: "n1".into(),
            delta_x: 0,
            delta_y: 10,
            x: None,
            y: None,
        }),
        binding(A::ScrollTo {
            node_id: None,
            x: 0.0,
            y: 640.5,
        }),
        binding(A::Key { key: "Tab".into() }),
        binding(A::Selection {
            anchor_id: "n2".into(),
            anchor_offset: 0,
            focus_id: "n2".into(),
            focus_offset: 3,
        }),
        binding(A::Blur {
            node_id: "n1".into(),
        }),
        binding(A::Set {
            node_id: "n1".into(),
            value: None,
            checked: None,
            selected_index: Some(1),
        }),
        binding(A::Set {
            node_id: "n2".into(),
            value: Some("2026-10-10".into()),
            checked: None,
            selected_index: None,
        }),
        binding(A::Set {
            node_id: "n3".into(),
            value: None,
            checked: Some(true),
            selected_index: None,
        }),
        binding(A::Coordinate {
            input: KernelBrowserInput::Click {
                x: 1,
                y: 2,
                button: None,
            },
        }),
    ];
    let values: Vec<_> = commands
        .into_iter()
        .map(|command| {
            serde_json::to_value(LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
                command,
            }))
            .unwrap()
        })
        .collect();
    for value in &values {
        assert!(serde_json::from_value::<LocalDaemonRequest>(value.clone()).is_ok());
    }
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("browser-mirror-517.json")).unwrap();
    assert_eq!(serde_json::json!(values), expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&values).unwrap())),
        "11b80a68517f28e8b03508f786b044aa88175c3b004a858178c1f930ca88c6e5"
    );
    let next = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
        command: C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec![],
            known_sheets: vec![],
            wait_ms: Some(1500),
            resend: Some(true),
            flush: None,
        },
    });
    assert!(
        !crate::runtime_transport::command_cache::request_is_cacheable(&next),
        "MP-11: no retained mirror DOM/media receipts"
    );
}
#[test]
fn browser_mirror_refuses_authority_scripts_and_missing_document_shapes() {
    for value in [
        serde_json::json!({"KernelBrowser":{"command":{"op":"mirror_subscribe","tab_id":"t","generation":1,"device_scale_factor":1,"user_id":"foreign"}}}),
        serde_json::json!({"KernelBrowser":{"command":{"op":"mirror_input","tab_id":"t","generation":1,"subscription_id":"s","sequence":1,"action":{"kind":"click","node_id":"n1"}}}}),
        serde_json::json!({"KernelBrowser":{"command":{"op":"mirror_input","tab_id":"t","generation":1,"document_id":"d","subscription_id":"s","sequence":1,"action":{"kind":"evaluate","script":"unsafe"}}}}),
        // Protocol 492 retired the never-sent composition action; `set` names only control state.
        serde_json::json!({"KernelBrowser":{"command":{"op":"mirror_input","tab_id":"t","generation":1,"document_id":"d","subscription_id":"s","sequence":1,"action":{"kind":"composition","text":"a","selection_start":0,"selection_end":1}}}}),
        serde_json::json!({"KernelBrowser":{"command":{"op":"mirror_input","tab_id":"t","generation":1,"document_id":"d","subscription_id":"s","sequence":1,"action":{"kind":"set","node_id":"n1","value":"x","script":"unsafe"}}}}),
        // Protocol 501: `resend` is a flag, nothing else.
        serde_json::json!({"KernelBrowser":{"command":{"op":"mirror_next","subscription_id":"s","generation":1,"after_sequence":1,"drift_nodes":[],"resend":"1"}}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(value).is_err());
    }
}

#[test]
fn browser_mirror2_sheet_cache_550_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 550/158 mirror v2 wire contract (known sheets).
    let snapshot = include_str!("browser-mirror2-sheet-cache-550.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(snapshot.as_bytes())),
        "ce5ca5b3be62a51eabb9e61b14e469b42d4b465d279183be08da6a1d76eb371f"
    );
    let fixture: serde_json::Value = serde_json::from_str(snapshot).unwrap();
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION as u64 >= fixture["protocol_version"].as_u64().unwrap());
    let request: LocalDaemonRequest = serde_json::from_value(fixture["command"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), fixture["command"]);
    // The pre-extension shape stays accepted and serializes without a dictionary.
    let mut legacy = fixture["command"].clone();
    legacy["KernelBrowser"]["command"]
        .as_object_mut()
        .unwrap()
        .remove("known_sheets");
    let request: LocalDaemonRequest = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), legacy);
}

#[test]
fn browser_mirror2_navigation_550_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 550/158 mirror v2 wire contract (preview + causal drain).
    let snapshot = include_str!("browser-mirror2-navigation-550.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(snapshot.as_bytes())),
        "d60d4494662ee29a1f7c52cb34aab7437da27c4d89042cb749d52fd1a45f88e8"
    );
    let fixture: serde_json::Value = serde_json::from_str(snapshot).unwrap();
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION as u64 >= fixture["protocol_version"].as_u64().unwrap());
    for value in fixture["commands"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let preview = serde_json::json!({"KernelBrowser":{"result":fixture["preview"]}});
    let preview: LocalDaemonResponse = serde_json::from_value(preview).unwrap();
    assert_eq!(
        serde_json::to_value(preview).unwrap()["KernelBrowser"]["result"],
        fixture["preview"]
    );
}

#[test]
fn browser_mirror2_native_font_failures_550_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 550/158 mirror v2 wire contract (font failures).
    let bytes = include_bytes!("browser-mirror2-font-failures-550.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "4c11dce149945060c0011a4301dd821bfccb167613a3e81bb2bbc38cd5027712"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for name in ["reset_header", "inherited_delta", "cleared_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_font_absent_567_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 567/175 adds `absent` (stack families the native machine lacks) to the `fonts` op.
    let bytes = include_bytes!("browser-mirror2-font-absent-567.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "276db700a218febbe433b76b0267a579744a5eda958a2acfe6130dd2f90b0c5c"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let subscribe: LocalDaemonRequest =
        serde_json::from_value(fixture["subscribe"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(subscribe).unwrap(),
        fixture["subscribe"]
    );
    for name in ["subscribed", "absent_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_viewport_first_629_shape_is_pinned() {
    // MP-08/MP-10: protocol 629 (relay peer unchanged) lets a subscription ask for the on-screen part first; off-screen
    // elements arrive with the box they hold (`deferred`) and fill later without moving the page.
    let bytes = include_bytes!("browser-mirror2-viewport-first-629.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "87fa87c1ac79f9d87b1eb6078f5cd33303de6a8796ba2a0f7914c410ebd9823f"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let subscribe: LocalDaemonRequest =
        serde_json::from_value(fixture["subscribe"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(subscribe).unwrap(),
        fixture["subscribe"]
    );
    for name in ["subscribed", "viewport_reset", "fill_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_visited_links_629_shape_is_pinned() {
    // MP-08/MP-10: protocol 629 (relay peer unchanged): a subscription that asks receives an empty `href` for a
    // link the kernel saw visited from the page's origin, at its snapshot or as a later `attr` op.
    let bytes = include_bytes!("browser-mirror2-visited-links-629.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "15c0bcce4c5391b6944b9db211052978d2b61ba8e778dcbd515cd00b2e222d0d"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let subscribe: LocalDaemonRequest =
        serde_json::from_value(fixture["subscribe"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(subscribe).unwrap(),
        fixture["subscribe"]
    );
    for name in ["subscribed", "remark_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_document_font_requirements_550_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 550/158 mirror v2 wire contract (font requirements).
    let bytes = include_bytes!("browser-mirror2-font-requirements-550.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "bfaeaee8849d018fd98c123064fc0bd380cb004166d97f4f540cde6ec019dd92"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let subscribe: LocalDaemonRequest =
        serde_json::from_value(fixture["subscribe"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(subscribe).unwrap(),
        fixture["subscribe"]
    );
    for name in ["subscribed", "reset", "ready_delta", "cleared_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_layout_560_shape_is_pinned() {
    // MP-08/MP-10/MP-11: allocated 560/168 lets the viewer name its display size for the Tab layout.
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 560);
    };
    let bytes = include_bytes!("browser-mirror2-layout-560.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "bed6489de9b231481fbc972031702ebcd80460fa52422aca33420cbe69ebc741"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let subscribe: LocalDaemonRequest =
        serde_json::from_value(fixture["subscribe"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(subscribe).unwrap(),
        fixture["subscribe"]
    );
}
