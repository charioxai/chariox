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
            layout_delta: None,
            layout_width: None,
            layout_height: None,
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec!["n1".into()],
            known_sheets: vec![],
            known_fonts: vec![],
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
            layout_delta: None,
            layout_width: None,
            layout_height: None,
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec![],
            known_sheets: vec![],
            known_fonts: vec![],
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
            known_fonts: vec![],
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
            button: None,
        }),
        binding(A::Click {
            node_id: "n1".into(),
            x: Some(4.5),
            y: Some(2.0),
            button: None,
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
            known_fonts: vec![],
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
// B3/B4: a hover move, a leave and a right click are mirror actions of their own shape; a middle-button page click is not one.
#[test]
fn browser_mirror_move_and_right_click_shapes() {
    for (value, ok) in [
        (
            serde_json::json!({"kind":"move","node_id":"n1","x":4.5,"y":2.0}),
            true,
        ),
        (
            serde_json::json!({"kind":"click","node_id":"n1","x":4.5,"y":2.0,"button":"right"}),
            true,
        ),
        (serde_json::json!({"kind":"leave"}), true),
        (serde_json::json!({"kind":"leave","node_id":"n1"}), false),
        (serde_json::json!({"kind":"move","node_id":"n1"}), false),
        (
            serde_json::json!({"kind":"move","node_id":"n1","x":1.0,"y":1.0,"button":"right"}),
            false,
        ),
        (
            serde_json::json!({"kind":"click","node_id":"n1","button":"left"}),
            false,
        ),
    ] {
        let parsed = serde_json::from_value::<A>(value.clone());
        assert_eq!(parsed.is_ok(), ok, "{value}");
        if let Ok(action) = parsed {
            assert_eq!(serde_json::to_value(action).unwrap(), value);
        }
    }
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
        "02f374d18cb2a6d74617c285bc2c439bd4c88adb1d8bd8b5cb6762d07877d9bf"
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
fn browser_mirror2_font_cache_1011_shape_is_pinned() {
    // MIRROR-P2: 1011 mirror v2 wire contract (fonts the viewer keeps across documents).
    let snapshot = include_str!("browser-mirror2-font-cache-1011.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(snapshot.as_bytes())),
        "e2e4607965f221ee299d1ede36f02705a7ce09c8c6ea4730a888224139611968"
    );
    let fixture: serde_json::Value = serde_json::from_str(snapshot).unwrap();
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION as u64 >= fixture["protocol_version"].as_u64().unwrap());
    let request: LocalDaemonRequest = serde_json::from_value(fixture["command"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), fixture["command"]);
    // A credit without the list stays accepted and serializes without it.
    let mut legacy = fixture["command"].clone();
    legacy["KernelBrowser"]["command"]
        .as_object_mut()
        .unwrap()
        .remove("known_fonts");
    let request: LocalDaemonRequest = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), legacy);
}

#[test]
fn browser_mirror2_navigation_550_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 550/158 mirror v2 wire contract (preview + causal drain).
    let snapshot = include_str!("browser-mirror2-navigation-550.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(snapshot.as_bytes())),
        "a529414775b732617235b8d77cfd407807460adec8d81b680e4e13cb3989b605"
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
        "93fc17a4dc862adc1f1fc38429b3b9853b67c5ef8b2483d84845ca3736f8aad3"
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
        "78d88d8fbaa49b9ca9083e271147db1d79924a0208c309912e13bb3f21619cfe"
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
        "395c0fba2f04e0f872bdfb5bf2b506bbce899a3a0ca31313df412c04e8cab65e"
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
        "3d28e9e70617eeb1b15429184f86d5075da3cd18079f97a0962097da0204130e"
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
fn browser_mirror2_layout_delta_1015_shape_is_pinned() {
    // MP-08/MP-10: local 1015 (relay peer unchanged): a subscription that asks for `layout_delta` and holds a complete
    // snapshot follows a page size change by an ordinary delta whose header names the new `css_width`/`css_height`;
    // the page's own reaction follows as ops. Without it the stream still re-snapshots (resetReason 'layout').
    let bytes = include_bytes!("browser-mirror2-layout-delta-1015.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "a508e2efe6bfa7a49de4bfc71653b2100a25b084a5e515972581655354fb8093"
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
    for name in ["subscribed", "resized_delta", "reaction_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_focus_visible_1013_shape_is_pinned() {
    // MP-08/MP-10: local 1013 (relay peer 450): the mirror v2 header says whether the page's focused element
    // matches :focus-visible, beside `focused`; a delta carries it when only the ring changes.
    let bytes = include_bytes!("browser-mirror2-focus-visible-1013.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "8cd8d543b67a8d3f3a4a85f22beb1bdca71cd70506c40b614dae082c69853c35"
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
    for name in ["pressed_delta", "key_delta"] {
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
        "b73ce876f0546c2581a16c00c4dec3032e96536b4d783a1fbb2e0ae8fec06c03"
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

#[test]
fn browser_pointer_cursor_and_document_mode_920_shape_is_pinned() {
    // MP-08/MP-10/MP-11: protocol 920 / relay peer 371: a mirror reset carries the page's document mode when it is not
    // no-quirks, and `pointer_cursor` reads the CSS cursor the page shows at a point (the viewer shows it over video).
    let bytes = include_bytes!("browser-pointer-cursor-document-mode-920.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "f0deba106dce40887bd8be55d2404d612de8b65dc0e4577dfcc35ecbc8ff7ef6"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let request: LocalDaemonRequest =
        serde_json::from_value(fixture["pointer_cursor"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture["pointer_cursor"]
    );
    for name in ["pointer_cursor_result", "quirks_reset"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_page_wheel_1014_shape_is_pinned() {
    // MP-08/MP-10: provisional 1014 (relay peer 451): the mirror v2 header says whether the page's own script takes the
    // wheel (a non-passive wheel listener on window, document, html or body); a delta carries it when it changes.
    let bytes = include_bytes!("browser-mirror2-page-wheel-1014.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "8660345611bf2049257cfcfcdf913228244930b91c141f8f55e8f957f0921864"
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
    for name in ["takes_delta", "gives_back_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

#[test]
fn browser_mirror2_media_playing_1015_shape_is_pinned() {
    // Row 486: provisional local 1015: the mirror v2 header says whether a mirrored video or audio element plays (muted or
    // not), so the viewer never evicts that Tab; a delta carries it when it changes.
    let bytes = include_bytes!("browser-mirror2-media-playing-1015.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "6976c59bc54c8a518d6a8c7e8f5f8ffec06e4e8ea553a86c26a4ce32e0d244dc"
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
    for name in ["starts_playing", "stops_playing"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": fixture[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}
