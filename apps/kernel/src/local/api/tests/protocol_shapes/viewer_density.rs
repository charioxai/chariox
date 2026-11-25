//! MP-08/MP-11: allocated 1015 (row 341): a viewer that accepts region stills at its own density says so on its subscribe.
use super::*;
use crate::local::{KernelBrowserCommand, KernelBrowserRequest};
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp11_viewer_density_stills_1015_snapshot_hash() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 1015);
    };
    let bytes = include_bytes!("browser-viewer-density-1015.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "26ec8fb3fb2fad3b4cfdef1138c462f8e8444bdfb30d75c6c1a097150081bbbb"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(snapshot["local_protocol"], 1015);
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
        device_scale_factor: 2,
        initial_push: false,
        tab_display: false,
        viewer_density_stills: false,
    };
    // A viewer that does not ask sends no field.
    assert!(
        serde_json::to_value(LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
            command: subscribe
        }))
        .unwrap()["KernelBrowser"]["command"]
            .get("viewer_density_stills")
            .is_none()
    );
}
