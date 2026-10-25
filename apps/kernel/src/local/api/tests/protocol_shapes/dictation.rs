//! Protocol 594: kernel dictation requests and replies.
use super::*;

#[test]
fn dictation_594_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 594);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 200);
    };
    let bytes = include_bytes!("dictation-594.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "1898b5cb5c454eeaae28dd34382cb1dfb9110a67837d34d10be9e62cbb18045d"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for key in [
        "service_local_request",
        "service_openai_request",
        "service_elevenlabs_request",
        "status_request",
        "download_request",
        "language_request",
        "append_request",
        "transcribe_request",
        "transcribe_context_request",
    ] {
        let request: crate::local::LocalDaemonRequest =
            serde_json::from_value(snapshot[key].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            snapshot[key],
            "{key}"
        );
    }
    for key in [
        "status_local",
        "status_openai",
        "status_elevenlabs",
        "status_not_downloaded",
        "status_downloading",
        "status_failed",
        "audio_accepted",
        "transcript",
    ] {
        let response: LocalDaemonResponse = serde_json::from_value(snapshot[key].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            snapshot[key],
            "{key}"
        );
    }
}

#[test]
fn dictation_rejects_unknown_operations_and_fields() {
    for invalid in [
        serde_json::json!({"Dictation": {"command": {"op": "record"}}}),
        serde_json::json!({"Dictation": {"command": {"op": "set_language", "language": null, "extra": true}}}),
        serde_json::json!({"Dictation": {"command": {"op": "transcribe"}}}),
    ] {
        assert!(serde_json::from_value::<crate::local::LocalDaemonRequest>(invalid).is_err());
    }
}
