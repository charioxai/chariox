//! MP-08 / MP-10 / MP-11: allocated accounting wire; local448 / peer91.
use super::*;
use sha2::{Digest, Sha256};
#[test]
fn mp08_mp10_mp11_provider_accounting_wire_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 448);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        91
    );
    let request = LocalDaemonRequest::GetSessionUsage(crate::local::GetSessionUsageRequest {
        session_id: "session".into(),
    });
    let usage = crate::usage_accounting::Usage {
        input: Some(100),
        cached_input: Some(40),
        output: Some(10),
        reasoning: Some(6),
        ..Default::default()
    };
    let run = crate::provider::ProviderRunTokenUsage {
        accounting: Some(usage),
        turn_accounting: Some(usage),
        ..Default::default()
    };
    let response = LocalDaemonResponse::SessionUsage {
        report: crate::usage_accounting::report::from_turns("session", vec![]),
    };
    let encoded = serde_json::to_vec(&serde_json::json!([request, run, response])).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&encoded)),
        "0f2dc009611c93a741653f9e6650777f4739bacfc70dca47aa321146093eb716"
    );
}
