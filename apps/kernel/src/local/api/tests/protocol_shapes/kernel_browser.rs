//! MD-2: protocol 461 sessionless host-browser snapshots.
use super::*;
use crate::local::{
    KernelBrowserCommand as Command, KernelBrowserInput as Input, KernelBrowserRequest,
};

#[test]
fn kernel_browser_protocol_443_request_snapshots() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
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
                input: Input::Click { x: 12, y: 20 },
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
                input: Input::Key { key: "Tab".into() },
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
    let result = serde_json::json!({"generation":2,"state":"ready","tabs":[{"tab_id":"host-tab-t","document_id":"d","url":"https://example.com/","title":"Example"}]});
    let response = LocalDaemonResponse::KernelBrowser {
        result: result.clone(),
    };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        serde_json::json!({"KernelBrowser":{"result":result}})
    );
}

#[test]
fn kernel_browser_display_protocol_475_shapes_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
    let commands = [
        Command::DisplaySubscribe {
            tab_id: "t".into(),
            generation: 2,
            codecs: vec![
                "avc1.420033".into(),
                "png".into(),
                "chariox-stripes-v1".into(),
            ],
            bitrate: 2_000_000,
            device_scale_factor: 2,
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
        },
        Command::DisplayInput {
            tab_id: "t".into(),
            generation: 2,
            document_id: "d".into(),
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
