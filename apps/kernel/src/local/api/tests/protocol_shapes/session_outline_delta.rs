use super::*;

#[test]
fn session_outline_delta_snapshot_and_hash() {
    let bytes = include_bytes!("session-outline-delta-1014.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "592112103ee47b96b17d059beeaede0c8c55311e859ee845a67508551a47f337"
    );
    let expected: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(expected["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    let request: LocalDaemonRequest = serde_json::from_value(expected["request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), expected["request"]);
    for variant in ["full", "delta"] {
        let response: LocalDaemonResponse =
            serde_json::from_value(expected[variant].clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), expected[variant]);
    }
}
