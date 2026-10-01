use super::*;
use crate::local::KernelConnectionClass;
use crate::runtime::state::critical_approval_audit_payload;

/// Exhaustive, so a new class fails to compile here until it is versioned.
fn wire_name(class: KernelConnectionClass) -> &'static str {
    match class {
        KernelConnectionClass::Terminal => "terminal",
        KernelConnectionClass::ExternalAgent => "external_agent",
        KernelConnectionClass::KernelAgent => "kernel_agent",
        KernelConnectionClass::Host => "host",
        KernelConnectionClass::RelayPeer => "relay_peer",
        KernelConnectionClass::Unauthenticated => "unauthenticated",
    }
}

#[test]
fn kernel_connection_classes_and_their_audit_attribution_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 393);
    let classes = [
        KernelConnectionClass::Terminal,
        KernelConnectionClass::ExternalAgent,
        KernelConnectionClass::KernelAgent,
        KernelConnectionClass::Host,
        KernelConnectionClass::RelayPeer,
        KernelConnectionClass::Unauthenticated,
    ];
    let vocabulary = serde_json::to_value(classes).unwrap();
    assert_eq!(
        vocabulary,
        serde_json::json!(classes.map(wire_name)),
        "the wire vocabulary is fixed"
    );
    for class in classes {
        assert_eq!(
            serde_json::from_value::<KernelConnectionClass>(serde_json::json!(wire_name(class)))
                .unwrap(),
            class
        );
    }
    // Only terminals may submit a passkey; unauthenticated connections keep
    // their current treatment until enforcement.
    assert_eq!(
        classes.map(KernelConnectionClass::may_submit_passkey),
        [true, false, false, false, false, true]
    );

    let audit = critical_approval_audit_payload(
        "local",
        "decision",
        "verified",
        Some(5),
        Some(KernelConnectionClass::Terminal),
    );
    assert_eq!(
        audit,
        serde_json::json!({
            "owner": "local", "interaction_id": "decision", "outcome": "verified",
            "remember_minutes": 5, "connection_class": "terminal",
        })
    );
    // The kernel's own callers have no connection, so no class.
    let internal = critical_approval_audit_payload("local", "decision", "missing", None, None);
    assert!(internal["connection_class"].is_null());

    let snapshot = serde_json::json!({"classes": vocabulary, "audit": audit, "internal": internal});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "947eca667500d33980960f533e1e35e66ff9dc57c50189afcc1cc8671976bfa4"
    );
}
