//! MP-08/MP-10/MP-11: allocated 720 (P1) Tab display: one page display per Tab, parked while the mirror paints.
use super::*;
use crate::local::{KernelBrowserCommand, KernelBrowserRequest};
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp10_mp11_tab_display_720_snapshot_hash() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 720);
    };
    let bytes = include_bytes!("browser-tab-display-720.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "24822e28ab2927396570afa973e605f2d333e015b0d219b095f80c6e50cb5c19"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(snapshot["local_protocol"], 720);
    for request in snapshot["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(&serde_json::to_value(parsed).unwrap(), request);
    }
    let subscribe = KernelBrowserCommand::DisplaySubscribe {
        tab_id: "t".into(),
        generation: 1,
        target: None,
        codecs: vec!["png".into(), "avc1.420033".into()],
        bitrate: 8_000_000,
        device_scale_factor: 1,
        initial_push: false,
        tab_display: false,
    };
    // An older viewer's subscribe is unchanged on the wire.
    assert!(
        serde_json::to_value(LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
            command: subscribe
        }))
        .unwrap()["KernelBrowser"]["command"]
            .get("tab_display")
            .is_none()
    );
    assert!(
        serde_json::from_value::<KernelBrowserCommand>(serde_json::json!({
            "op":"display_park","subscription_id":"s","generation":7,"parked":true,"observed_by":"forged"
        }))
        .is_err(),
        "caller identity must come from transport admission"
    );
}
