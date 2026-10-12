//! MP-08/MP-10/MP-11: protocol 447/492/501/503 mirroring shape and rejection contract.
use super::*;
use crate::local::{
    KernelBrowserCommand as C, KernelBrowserInput, KernelBrowserMirrorAction as A,
    KernelBrowserRequest,
};
#[test]
fn browser_mirror_protocol_503_shapes_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 503);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        111
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
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec!["n1".into()],
            wait_ms: None,
            resend: None,
        },
        C::MirrorSubscribe {
            tab_id: "t".into(),
            generation: 2,
            device_scale_factor: 2,
            wire: Some(2),
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec![],
            wait_ms: Some(1500),
            resend: None,
        },
        C::MirrorNext {
            subscription_id: "s".into(),
            generation: 2,
            after_sequence: 3,
            drift_nodes: vec![],
            wait_ms: Some(1500),
            resend: Some(true),
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
            input: KernelBrowserInput::Click { x: 1, y: 2 },
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
        serde_json::from_str(include_str!("browser-mirror-503.json")).unwrap();
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
            wait_ms: Some(1500),
            resend: Some(true),
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
