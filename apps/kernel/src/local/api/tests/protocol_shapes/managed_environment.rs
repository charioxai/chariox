use super::*;
use crate::local::*;

#[test]
fn managed_reimage_receipt_read_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 345);
    let request =
        LocalDaemonRequest::GetManagedEnvironmentReimageReceipt(GetManagedEnvironmentRequest {
            environment_id: "environment-1".to_string(),
        });
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({
            "GetManagedEnvironmentReimageReceipt": { "environmentId": "environment-1" }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );
}

#[test]
fn local_daemon_managed_environment_control_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 345);
    let policy = ManagedEnvironmentAutoStopPolicy {
        minimum_runtime_seconds: 0,
        idle_delay_seconds: Some(900),
    };
    let context_input = ManagedEnvironmentContextPlanInput {
        source_target_id: None,
        kernel_context: ManagedEnvironmentKernelContextSelection::Empty,
        development_setup: ManagedEnvironmentDevelopmentSetup::Empty,
        provider_accounts: ManagedEnvironmentProviderAccounts::None,
        git_credentials: ManagedEnvironmentGitCredentials::None,
    };
    let source_context_input = ManagedEnvironmentContextPlanInput {
        source_target_id: Some("source-target-1".to_string()),
        kernel_context: ManagedEnvironmentKernelContextSelection::SourceKernel,
        development_setup: ManagedEnvironmentDevelopmentSetup::SourceProject {
            project_id: "project-1".to_string(),
            repositories: vec![
                ManagedEnvironmentRepositorySelection {
                    role: ManagedEnvironmentRepositoryRole::Primary,
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: Some("worktree-primary".to_string()),
                },
                ManagedEnvironmentRepositorySelection {
                    role: ManagedEnvironmentRepositoryRole::Supporting,
                    workspace_id: "workspace-supporting".to_string(),
                    worktree_id: None,
                },
            ],
        },
        provider_accounts: ManagedEnvironmentProviderAccounts::Selected {
            accounts: vec![ManagedEnvironmentProviderAccountSelection {
                provider: "codex".to_string(),
                account_profile: "work".to_string(),
            }],
        },
        git_credentials: ManagedEnvironmentGitCredentials::Selected {
            credential_ids: vec!["github-work".to_string()],
        },
    };
    let environment = managed_environment_summary(policy.clone(), context_input.clone());
    let mut source_environment =
        managed_environment_summary(policy.clone(), source_context_input.clone());
    source_environment.managed_repository_root = "/srv/chariox/repos".to_string();
    source_environment.context_plan.source = Some(ManagedEnvironmentContextSource {
        source_target_id: "source-target-1".to_string(),
        relay_realm_id: "realm-1".to_string(),
        machine_id: "machine-1".to_string(),
        kernel_id: "kernel-1".to_string(),
        key_thumbprint: "sha256:source-key".to_string(),
    });
    let operation = ManagedEnvironmentOperationSummary {
        operation_id: "operation-1".to_string(),
        environment_id: "environment-1".to_string(),
        requested_by_user_id: "user-1".to_string(),
        kind: ManagedEnvironmentOperationKind::Create,
        idempotency_key: "create-1".to_string(),
        request_digest: format!("sha256:{}", "c".repeat(64)),
        desired_revision: 1,
        status: ManagedEnvironmentOperationStatus::Pending,
        attempt: 0,
        retryable: false,
        failure_code: None,
        failure_message: None,
        completed_at: None,
        created_at: "2026-08-21T00:00:00.000Z".to_string(),
        updated_at: "2026-08-21T00:00:00.000Z".to_string(),
    };
    let reimage_operation = ManagedEnvironmentOperationSummary {
        operation_id: "operation-reimage-1".to_string(),
        environment_id: "environment-1".to_string(),
        requested_by_user_id: "user-1".to_string(),
        kind: ManagedEnvironmentOperationKind::Reimage,
        idempotency_key: "reimage-1".to_string(),
        request_digest: format!("sha256:{}", "d".repeat(64)),
        desired_revision: 2,
        status: ManagedEnvironmentOperationStatus::Pending,
        attempt: 0,
        retryable: false,
        failure_code: None,
        failure_message: None,
        completed_at: None,
        created_at: "2026-08-21T00:00:00.000Z".to_string(),
        updated_at: "2026-08-21T00:00:00.000Z".to_string(),
    };
    let reimage_receipt = ManagedEnvironmentReimageReceipt {
        receipt_id: "receipt-reimage-1".to_string(),
        environment_id: "environment-1".to_string(),
        operation_id: "operation-reimage-1".to_string(),
        previous_generation: 1,
        generation: 2,
        status: ManagedEnvironmentReimageReceiptStatus::Pending,
        fresh_equivalent: false,
        provider_server_id: "123456789".to_string(),
        previous_provider_image_id: Some("987654321".to_string()),
        provider_image_id: "987654321".to_string(),
        provider_profile_id: "hetzner-path1".to_string(),
        provider_profile_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_release_digest: format!("sha256:{}", "e".repeat(64)),
        old_machine_id: Some("managed-machine-1".to_string()),
        new_machine_id: None,
        old_kernel_id: Some("managed-kernel-1".to_string()),
        new_kernel_id: None,
        old_relay_realm_id: Some("realm-1".to_string()),
        new_relay_realm_id: Some("realm-2".to_string()),
        old_relay_target_id: Some("target-1".to_string()),
        new_relay_target_id: None,
        old_bootstrap_grant_id: Some("grant-1".to_string()),
        new_bootstrap_grant_id: None,
        old_credential_ids: serde_json::json!(["credential-1"]),
        new_credential_ids: serde_json::json!([]),
        runtime_evidence: serde_json::json!({"generation": 2}),
        source_evidence: serde_json::json!({
            "providerProfileId": "hetzner-path1",
            "providerProfileDigest": format!("sha256:{}", "b".repeat(64)),
            "providerImageId": "987654321",
            "runtimeReleaseDigest": format!("sha256:{}", "e".repeat(64)),
            "runtimeSourceCommit": "c".repeat(40),
            "runtimeSourceTree": "d".repeat(40),
        }),
        residue_checks: serde_json::json!({"oldGenerationFenced": true}),
        revocations: serde_json::json!({"machineId": "managed-machine-1"}),
        billing_observation: serde_json::json!({"provider": "hetzner"}),
        resource_observation: serde_json::json!({"providerServerId": "123456789"}),
        cleanup_state: serde_json::json!({"oldGenerationFenced": true}),
        rollback_state: serde_json::json!({"state": "fail_closed"}),
        receipt_digest: None,
        failure_code: None,
        failure_message: None,
        requested_at: "2026-08-21T00:00:00.000Z".to_string(),
        completed_at: None,
        created_at: "2026-08-21T00:00:00.000Z".to_string(),
        updated_at: "2026-08-21T00:00:00.000Z".to_string(),
    };
    // The receipt payload below is already covered by the fixed managed-control
    // snapshot hash. Lock the new read-only response wrapper to that same payload.
    let receipt_read_response = LocalDaemonResponse::ManagedEnvironmentReimageReceipt {
        receipt: reimage_receipt.clone(),
    };
    assert_eq!(
        serde_json::to_value(&receipt_read_response).unwrap(),
        serde_json::json!({
            "ManagedEnvironmentReimageReceipt": { "receipt": reimage_receipt.clone() }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(
            serde_json::to_value(&receipt_read_response).unwrap()
        )
        .unwrap(),
        receipt_read_response
    );
    let transfer_ticket = crate::managed_context::outbound_service::ManagedContextTransferTicket {
        environment_id: "environment-1".to_string(),
        context_plan: crate::managed_bootstrap::ManagedKernelContextPlan::source_project_for_tests(
            "context-1",
            "realm-1",
            "kernel-1",
            "sha256:source-key",
            "project-1",
        ),
        target: crate::managed_context::outbound_service::ManagedContextTransferTarget {
            relay_realm_id: "realm-1".to_string(),
            machine_id: "managed-machine-1".to_string(),
            kernel_id: "managed-kernel-1".to_string(),
            relay_public_key: "target-public-key".to_string(),
            key_thumbprint: "sha256:target-key".to_string(),
        },
    };
    let snapshot = serde_json::json!([
        LocalDaemonRequest::ListManagedEnvironmentCatalog(ListManagedEnvironmentCatalogRequest,),
        LocalDaemonRequest::CreateManagedEnvironment(CreateManagedEnvironmentRequest {
            client_request_id: "create-1".to_string(),
            name: "Managed agent".to_string(),
            region: "hel1".to_string(),
            compute_class: "agent-small".to_string(),
            managed_repository_root: None,
            auto_stop_policy: policy.clone(),
            context_plan: context_input.clone(),
        }),
        LocalDaemonRequest::CreateManagedEnvironment(CreateManagedEnvironmentRequest {
            client_request_id: "create-source-1".to_string(),
            name: "Managed source agent".to_string(),
            region: "fsn1".to_string(),
            compute_class: "agent-medium".to_string(),
            managed_repository_root: Some("/srv/chariox/repos".to_string()),
            auto_stop_policy: ManagedEnvironmentAutoStopPolicy {
                minimum_runtime_seconds: 300,
                idle_delay_seconds: None,
            },
            context_plan: source_context_input,
        }),
        LocalDaemonRequest::GetManagedEnvironment(GetManagedEnvironmentRequest {
            environment_id: "environment-1".to_string(),
        }),
        LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(
            PrepareManagedEnvironmentContextTransferRequest {
                environment_id: "environment-1".to_string(),
            },
        ),
        LocalDaemonRequest::PrepareManagedEnvironmentGitCredentialEnrollment(
            PrepareManagedEnvironmentGitCredentialEnrollmentRequest {
                environment_id: "environment-1".to_string(),
                source_target_id: "source-target-1".to_string(),
                git_credentials: ManagedEnvironmentGitCredentials::Selected {
                    credential_ids: vec!["github-work".to_string()],
                },
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Start,
                idempotency_key: "start-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Stop,
                idempotency_key: "stop-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Restart,
                idempotency_key: "restart-1".to_string(),
            },
        ),
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
            RequestManagedEnvironmentLifecycleRequest {
                environment_id: "environment-1".to_string(),
                action: ManagedEnvironmentLifecycleAction::Delete,
                idempotency_key: "delete-1".to_string(),
            },
        ),
        LocalDaemonResponse::ManagedEnvironmentCatalog {
            catalog: ManagedEnvironmentCatalog {
                compute_classes: vec![ManagedEnvironmentComputeClassOption {
                    compute_class: "agent-small".to_string(),
                    regions: vec!["hel1".to_string()],
                }],
                context_sources: vec![],
                environments: vec![environment.clone(), source_environment.clone()],
            },
        },
        LocalDaemonResponse::ManagedEnvironment {
            environment: source_environment.clone(),
        },
        LocalDaemonResponse::ManagedEnvironmentContextTransferPrepared {
            ticket: transfer_ticket,
        },
        LocalDaemonResponse::ManagedEnvironmentCreated {
            result: ManagedEnvironmentResult {
                environment: environment.clone(),
                operation: operation.clone(),
            },
        },
        LocalDaemonResponse::ManagedEnvironmentLifecycleRequested {
            result: ManagedEnvironmentResult {
                environment: source_environment,
                operation,
            },
        },
        LocalDaemonRequest::RequestManagedEnvironmentReimage(
            RequestManagedEnvironmentReimageRequest {
                environment_id: "environment-1".to_string(),
                expected_generation: 1,
                expected_provider_server_id: "123456789".to_string(),
                expected_provider_image_id: "987654321".to_string(),
                expected_provider_profile_id: "hetzner-path1".to_string(),
                expected_provider_profile_digest: format!("sha256:{}", "b".repeat(64)),
                expected_runtime_release_digest: format!("sha256:{}", "e".repeat(64)),
                expected_runtime_source_commit: "c".repeat(40),
                expected_runtime_source_tree: "d".repeat(40),
                context_plan: context_input,
                idempotency_key: "reimage-1".to_string(),
            },
        ),
        LocalDaemonResponse::ManagedEnvironmentReimageRequested {
            result: ManagedEnvironmentReimageResult {
                environment: environment.clone(),
                operation: reimage_operation,
                receipt: reimage_receipt,
            },
        },
        serde_json::json!({
            "desiredStates": [
                ManagedEnvironmentDesiredState::Running,
                ManagedEnvironmentDesiredState::Stopped,
                ManagedEnvironmentDesiredState::Deleted,
            ],
            "observedStates": [
                ManagedEnvironmentObservedState::Requested,
                ManagedEnvironmentObservedState::Provisioning,
                ManagedEnvironmentObservedState::Bootstrapping,
                ManagedEnvironmentObservedState::AwaitingContext,
                ManagedEnvironmentObservedState::Ready,
                ManagedEnvironmentObservedState::Starting,
                ManagedEnvironmentObservedState::Stopping,
                ManagedEnvironmentObservedState::Stopped,
                ManagedEnvironmentObservedState::Deleting,
                ManagedEnvironmentObservedState::Deleted,
                ManagedEnvironmentObservedState::Failed,
            ],
            "operationKinds": [
                ManagedEnvironmentOperationKind::Create,
                ManagedEnvironmentOperationKind::Start,
                ManagedEnvironmentOperationKind::Stop,
                ManagedEnvironmentOperationKind::Restart,
                ManagedEnvironmentOperationKind::Delete,
                ManagedEnvironmentOperationKind::Reimage,
            ],
            "operationStatuses": [
                ManagedEnvironmentOperationStatus::Pending,
                ManagedEnvironmentOperationStatus::Running,
                ManagedEnvironmentOperationStatus::Succeeded,
                ManagedEnvironmentOperationStatus::Failed,
            ],
        }),
    ]);

    assert_eq!(
        snapshot.pointer("/1/CreateManagedEnvironment/contextPlan/kernelContext"),
        Some(&serde_json::json!("empty"))
    );
    assert!(
        snapshot
            .pointer("/1/CreateManagedEnvironment/managedRepositoryRoot")
            .is_none(),
        "omitted roots preserve the Cloud default"
    );
    assert_eq!(
        snapshot.pointer("/2/CreateManagedEnvironment/managedRepositoryRoot"),
        Some(&serde_json::json!("/srv/chariox/repos"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/CreateManagedEnvironment/contextPlan/developmentSetup/repositories/1/role"
        ),
        Some(&serde_json::json!("supporting"))
    );
    assert_eq!(
        snapshot.pointer(
            "/2/CreateManagedEnvironment/contextPlan/providerAccounts/accounts/0/accountProfile"
        ),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/10/ManagedEnvironmentCatalog/catalog/computeClasses/0/computeClass"),
        Some(&serde_json::json!("agent-small"))
    );
    assert_eq!(
        snapshot.pointer("/14/ManagedEnvironmentLifecycleRequested/result/operation/status"),
        Some(&serde_json::json!("pending"))
    );
    assert_eq!(
        snapshot.pointer("/15/RequestManagedEnvironmentReimage/expectedGeneration"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/15/RequestManagedEnvironmentReimage/contextPlan/kernelContext"),
        Some(&serde_json::json!("empty"))
    );
    assert_eq!(
        snapshot.pointer("/16/ManagedEnvironmentReimageRequested/result/receipt/providerProfileId"),
        Some(&serde_json::json!("hetzner-path1"))
    );
    assert_eq!(
        snapshot.pointer("/10/ManagedEnvironmentCatalog/catalog/environments/0/runtimeKernelId"),
        Some(&serde_json::json!("managed-kernel-1"))
    );
    assert_eq!(
        snapshot
            .pointer("/10/ManagedEnvironmentCatalog/catalog/environments/0/managedRepositoryRoot"),
        Some(&serde_json::json!("/home/chariox"))
    );
    assert_eq!(
        snapshot
            .pointer("/10/ManagedEnvironmentCatalog/catalog/environments/1/managedRepositoryRoot"),
        Some(&serde_json::json!("/srv/chariox/repos"))
    );
    assert_eq!(
        snapshot.pointer("/12/ManagedEnvironmentContextTransferPrepared/ticket/target/kernelId"),
        Some(&serde_json::json!("managed-kernel-1"))
    );
    assert_eq!(
        snapshot.pointer(
            "/5/PrepareManagedEnvironmentGitCredentialEnrollment/gitCredentials/credentialIds/0"
        ),
        Some(&serde_json::json!("github-work"))
    );
    let mut previous_shape = snapshot.clone();
    remove_managed_repository_root_fields(&mut previous_shape);
    let previous_serialized = serde_json::to_string(&previous_shape)
        .expect("managed environment shape without the protocol 342 addition");
    assert_eq!(
        format!("{:x}", Sha256::digest(previous_serialized.as_bytes())),
        "53f9fb27de875d36256fc9c44092350d6e69dfbbe6b0b2649a2d850d6bc9c46d"
    );
    let root_projection = serde_json::json!({
        "omittedCreateRoot": snapshot.pointer("/1/CreateManagedEnvironment/managedRepositoryRoot"),
        "customCreateRoot": snapshot.pointer("/2/CreateManagedEnvironment/managedRepositoryRoot"),
        "defaultSummaryRoot": snapshot.pointer(
            "/10/ManagedEnvironmentCatalog/catalog/environments/0/managedRepositoryRoot"
        ),
        "customSummaryRoot": snapshot.pointer(
            "/10/ManagedEnvironmentCatalog/catalog/environments/1/managedRepositoryRoot"
        ),
    });
    let root_serialized =
        serde_json::to_string(&root_projection).expect("managed repository root protocol shape");
    assert_eq!(
        format!("{:x}", Sha256::digest(root_serialized.as_bytes())),
        "5d6eccb89e50875e842c507c6147a78c938014c77feeee063bb584c00be69404"
    );
}

#[test]
fn local_daemon_reimage_request_rejects_missing_context_plan() {
    let request = serde_json::json!({
        "RequestManagedEnvironmentReimage": {
            "environmentId": "environment-1",
            "expectedGeneration": 1,
            "expectedProviderServerId": "123456789",
            "expectedProviderImageId": "987654321",
            "expectedProviderProfileId": "hetzner-path1",
            "expectedProviderProfileDigest": format!("sha256:{}", "b".repeat(64)),
            "expectedRuntimeReleaseDigest": format!("sha256:{}", "e".repeat(64)),
            "expectedRuntimeSourceCommit": "c".repeat(40),
            "expectedRuntimeSourceTree": "d".repeat(40),
            "idempotencyKey": "reimage-1",
        }
    });

    let error = serde_json::from_value::<LocalDaemonRequest>(request)
        .expect_err("reimage contextPlan must be required");
    assert!(error.to_string().contains("contextPlan"));
}

#[test]
fn local_daemon_reimage_request_rejects_repository_root_override() {
    let request = serde_json::json!({
        "RequestManagedEnvironmentReimage": {
            "environmentId": "environment-1",
            "expectedGeneration": 1,
            "expectedProviderServerId": "123456789",
            "expectedProviderImageId": "987654321",
            "expectedProviderProfileId": "hetzner-path1",
            "expectedProviderProfileDigest": format!("sha256:{}", "b".repeat(64)),
            "expectedRuntimeReleaseDigest": format!("sha256:{}", "e".repeat(64)),
            "expectedRuntimeSourceCommit": "c".repeat(40),
            "expectedRuntimeSourceTree": "d".repeat(40),
            "contextPlan": {
                "sourceTargetId": null,
                "kernelContext": "empty",
                "developmentSetup": { "kind": "empty" },
                "providerAccounts": { "kind": "none" },
                "gitCredentials": { "kind": "none" },
            },
            "managedRepositoryRoot": "/tmp/override",
            "idempotencyKey": "reimage-1",
        }
    });

    let error = serde_json::from_value::<LocalDaemonRequest>(request)
        .expect_err("reimage must not accept a repository-root override");
    assert!(error.to_string().contains("managedRepositoryRoot"));
}

#[test]
fn local_daemon_reimage_preflight_shape_is_versioned_and_allowlisted() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 345);
    let preflight = ManagedEnvironmentReimagePreflight {
        environment_id: "environment-1".to_string(),
        retained: ManagedEnvironmentReimagePreflightRetained {
            provider_server_id: "123456789".to_string(),
            generation: 3,
            desired_revision: 7,
            observed_revision: 7,
            runtime_machine_id: "managed-machine-3".to_string(),
            runtime_kernel_id: "managed-kernel-3".to_string(),
            runtime_relay_realm_id: "managed-realm-3".to_string(),
            runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
        },
        desired_release: ManagedEnvironmentReimagePreflightDesiredRelease {
            provider_id: ManagedEnvironmentReimageProviderId::Hetzner,
            provider_image_id: "987654321".to_string(),
            provider_profile_id: "hetzner-path1".to_string(),
            provider_profile_digest: format!("sha256:{}", "b".repeat(64)),
            runtime_release_digest: format!("sha256:{}", "c".repeat(64)),
            runtime_source_commit: "d".repeat(40),
            runtime_source_tree: "e".repeat(40),
        },
    };
    let snapshot = serde_json::json!([
        LocalDaemonRequest::GetManagedEnvironmentReimagePreflight(
            GetManagedEnvironmentReimagePreflightRequest {
                environment_id: "environment-1".to_string(),
            },
        ),
        LocalDaemonResponse::ManagedEnvironmentReimagePreflight {
            preflight: preflight.clone(),
        },
    ]);

    assert_eq!(
        snapshot.pointer("/0/GetManagedEnvironmentReimagePreflight/environmentId"),
        Some(&serde_json::json!("environment-1"))
    );
    assert_eq!(
        snapshot
            .pointer("/1/ManagedEnvironmentReimagePreflight/preflight/retained/providerServerId"),
        Some(&serde_json::json!("123456789"))
    );
    assert_eq!(
        snapshot.pointer(
            "/1/ManagedEnvironmentReimagePreflight/preflight/desiredRelease/providerImageId"
        ),
        Some(&serde_json::json!("987654321"))
    );
    assert!(snapshot
        .pointer("/1/ManagedEnvironmentReimagePreflight/preflight/retained/providerImageId")
        .is_none());
    let serialized = serde_json::to_string(&snapshot).expect("reimage preflight shape");
    // Hash serde_json::Value's sorted object keys, not a JS insertion-order reconstruction.
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "c9c26926c17a881930bfc262d29c55db1bee1450aa51eea49ea94aa98f623d5c"
    );
}

#[test]
fn local_daemon_pre_reimage_observation_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 345);
    let snapshot = serde_json::json!([
        LocalDaemonRequest::ObserveManagedEnvironmentPreReimage(
            ObserveManagedEnvironmentPreReimageRequest {
                environment_id: "environment-1".to_string(),
                expected_generation: 1,
            },
        ),
        LocalDaemonResponse::ManagedEnvironmentPreReimageObserved {
            acknowledgement: ManagedEnvironmentPreReimageObservationAcknowledgement {
                environment_id: "environment-1".to_string(),
                generation: 1,
                observed_at: "2026-09-22T01:02:03.000Z".to_string(),
            },
        },
    ]);
    assert_eq!(
        snapshot.pointer("/0/ObserveManagedEnvironmentPreReimage/expectedGeneration"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        snapshot.pointer("/1/ManagedEnvironmentPreReimageObserved/acknowledgement/observedAt"),
        Some(&serde_json::json!("2026-09-22T01:02:03.000Z"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("pre-reimage observation shape");
    assert_eq!(
        format!("{:x}", Sha256::digest(serialized.as_bytes())),
        "f4b1b31a6a0aa345f55c9bb1705095d9c0aed67846c93540f02c2eefd2dd2816"
    );
}

fn managed_environment_summary(
    policy: ManagedEnvironmentAutoStopPolicy,
    context_input: ManagedEnvironmentContextPlanInput,
) -> ManagedEnvironmentSummary {
    ManagedEnvironmentSummary {
        environment_id: "environment-1".to_string(),
        account_id: "account-1".to_string(),
        created_by_user_id: "user-1".to_string(),
        name: "Managed agent".to_string(),
        region: "hel1".to_string(),
        compute_class: "agent-small".to_string(),
        managed_repository_root: "/home/chariox".to_string(),
        desired_state: ManagedEnvironmentDesiredState::Running,
        observed_state: ManagedEnvironmentObservedState::Requested,
        desired_revision: 1,
        observed_revision: 0,
        runtime_machine_id: Some("managed-machine-1".to_string()),
        runtime_kernel_id: Some("managed-kernel-1".to_string()),
        runtime_release_digest: None,
        context_plan: ManagedEnvironmentContextPlan {
            schema_version: 1,
            context_id: "context-1".to_string(),
            plan_digest: format!("sha256:{}", "a".repeat(64)),
            source: None,
            kernel_context: context_input.kernel_context,
            development_setup: context_input.development_setup,
            provider_accounts: context_input.provider_accounts,
            git_credentials: context_input.git_credentials,
        },
        context_manifest_digest: None,
        auto_stop_policy: policy,
        last_error_code: None,
        last_error_message: None,
        created_at: "2026-08-21T00:00:00.000Z".to_string(),
        updated_at: "2026-08-21T00:00:00.000Z".to_string(),
    }
}

fn remove_managed_repository_root_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                remove_managed_repository_root_fields(item);
            }
        }
        serde_json::Value::Object(fields) => {
            fields.remove("managedRepositoryRoot");
            for item in fields.values_mut() {
                remove_managed_repository_root_fields(item);
            }
        }
        _ => {}
    }
}
