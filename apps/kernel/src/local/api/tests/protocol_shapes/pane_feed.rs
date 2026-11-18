//! Local 1031: the pane feed (docs/PROTOCOL.md section 3.3.2.1): what one agent pane shows,
//! read by the web pane and the provider terminals alike, and the settings a provider
//! terminal reports back.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn pane_feed_1031_snapshot_and_hash() {
    let bytes = include_bytes!("pane-feed-1031.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "0f45c10969b16a208326d49b86dd9f4bbb4675c41f63e59fca88c749bba93f68"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["responses"].as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    // A terminal reports what it changed and nothing else; the feed is the kernel's.
    for forged in [
        json!({"ReportPaneSettings":{"session_id":"r","agent_id":"a","account":"other"}}),
        json!({"ReportPaneSettings":{"session_id":"r","agent_id":"a","text_terminal":true}}),
        json!({"GetPaneFeed":{"session_id":"r"}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
