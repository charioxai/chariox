use super::*;
use crate::local::*;

#[test]
fn local_daemon_runtime_identity_observation_shape_is_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let snapshot = serde_json::json!([
        LocalDaemonRequest::ObserveKernelRuntimeIdentity(
            ObserveKernelRuntimeIdentityRequest {
                environment_id: "environment-1".to_string(),
                expected_generation: 1,
            },
        ),
        LocalDaemonResponse::KernelRuntimeIdentityObserved {
            acknowledgement: KernelRuntimeIdentityObservationAcknowledgement {
                environment_id: "environment-1".to_string(),
                generation: 1,
                observed_at: "2026-09-22T01:02:03.000Z".to_string(),
            },
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/ObserveKernelRuntimeIdentity/expectedGeneration"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/1/KernelRuntimeIdentityObserved/acknowledgement/observedAt"),
        Some(&serde_json::json!("2026-09-22T01:02:03.000Z"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("pre-reimage observation shape");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "f4b1b31a6a0aa345f55c9bb1705095d9c0aed67846c93540f02c2eefd2dd2816"
    );
}

// OSS boundary: the human Cloud session, rental and disposable-worker controls were retired in
// favour of the connected service's own client API; the kernel no longer knows their names.
#[test]
fn retired_cloud_controls_are_not_on_the_wire() {
    for retired in [
        serde_json::json!({"PairCloudRelayClient": {"clientId": "c"}}),
        serde_json::json!({"CreateCloudSessionInvite": {}}),
        serde_json::json!({"ListCloudCollaborators": null}),
        serde_json::json!({"ListManagedEnvironmentCatalog": null}),
        serde_json::json!({"CreateDisposableWorker": {}}),
        serde_json::json!({"GetManagedEnvironment": {"environmentId": "e"}}),
        serde_json::json!({"RequestManagedEnvironmentReimage": {}}),
        serde_json::json!({"PrepareManagedEnvironmentGitCredentialEnrollment": {}}),
    ] {
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(retired.clone()).is_err(),
            "{retired} is still a local request"
        );
    }
}

#[test]
fn provider_account_portability_preflight_protocol_478_is_acknowledgement_only() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request_json = serde_json::json!({"PreflightProviderAccountPortability": {
        "providerAccounts": {"kind": "selected", "accounts": [{"provider": "codex", "accountProfile": "synthetic-profile"}]}
    }});
    let request: LocalDaemonRequest =
        serde_json::from_value(request_json.clone()).expect("selection-only kernel preflight");
    assert_eq!(serde_json::to_value(request).unwrap(), request_json);
    let response_json = serde_json::json!({"ProviderAccountPortabilityPreflightPassed": {}});
    let response: LocalDaemonResponse =
        serde_json::from_value(response_json.clone()).expect("acknowledgement-only response");
    assert_eq!(serde_json::to_value(response).unwrap(), response_json);
    let snapshot = serde_json::json!([request_json, response_json]);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "0fdaa3cbf78bfba9b1e58d2164fdcf5471ff8e989d064e2614be8e8fbcd27115"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"PreflightProviderAccountPortability": {
            "providerAccounts": {"kind": "none"}, "credential": "forbidden"
        }})
    )
    .is_err());
}

// MP-10 / MP-11: preflight and transfer start carry one optional launch key; without it the
// shapes are the ones above, so the pinned hashes stay.
#[test]
fn preflight_and_transfer_start_carry_an_optional_launch_key() {
    let preflight = serde_json::json!({"PreflightProviderAccountPortability": {
        "providerAccounts": {"kind": "none"}, "launchId": "launch-1"
    }});
    let parsed: LocalDaemonRequest = serde_json::from_value(preflight.clone()).unwrap();
    let LocalDaemonRequest::PreflightProviderAccountPortability(request) = &parsed else {
        panic!("preflight request");
    };
    assert_eq!(request.launch_id.as_deref(), Some("launch-1"));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), preflight);

    let start = serde_json::json!({"StartManagedContextTransfer": {
        "interactive": true, "launchId": "launch-1"
    }});
    let parsed: LocalDaemonRequest = serde_json::from_value(start.clone()).unwrap();
    let LocalDaemonRequest::StartManagedContextTransfer(request) = &parsed else {
        panic!("transfer start request");
    };
    assert_eq!(request.launch_id.as_deref(), Some("launch-1"));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), start);

    let without = serde_json::to_value(LocalDaemonRequest::StartManagedContextTransfer(Box::new(
        StartManagedContextTransferRequest {
            ticket: None,
            owner_managed: None,
            interactive: true,
            launch_id: None,
        },
    )))
    .unwrap();
    assert_eq!(
        without,
        serde_json::json!({"StartManagedContextTransfer": {"interactive": true}})
    );
}
