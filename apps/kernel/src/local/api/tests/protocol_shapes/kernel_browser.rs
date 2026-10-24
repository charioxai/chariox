//! MD-2: protocol 461 sessionless host-browser snapshots.
use super::*;
use crate::local::{
    KernelBrowserCommand as Command, KernelBrowserInput as Input, KernelBrowserRequest,
};

#[test]
fn kernel_browser_protocol_443_request_snapshots() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let cases = [
        (Command::Start, serde_json::json!({"op":"start"})),
        (Command::State, serde_json::json!({"op":"state"})),
        (Command::Stop, serde_json::json!({"op":"stop"})),
        (
            Command::Open {
                url: "https://example.com".into(),
            },
            serde_json::json!({"op":"open","url":"https://example.com"}),
        ),
        (
            Command::Close {
                tab_id: "t".into(),
                generation: 2,
            },
            serde_json::json!({"op":"close","tab_id":"t","generation":2}),
        ),
        (
            Command::Navigate {
                tab_id: "t".into(),
                generation: 2,
                url: "https://example.com".into(),
            },
            serde_json::json!({"op":"navigate","tab_id":"t","generation":2,"url":"https://example.com"}),
        ),
        (
            Command::Snapshot {
                tab_id: "t".into(),
                generation: 2,
            },
            serde_json::json!({"op":"snapshot","tab_id":"t","generation":2}),
        ),
        (
            Command::Screenshot {
                tab_id: "t".into(),
                generation: 2,
            },
            serde_json::json!({"op":"screenshot","tab_id":"t","generation":2}),
        ),
        (
            Command::Subscribe {
                tab_id: "t".into(),
                generation: 2,
            },
            serde_json::json!({"op":"subscribe","tab_id":"t","generation":2}),
        ),
        (
            Command::Poll {
                subscription_id: "s".into(),
                generation: 2,
            },
            serde_json::json!({"op":"poll","subscription_id":"s","generation":2}),
        ),
        (
            Command::Unsubscribe {
                subscription_id: "s".into(),
                generation: 2,
            },
            serde_json::json!({"op":"unsubscribe","subscription_id":"s","generation":2}),
        ),
        (
            Command::Input {
                tab_id: "t".into(),
                generation: 2,
                input: Input::Click {
                    x: 12,
                    y: 20,
                    button: None,
                },
            },
            serde_json::json!({"op":"input","tab_id":"t","generation":2,"input":{"kind":"click","x":12,"y":20}}),
        ),
        (
            Command::Input {
                tab_id: "t".into(),
                generation: 2,
                input: Input::Text {
                    text: "fixture".into(),
                },
            },
            serde_json::json!({"op":"input","tab_id":"t","generation":2,"input":{"kind":"text","text":"fixture"}}),
        ),
        (
            Command::Input {
                tab_id: "t".into(),
                generation: 2,
                input: Input::Key {
                    key: "Tab".into(),
                    modifiers: 0,
                },
            },
            serde_json::json!({"op":"input","tab_id":"t","generation":2,"input":{"kind":"key","key":"Tab"}}),
        ),
        (
            Command::Input {
                tab_id: "t".into(),
                generation: 2,
                input: Input::Scroll {
                    x: 12,
                    y: 20,
                    delta_x: 0,
                    delta_y: 40,
                },
            },
            serde_json::json!({"op":"input","tab_id":"t","generation":2,"input":{"kind":"scroll","x":12,"y":20,"delta_x":0,"delta_y":40}}),
        ),
    ];
    for (command, expected) in cases {
        let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command });
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"KernelBrowser":{"command":expected}})
        );
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(value).unwrap(),
            request
        );
    }
    for invalid in [
        serde_json::json!({"KernelBrowser":{"user_id":"forged","command":{"op":"state"}}}),
        serde_json::json!({"KernelBrowser":{"command":{"op":"input","tab_id":"t","generation":2,"input":{"kind":"evaluate","script":"x"}}}}),
        serde_json::json!({"KernelBrowser":{"command":{"op":"navigate","tab_id":"t","url":"https://example.com"}}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(invalid).is_err());
    }
}

#[test]
fn kernel_browser_protocol_443_response_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let result = serde_json::from_str::<serde_json::Value>(include_str!(
        "kernel-browser-agent-tabs-491.json"
    ))
    .unwrap()["KernelBrowser"]["result"]
        .clone();
    let response = LocalDaemonResponse::KernelBrowser {
        result: result.clone(),
    };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::json!({"KernelBrowser":{"result":result}})
    );
}

#[test]
fn kernel_browser_display_protocol_443_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let commands = [
        Command::DisplaySubscribe {
            tab_id: "t".into(),
            generation: 2,
            target: None,
            codecs: vec![
                "avc1.420033".into(),
                "png".into(),
                "chariox-stripes-v1".into(),
            ],
            bitrate: 2_000_000,
            device_scale_factor: 2,
            initial_push: false,
        },
        Command::DisplayNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
        },
        Command::DisplayAck {
            subscription_id: "s".into(),
            generation: 2,
            sequence: 3,
            lost: true,
            held_ms: None,
        },
        Command::DisplayInput {
            tab_id: "t".into(),
            generation: 2,
            document_id: "d".into(),
            binding_id: None,
            sequence: None,
            input: Input::Text {
                text: "fixture".into(),
            },
        },
        Command::DisplayCapture {
            tab_id: "t".into(),
            generation: 2,
        },
        Command::DisplayTakeover {
            tab_id: "t".into(),
            generation: 2,
        },
        Command::DisplayRelease {
            tab_id: "t".into(),
            generation: 2,
        },
        Command::DisplayActors,
    ];
    let values: Vec<_> = commands
        .iter()
        .map(|command| {
            serde_json::to_value(LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
                command: command.clone(),
            }))
            .unwrap()
        })
        .collect();
    for value in &values {
        assert!(serde_json::from_value::<LocalDaemonRequest>(value.clone()).is_ok());
    }
    let event = crate::transport::kernel_protocol::KernelEvent::KernelBrowserFrame {
        subscription_id: "s".into(),
        frame: serde_json::json!({"kind":"stripes","base_sequence":3,"sequence":4,"document_id":"d","generation":2,"tab_id":"t","width":2560,"height":1600,"device_scale_factor":2,"stripes":[{"row":0,"y":0,"height":200,"codec":"avc1.420033","key":false,"sequence":5,"reference_sequence":4,"data_base64":"b3BhcXVl"}]}),
    };
    // MP-08/MP-10: protocol 466 pins the binary display event and a lossless
    // scroll frame (moves + WebP residual) exactly as clients receive them.
    let scroll = crate::transport::kernel_protocol::KernelEvent::KernelBrowserFrame {
        subscription_id: "s".into(),
        frame: serde_json::json!({"kind":"tiles","base_sequence":4,"sequence":5,"document_id":"d","generation":2,"tab_id":"t","width":1920,"height":1080,"device_scale_factor":1,"moves":[[0,0,1920,1060,-20]],"tiles":[{"x":0,"y":1060,"width":1920,"height":20,"format":"webp","data_base64":"UklGRg=="}]}),
    };
    let binary = |event: &crate::transport::kernel_protocol::KernelEvent| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(
            crate::transport::kernel_browser_display::encode_display_event(event.clone()).unwrap(),
        )
    };
    let snapshot =
        serde_json::json!({"requests":values,"event":binary(&event),"scroll":binary(&scroll)});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "422dea57f036781e882a57634a1a3bba953505038853725eb2b2aa0768f4d7ed"
    );
}

#[test]
fn mp08_agent_tabs_protocol_491_snapshot_and_hash() {
    // MP-08/MP-11: allocated491 combines provenance/activity with the native display union.
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("kernel-browser-agent-tabs-491.json")).unwrap();
    let response: LocalDaemonResponse = serde_json::from_value(snapshot.clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), snapshot);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "cf8e119de5af3e428663bdd2795f6f75e68ac8aa2d3d76291f7c95795b259b55"
    );
}

// MP-08/MP-11: protocol517 bounds modifiers at deserialization, shared by
// local, remote and provider input. Legacy keys still omit the zero mask.
#[test]
fn kernel_browser_protocol_517_bounded_modifiers() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 517);
    };
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let input = Input::Key {
        key: "a".into(),
        modifiers: 2,
    };
    let wire = serde_json::to_value(&input).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("browser-key-modifiers-517.json")).unwrap();
    assert_eq!(wire, expected);

    assert_eq!(
        wire,
        serde_json::json!({"kind":"key","key":"a","modifiers":2})
    );
    assert_eq!(
        serde_json::from_value::<Input>(wire.clone()).unwrap(),
        input
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "1cdc9ff0b2391e8a184cad088c6b2fab792fd2f95b0ee37265cb600687e3cc8c"
    );
    for value in [
        serde_json::json!(-1),
        serde_json::json!(16),
        serde_json::json!(256),
        serde_json::json!(1.5),
        serde_json::json!(null),
        serde_json::json!("2"),
    ] {
        assert!(serde_json::from_value::<Input>(
            serde_json::json!({"kind":"key","key":"a","modifiers":value})
        )
        .is_err());
    }
}

#[test]
fn mp08_mp10_mp11_browser_display_region_519_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 519);
    };
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("browser-display-region-519.json")).unwrap();
    for value in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "600c3e22c812a66ee7712a302b5889e83db4ba1f91b4d8affcc77281bb027b32"
    );
    for field in ["rect", "xid", "pid", "selector", "policy"] {
        let mut request = snapshot["requests"][0].clone();
        request["KernelBrowser"]["command"]["target"][field] = serde_json::json!(1);
        assert!(serde_json::from_value::<LocalDaemonRequest>(request).is_err());
    }
}

// MP-08/MP-10/MP-11 (BROWSER-UX-14/-16): allocated 528/136 adds the Browser state's
// `chrome_window` (the native band's window epoch and active Tab, or null) and
// `browser_lockdown`. The host's state keys are pinned to this fixture by a Node test.
#[test]
fn mp08_mp10_mp11_browser_chrome_state_528_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 528);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 136);
    };
    let bytes = include_bytes!("browser-chrome-state-528.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "ff8cb6bbc22c9002844ea72538a801501ef9fd8dacc870e56bd5e366086383e8"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
}

// MP-08/MP-10/MP-11 (BROWSER-UX-14): allocated 530/138 adds the band's middle/right
// click and Tab drag, the `browser_auxiliary` display target (the band window's
// menus, omnibox dropdown, extension popups and bubbles) and the state's
// `chrome_window.auxiliary` list. The host's state keys are pinned to this fixture.
#[test]
fn mp08_mp10_mp11_browser_chrome_ui_530_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let bytes = include_bytes!("browser-chrome-ui-530.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "e706cd0d428575d7f90932a4b89c5ad8872a4a73e60556edeba39a477027b071"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for value in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
    // A primary click keeps its pre-530 shape; unknown buttons, a client-named window
    // and an auxiliary without its window epoch are refused.
    assert_eq!(
        serde_json::to_value(Input::Click {
            x: 1,
            y: 2,
            button: None
        })
        .unwrap(),
        serde_json::json!({"kind":"click","x":1,"y":2})
    );
    for input in [
        serde_json::json!({"kind":"click","x":1,"y":2,"button":"left"}),
        serde_json::json!({"kind":"click","x":1,"y":2,"button":2}),
        serde_json::json!({"kind":"drag","x":1,"y":2,"to_x":3}),
        serde_json::json!({"kind":"drag","x":1,"y":2,"to_x":3,"to_y":-1}),
    ] {
        assert!(serde_json::from_value::<Input>(input).is_err());
    }
    for field in ["xid", "rect", "pid"] {
        let mut request = snapshot["requests"][0].clone();
        request["KernelBrowser"]["command"]["target"][field] = serde_json::json!(1);
        assert!(serde_json::from_value::<LocalDaemonRequest>(request).is_err());
    }
    let mut request = snapshot["requests"][0].clone();
    request["KernelBrowser"]["command"]["target"]
        .as_object_mut()
        .unwrap()
        .remove("window_generation");
    assert!(serde_json::from_value::<LocalDaemonRequest>(request).is_err());
}

// MP-08/MP-10/MP-11 (BROWSER-UX-14): allocated 540/148 keeps the omnibox list's
// `chrome_window.auxiliary` entry with visible height 0 while the address bar has the
// person's keys, so a viewer keeps its stream and the next list paints from it.
#[test]
fn mp08_mp10_mp11_browser_omnibox_warm_540_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 540);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 148);
    };
    let bytes = include_bytes!("browser-omnibox-warm-540.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "71dc08a7419e1ce576380028c94fab6a9d36c9b08372c884b69948a0c70470bd"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    // PERF_AUDIT 4.2.2/4.2.6: a band input's reply carries the state's `chrome_window` shape.
    for key in ["response", "band_input_response"] {
        let response: LocalDaemonResponse = serde_json::from_value(snapshot[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), snapshot[key]);
    }
    // PERF_AUDIT 4.2.3: the subscribe asks for its first frame at relay registration.
    let request: crate::local::LocalDaemonRequest =
        serde_json::from_value(snapshot["initial_push_request"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        snapshot["initial_push_request"]
    );
}

// MP-08/MP-10 (allocated 556/164): `state_wait` parks a Browser state read until the
// state differs from the `revision` the caller holds; the reply is the state plus a new `revision`.
#[test]
fn mp08_mp10_browser_state_wait_556_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 556);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 164);
    };
    let bytes = include_bytes!("browser-state-wait-556.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "4b72ea2ed7ac115e2de252bbf36e612b30016df3dd97bb6b48deef2b1c8eec73"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for key in ["request", "first_request"] {
        let request: crate::local::LocalDaemonRequest =
            serde_json::from_value(snapshot[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), snapshot[key]);
    }
    // A caller with no state yet may omit `after`.
    let omitted: crate::local::LocalDaemonRequest = serde_json::from_value(
        serde_json::json!({"KernelBrowser":{"command":{"op":"state_wait","wait_ms":0}}}),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(omitted).unwrap(),
        snapshot["first_request"]
    );
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
}

#[test]
fn mp08_mp10_mp11_browser_display_placement_529_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 529);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 137);
    };
    let bytes = include_bytes!("browser-display-placement-529.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "1396cbd02821e79f5fa2f4166548d9d1f4c1ca554cf7100873d556205cbae47b"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(snapshot["request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), snapshot["request"]);
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
}

// MP-08/MP-10/MP-11: opaque mirror bodies are carried by the ordinary kernel response.
#[test]
fn mp08_mp10_mp11_mirror_completion_550_158_snapshot_hash() {
    let bytes = include_bytes!("browser-mirror2-completion-550.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "30553a86ec7f8e0d98844a5dce6ef4b05250cc0ad35eeb64b771dffc8cec3bb1"
    );
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(value["local_protocol"], 567);
    assert_eq!(value["peer_protocol"], 175);
    for name in ["completion_delta", "inherited_delta"] {
        let wire = serde_json::json!({"KernelBrowser": {"result": value[name]}});
        let typed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), wire);
    }
}

// MP-08/MP-10/MP-11: default page capture uses the existing region lease shape.
#[test]
fn mp08_mp10_mp11_browser_visible_page_536_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 536);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 144);
    };
    let bytes = include_bytes!("browser-visible-page-536.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "867f99b3a87658e32c893b7030ac6c0d20b05ab001e336563081b8ab5892e588"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for value in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
    let ack: LocalDaemonResponse =
        serde_json::from_value(snapshot["ack_response"].clone()).unwrap();
    assert_eq!(serde_json::to_value(ack).unwrap(), snapshot["ack_response"]);
}

// MP-08/MP-10/MP-11: allocated 552/160 adds the Browser reply's `parked_input` count.
#[test]
fn mp08_mp10_mp11_browser_parked_input_552_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 552);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 160);
    };
    let bytes = include_bytes!("browser-parked-input-552.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "e52a88111dc29c63dbddf377c8a797b264bca205ff89176d596abebf5e99906e"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let response: LocalDaemonResponse =
        serde_json::from_value(snapshot["response"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        snapshot["response"]
    );
}

// MP-08/MP-10/MP-11: allocated 560/168 adds the `browser_window` display target (owner window display).
#[test]
fn mp08_mp10_mp11_browser_window_display_560_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 560);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 168);
    };
    let bytes = include_bytes!("browser-window-display-560.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "18b0226af9d413a8abc9e74585da22398232b5f1b744bfd7cf59afb34662a174"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let request: LocalDaemonRequest = serde_json::from_value(snapshot["request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), snapshot["request"]);
    // A window display names its epoch only: no Tab geometry, no window handle.
    let extra = serde_json::json!({"KernelBrowser":{"command":{"op":"display_subscribe","tab_id":"t","generation":7,
        "target":{"kind":"browser_window","window_generation":3,"xid":42},"codecs":["png"],"bitrate":2000000,"device_scale_factor":1}}});
    assert!(serde_json::from_value::<LocalDaemonRequest>(extra).is_err());
}
