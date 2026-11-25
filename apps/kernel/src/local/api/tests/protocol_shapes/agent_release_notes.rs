use crate::local::{LocalDaemonRequest, LocalDaemonResponse, LOCAL_DAEMON_PROTOCOL_VERSION};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn agent_release_notes_protocol_shape() {
    let bytes = include_bytes!("agent-release-notes.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "d7a700c32362ccdc9df0631d3d9e2281923790eeec0701c8eb480927f168fcef"
    );
    let fixture: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    let request = &fixture["requests"][0];
    let response = &fixture["responses"][0];
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<LocalDaemonRequest>(request.clone()).unwrap()
        )
        .unwrap(),
        *request
    );
    assert_eq!(
        serde_json::to_value(
            serde_json::from_value::<LocalDaemonResponse>(response.clone()).unwrap()
        )
        .unwrap(),
        *response
    );
}
