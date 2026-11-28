//! Local 1015: the pane feed (docs/PROTOCOL.md section 3.3.2.1): what one agent pane shows,
//! read by the web pane and the provider terminals alike, and the settings a provider
//! terminal reports back.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn pane_feed_1015_snapshot_and_hash() {
    let bytes = include_bytes!("pane-feed-1015.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "36370919023aee8cef3fe248c1ba61fedea94a0d5b1b427ae9924e28bfbcc8e8"
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
