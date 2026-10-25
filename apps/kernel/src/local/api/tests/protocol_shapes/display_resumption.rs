//! MP-08/MP-10/MP-11: allocated 525/133 shared producer resumption contract.
use super::*;
use crate::local::{KernelBrowserCommand, KernelBrowserRequest};
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp10_mp11_display_resumption_525_133_snapshot_hash() {
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 525);
    assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 133);
    let requests = [
        KernelBrowserCommand::DisplaySuspend {
            subscription_id: "delivery-old".into(),
            generation: 7,
        },
        KernelBrowserCommand::DisplayResume {
            subscription_id: "delivery-old".into(),
            generation: 7,
        },
    ]
    .into_iter()
    .map(|command| LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command }))
    .map(|request| serde_json::to_value(request).unwrap())
    .collect::<Vec<_>>();
    let snapshot = serde_json::json!({
        "local_protocol": 525,
        "peer_protocol": 133,
        "requests": requests,
    });
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "8a3eb16c3a2cbbd170cbd7d608a990f5994b0cff57f9047440d93bed0a0f7674"
    );
    for op in ["display_suspend", "display_resume"] {
        assert!(
            serde_json::from_value::<KernelBrowserCommand>(serde_json::json!({
                "op":op,"subscription_id":"delivery-old","generation":7,"observed_by":"forged"
            }))
            .is_err(),
            "caller identity must come from transport admission"
        );
    }
}

#[test]
fn mp08_mp10_mp11_room_display_carry_525_133_snapshot_hash() {
    let bytes = include_bytes!("room-shared-display-525.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "2992b9a6951f5ba8b5b44619af39e9e91231f60e802694c56a4527fb635d066b"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(snapshot["local_protocol"], 525);
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 525);
    assert_eq!(snapshot["peer_protocol"], 133);
    assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 133);
    let request: LocalDaemonRequest = serde_json::from_value(snapshot["request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), snapshot["request"]);
    let endpoint: LocalDaemonResponse =
        serde_json::from_value(snapshot["endpoint"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(endpoint).unwrap(),
        snapshot["endpoint"]
    );
    for value in snapshot["peer_requests"].as_array().unwrap() {
        let request: crate::transport::relay_peer::RelayPeerRequest =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    for value in snapshot["peer_responses"].as_array().unwrap() {
        let response: crate::transport::relay_peer::RelayPeerResponse =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
}

// MP-08/MP-10/MP-11:535/143 registration retains525 as sole resume authority.
#[test]
fn mp08_mp10_mp11_display_registration_535_143_snapshot_hash() {
    let bytes = include_bytes!("display-registration-535.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "bd3b96605c2cc8c92f6b1322217abb768aaf62f16a133e6b842e7825d660f45f"
    );
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    // The registration shape introduced at 535/143 is carried unchanged by later versions.
    assert_eq!(value["local_protocol"], 535);
    assert_eq!(value["peer_protocol"], 143);
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 535);
    assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 143);
    let registration: chariox_relay::protocol::RelayEnvelope =
        serde_json::from_value(value["registration"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(registration).unwrap(),
        value["registration"]
    );
    assert_eq!(
        value["registration"]["subscription_scope"],
        chariox_relay::frame_transport::DISPLAY_RESUME_SCOPE
    );
    assert!(value["registration"]["resume_from_event_id"].is_null());
    let peer: crate::transport::relay_peer::RelayPeerRequest =
        serde_json::from_value(value["worker_request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(peer).unwrap(), value["worker_request"]);
}

// MP-08/MP-10/MP-11: 553/161 composes client admission with one initial 535 registration.
#[test]
fn mp08_mp10_mp11_display_connect_553_161_snapshot_hash() {
    let bytes = include_bytes!("display-connect-553.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "f20f5d967f65a3524beea6f17acb99bb9b6f4d7f06a4ae9bbef92b61ff7eb3d8"
    );
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(value["local_protocol"], 553);
    assert_eq!(value["peer_protocol"], 161);
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 553);
    assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 161);
    let connect: chariox_relay::protocol::RelayEnvelope =
        serde_json::from_value(value["initial_connect"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(connect).unwrap(),
        value["initial_connect"]
    );
    assert_eq!(
        value["connect_capability"],
        chariox_relay::frame_transport::DISPLAY_RESUME_CONNECT_CAPABILITY
    );
    let mut wrong_scope = value["initial_connect"].clone();
    wrong_scope["registration"]["subscription_scope"] = serde_json::json!("session");
    assert!(serde_json::from_value::<chariox_relay::protocol::RelayEnvelope>(wrong_scope).is_err());
}

// MP-08/MP-10/MP-11: only a bound home peer sends this control; viewer
// offers remain incapable of changing the shared audience.
#[test]
fn room_display_audience_600_206_snapshot_hash() {
    let bytes = include_bytes!("room-display-audience-600.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "154eadcb7e3366b6a36abc1d4a9207550adc1a1de17adedb9b73004196706439"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(snapshot["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        snapshot["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for value in snapshot["requests"].as_array().unwrap() {
        let request: crate::transport::relay_peer::RelayPeerRequest =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let mut viewer_offer =
        serde_json::json!({"codecs":["avc1.420033"],"bitrate":8000000,"device_scale_factor":1});
    assert!(serde_json::from_value::<crate::slice::RoomDisplayOffer>(viewer_offer.clone()).is_ok());
    viewer_offer["mask"] = false.into();
    assert!(serde_json::from_value::<crate::slice::RoomDisplayOffer>(viewer_offer).is_err());
}
