//! Staging round 2: reconcile retained grants, DOM mirror and typed refusal shapes.
use super::*;

#[test]
fn staging_union_protocol_454_snapshot_and_hash() {
    use crate::error::UserDomainRefusalReason as Reason;
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 467);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        95
    );
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("staging-union-454.json")).unwrap();
    let requests: Vec<LocalDaemonRequest> =
        serde_json::from_value(expected["requests"].clone()).unwrap();
    let reasons = [
        Reason::NotFocusedAgent,
        Reason::ForeignOwner,
        Reason::StaleEpoch,
        Reason::StaleReference,
        Reason::NotGranted,
        Reason::SensitiveRequiresFocus,
    ];
    let actual = serde_json::json!({
        "local_daemon_protocol_version": LOCAL_DAEMON_PROTOCOL_VERSION,
        "relay_peer_protocol_version": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "requests": requests, "refusal_reasons": reasons,
    });
    assert_eq!(actual, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("staging-union-454.json"))
        ),
        "1a87e0e2c6124c83cf60794bab8eb57700edb63a267d80a5f554af54f596ebae"
    );
    for request in requests {
        if matches!(
            &request,
            LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                command: crate::local::KernelBrowserCommand::MirrorNext { .. }
                    | crate::local::KernelBrowserCommand::SubscribeGrants { .. },
            })
        ) {
            assert!(
                !crate::runtime_transport::command_cache::request_is_cacheable(&request),
                "Union event/DOM streams must not enter the replay cache"
            );
        }
    }
}
