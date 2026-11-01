//! Protocol 603: kernel dictation requests and replies.
use super::*;

#[test]
fn dictation_603_snapshot() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 603);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 209);
    };
    let bytes = include_bytes!("dictation-603.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "2110cddfa2af33a427c86e60cc8d6cf6e5a6e3d148bbfbb92d46b4ea004d5299"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    for key in [
        "service_local_request",
        "service_openai_request",
        "service_elevenlabs_request",
        "status_request",
        "clear_service_request",
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
        "status_not_configured",
        "status_vault_locked",
        "status_key_missing",
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

#[test]
fn dictation_594_snapshot_remains_the_original_local_only_contract() {
    let bytes = include_bytes!("dictation-594.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "a131c1d3aa3031fb91d89dfe54a01e21bf9f02bf0db07961262b791a4b3dcece"
    );
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert!(value.get("service_openai_request").is_none());
}
