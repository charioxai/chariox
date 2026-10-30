use super::*;
use sha2::{Digest, Sha256};

#[test]
fn local_daemon_protocol_provider_account_credential_shape_is_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

    let request = LocalDaemonRequest::SetProviderAccountCredential(
        crate::local::SetProviderAccountCredentialRequest {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            provider: "claude".to_string(),
            account_profile: "work".to_string(),
            value: "setup-token-secret".to_string(),
            run: true,
            overwrite: true,
        },
    );
    let legacy: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "SetProviderAccountCredential": {"provider":"claude", "account_profile":"work", "value":"synthetic", "overwrite":false}
    })).unwrap();
    assert!(
        matches!(legacy, LocalDaemonRequest::SetProviderAccountCredential(ref request) if !request.run)
    );
    let response = LocalDaemonResponse::ProviderAccountCredentialStored {
        provider: "claude".to_string(),
        account_profile: "work".to_string(),
        credential_id: "provider-account-claude-handle".to_string(),
        replaced: true,
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/account_profile"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/overwrite"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderAccountCredentialStored/credential_id"),
        Some(&serde_json::json!("provider-account-claude-handle"))
    );

    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/run"),
        Some(&serde_json::json!(true))
    );
    let encoded = serde_json::to_string(&snapshot).expect("snapshot should encode");
    let hash = Sha256::digest(encoded.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "e525fa0b75a08d3ce4267c0fe016d37a5ae1fea0d8734368bb3887bc833cf8bc"
    );
}

#[test]
fn account_copy_public_source_request_is_admitted() {
    let source = serde_json::json!({"CopyProviderAccountToKernel": {
        "provider":"claude", "account_profile":"work-exact", "operation_id":"op-1",
        "target_kernel_id":"target", "target_key_thumbprint":"a".repeat(64), "replace":false
    }});
    let parsed = serde_json::from_value::<LocalDaemonRequest>(source.clone());
    assert!(
        parsed.is_ok(),
        "MP-08/MP-09/MP-10/MP-11 source request must be supported"
    );
    let parsed = parsed.unwrap();
    assert!(
        crate::runtime::kernel_gateway::ledger::deadline_for(&parsed)
            >= std::time::Duration::from_secs(150)
    );
    assert_eq!(serde_json::to_value(parsed).unwrap(), source);
}

#[test]
fn account_copy_encrypted_peer_request_is_admitted() {
    let private = crate::transport::relay_crypto::generate_private_key_base64();
    let public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&private).unwrap();
    let peer = serde_json::json!({"kind":"install_provider_account_copy", "binding": {
        "operation_id":"op-1", "provider":"claude", "source_profile_id":"work-exact",
        "source_kernel_id":"source", "source_key_thumbprint":"b".repeat(64),
        "target_kernel_id":"target", "target_key_thumbprint":"a".repeat(64), "replace":false
    }, "credential":"synthetic-copy-canary"});
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &private,
        &public,
        &serde_json::to_vec(&peer).unwrap(),
    )
    .unwrap();
    let plaintext =
        crate::transport::relay_crypto::decrypt_payload_for_private_key(&private, &encrypted)
            .unwrap();
    let request = serde_json::from_slice::<crate::transport::relay_peer::RelayPeerRequest>(
        &plaintext.plaintext,
    );
    assert!(
        request.is_ok(),
        "encrypted account install must be supported"
    );
    assert!(!format!("{:?}", request.unwrap()).contains("synthetic-copy-canary"));
}

#[test]
fn account_copy_protocol_609_peer_215_snapshot_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let request:LocalDaemonRequest=serde_json::from_value(serde_json::json!({"CopyProviderAccountToKernel":{
        "provider":"claude","account_profile":"work","operation_id":"op-1","target_kernel_id":"target",
        "target_key_thumbprint":"a".repeat(64),"replace":false
    }})).unwrap();
    let receipt = crate::provider_account_copy::ProviderAccountCopyReceipt {
        operation_id: "op-1".into(),
        provider: "claude".into(),
        source_profile_id: "work".into(),
        target_kernel_id: "target".into(),
        account_profile: "copy-target".into(),
    };
    let peer: crate::transport::relay_peer::RelayPeerRequest =
        serde_json::from_value(serde_json::json!({
            "kind":"install_provider_account_copy", "binding":{
                "operation_id":"op-1", "provider":"claude", "source_profile_id":"work",
                "source_kernel_id":"source", "source_key_thumbprint":"b".repeat(64),
                "target_kernel_id":"target", "target_key_thumbprint":"a".repeat(64),"replace":false
            },"credential":"opaque-test-credential"
        }))
        .unwrap();
    let snapshot = serde_json::json!([
        request,
        peer,
        LocalDaemonResponse::ProviderAccountCopied {
            receipt: receipt.clone()
        },
        crate::transport::relay_peer::RelayPeerResponse::ProviderAccountCopyInstalled { receipt },
        crate::transport::relay_peer::RelayPeerResponse::ProviderAccountCopyFailed {
            code: "provider_account_copy_conflict".into()
        }
    ]);
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        hash,
        "43899af76c00f35b24dfde276a44f7345e6d8794197eeb761f9844c99f83555d"
    );
}

#[test]
fn account_copy_recipient_offer_requests_are_admitted() {
    // MP-08 / MP-09 / MP-10 / MP-11: discovery is metadata-only and accepting
    // an offer is routed to the receiving kernel, never a fabricated lease.
    for value in [
        serde_json::json!({"ListProviderAccountCopyOffers": {}}),
        serde_json::json!({"CopyProviderAccountFromKernel": {
            "provider":"claude", "account_profile":"work", "operation_id":"receive-1",
            "source_kernel_id":"source", "source_key_thumbprint":"a".repeat(64), "replace":false
        }}),
        // The session that shows the Vault prompt a locked or new Vault needs.
        serde_json::json!({"CopyProviderAccountFromKernel": {
            "session_id":"session-1", "agent_id":"agent-1",
            "provider":"claude", "account_profile":"work", "operation_id":"receive-1",
            "source_kernel_id":"source", "source_key_thumbprint":"a".repeat(64), "replace":false
        }}),
    ] {
        let request = serde_json::from_value::<LocalDaemonRequest>(value.clone());
        assert!(
            request.is_ok(),
            "recipient account-copy request must be supported"
        );
        assert_eq!(serde_json::to_value(request.unwrap()).unwrap(), value);
    }
    let value = serde_json::json!({"kind":"request_provider_account_copy", "request": {
        "provider":"claude", "account_profile":"work", "operation_id":"receive-1",
        "target_kernel_id":"target", "target_key_thumbprint":"b".repeat(64), "replace":false
    }});
    let request =
        serde_json::from_value::<crate::transport::relay_peer::RelayPeerRequest>(value.clone());
    assert!(
        request.is_ok(),
        "authenticated recipient copy request must be supported"
    );
    assert_eq!(serde_json::to_value(request.unwrap()).unwrap(), value);
}

// The session_id/agent_id on CopyProviderAccountFromKernel are protocol 711.
const _: () = assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 711);

#[test]
fn account_copy_recipient_protocol_711_peer_274_snapshot_and_hash() {
    let list: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"ListProviderAccountCopyOffers":{}})).unwrap();
    let receive: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"CopyProviderAccountFromKernel":{
            "provider":"claude", "account_profile":"work", "operation_id":"receive-1",
            "source_kernel_id":"source", "source_key_thumbprint":"a".repeat(64), "replace":false
        }}))
        .unwrap();
    let scoped: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"CopyProviderAccountFromKernel":{
            "session_id":"session", "agent_id":"agent",
            "provider":"claude", "account_profile":"work", "operation_id":"receive-2",
            "source_kernel_id":"source", "source_key_thumbprint":"a".repeat(64), "replace":false
        }}))
        .unwrap();
    let mut bypass = serde_json::to_value(&receive).unwrap();
    bypass["CopyProviderAccountFromKernel"]["accepted"] = true.into();
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(bypass).is_err(),
        "a client cannot claim kernel consent in its request"
    );
    let peer: crate::transport::relay_peer::RelayPeerRequest =
        serde_json::from_value(serde_json::json!({
            "kind":"request_provider_account_copy", "request":{
                "provider":"claude", "account_profile":"work", "operation_id":"receive-1",
                "target_kernel_id":"target", "target_key_thumbprint":"b".repeat(64), "replace":false
            }
        }))
        .unwrap();
    let offers = LocalDaemonResponse::ProviderAccountCopyOffers {
        offers: vec![crate::provider_account_copy::ProviderAccountCopyOffer {
            source_kernel_id: "source".into(),
            source_machine_id: "machine-a".into(),
            source_label: "A".into(),
            source_key_thumbprint: "a".repeat(64),
            target_kernel_id: "target".into(),
            provider: "claude".into(),
            account_profile: "work".into(),
            label: "Work".into(),
        }],
    };
    let mut metadata: chariox_relay::protocol::RelayProviderAccountSummary =
        serde_json::from_value(serde_json::json!({
            "provider":"claude", "state":"authenticated", "account_id":"work", "alias":"Work"
        }))
        .unwrap();
    assert!(
        !metadata.copyable,
        "legacy metadata never implies transfer support"
    );
    assert!(serde_json::to_value(&metadata)
        .unwrap()
        .get("copyable")
        .is_none());
    metadata.copyable = true;
    let snapshot = serde_json::json!([list, receive, scoped, peer, offers, metadata]);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "77431d2ab46f1608599ac6b2f2e9db738307f09d6140da81b9fc79cb1ff28e51"
    );
}
