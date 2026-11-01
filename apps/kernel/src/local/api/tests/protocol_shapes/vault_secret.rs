use super::*;

// Local 929: an outside agent asks the owner for a Vault value and learns the link and the outcome, never the value.
#[test]
fn vault_secret_request_929_shapes_are_pinned() {
    const { assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 929) };
    let request = LocalDaemonRequest::RequestVaultSecret(crate::local::RequestVaultSecretRequest {
        credential_id: "drill_key".into(),
        label: "Drill key".into(),
        overwrite: false,
    });
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"RequestVaultSecret": {"credential_id": "drill_key", "label": "Drill key", "overwrite": false}})
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"RequestVaultSecret": {"credential_id": "k", "label": "l", "value": "typed"}})
    )
    .is_err());
    let status =
        LocalDaemonRequest::GetVaultSecretStatus(crate::local::GetVaultSecretStatusRequest {
            interaction_id: "secret-request-1".into(),
        });
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        serde_json::json!({"GetVaultSecretStatus": {"interaction_id": "secret-request-1"}})
    );
    let requested = LocalDaemonResponse::VaultSecretRequested {
        interaction_id: "secret-request-1".into(),
        link: "https://page.test/s/kernel/secret-request-1".into(),
        expires_at: 7,
        credential_id: "drill_key".into(),
    };
    assert_eq!(
        serde_json::to_value(&requested).unwrap(),
        serde_json::json!({"VaultSecretRequested": {
            "interaction_id": "secret-request-1", "link": "https://page.test/s/kernel/secret-request-1",
            "expires_at": 7, "credential_id": "drill_key"
        }})
    );
    for (state, wire) in [
        (crate::local::VaultSecretStatus::Pending, "pending"),
        (crate::local::VaultSecretStatus::Stored, "stored"),
        (crate::local::VaultSecretStatus::Cancelled, "cancelled"),
        (crate::local::VaultSecretStatus::TimedOut, "timed_out"),
    ] {
        assert_eq!(
            serde_json::to_value(LocalDaemonResponse::VaultSecretStatusReported {
                interaction_id: "i".into(),
                credential_id: "c".into(),
                status: state,
            })
            .unwrap(),
            serde_json::json!({"VaultSecretStatusReported": {"interaction_id": "i", "credential_id": "c", "status": wire}})
        );
    }
    // The popup every owner terminal shows for it.
    let prompt: crate::local::PasskeyPrompt = serde_json::from_value(serde_json::json!({
        "kind": "vault_secret", "session_id": "kernel-access", "interaction_id": "secret-request-1",
        "title": "Value for the Vault", "message": "Display only", "approve_choice_id": "secret",
        "refuse_choice_id": "cancel", "requested_at_ms": 1000, "expires_at_ms": 601000
    }))
    .unwrap();
    assert_eq!(prompt.kind, crate::local::PasskeyPromptKind::VaultSecret);
}
