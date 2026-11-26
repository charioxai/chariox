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
        "d64b64de09fe6eae6df4794d8cc62ebfa289cc8d197484970a66780c08b4979d"
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
        "45c7efd32c7cac1e6396960609c3b2fc28c36a56a9af908033e536c94bb5c541"
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
        "36f5839538e5ff9eda981e14371c969e652b28b808821cbe7d53444f1006a536"
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
        "630274ea5a83d58ae4acecd9140cc0eee7c81c1ab7a4e24351a1da90762f07e3"
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
        "0da9dd41d1ab1953fde24928b0e7d948e2c522b0516ab393bd7e7605f5047aee"
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
        "83e109e88f834649846656fbe21cff5fed12a093287059c9cc933dc7762aa14c"
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
        "9986e664db82496567fde0a182ef39496054f0e8778fe7f387d53db3ff78e300"
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
        "ebd5353888185c697fdab62f442236e3beee412b4f9ef5e98f9a0f4bc2668c85"
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
        "319cd3377e5c1934ff5db69fe044384be78d9fa808f16df6e73c55c324f5664c"
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
        "6608b08f58c8cc72129e02209f8202d5344910ba9643654c3ce5e5f13ec92321"
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
        "1b2c254afc9ef76a31829331c72b28c01efa93f17b6c37e8cbcfb4b466fc236e"
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
