use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonRequest, LocalDaemonResponse, ManagedEnvironmentCatalog,
    ManagedEnvironmentOperationKind, ManagedEnvironmentProviderAccounts,
    ManagedEnvironmentReimagePreflight, ManagedEnvironmentReimageResult,
    RequestManagedEnvironmentReimageRequest,
};
use crate::runtime::cloud_api_client::{
    cloud_url_component, get_cloud_json_authenticated, post_cloud_json_authenticated,
};

mod cloud_contract;
use cloud_contract::{
    EnvironmentDetailsResponse, EnvironmentResult, EnvironmentsResponse, OptionsResponse,
    ReimageReceipt, ReimageResult,
};

pub(crate) async fn execute_managed_environment_control_request(
    config: DaemonConfig,
    provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
    outbound_store: crate::managed_context::outbound_service::ManagedContextOutboundOperationStore,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let cloud = authorized_cloud_profile(&config, caller_user_id)?;
    let token = cloud
        .cloud_session_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| control_error("Cloud session is unavailable"))?;
    let account_id = cloud.account_id.trim();
    if account_id.is_empty() {
        return Err(control_error("Cloud account is unavailable"));
    }
    let account_query = format!("accountId={}", cloud_url_component(account_id));

    match request {
        LocalDaemonRequest::ListManagedEnvironmentCatalog(_) => {
            let options_path = format!("/managed-environments/options?{account_query}");
            let environments_path = format!("/managed-environments?{account_query}");
            let (options, environments) = tokio::try_join!(
                get_cloud_json_authenticated::<OptionsResponse>(
                    cloud.api_url.clone(),
                    options_path,
                    token.to_string(),
                ),
                get_cloud_json_authenticated::<EnvironmentsResponse>(
                    cloud.api_url.clone(),
                    environments_path,
                    token.to_string(),
                ),
            )?;
            Ok(LocalDaemonResponse::ManagedEnvironmentCatalog {
                catalog: ManagedEnvironmentCatalog {
                    compute_classes: options
                        .compute_classes
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    context_sources: options
                        .context_sources
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    environments: environments
                        .environments
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                },
            })
        }
        LocalDaemonRequest::GetManagedEnvironment(request) => {
            let path = format!(
                "/managed-environments/{}?{account_query}",
                cloud_url_component(&request.environment_id),
            );
            let response = get_cloud_json_authenticated::<EnvironmentDetailsResponse>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
            )
            .await?;
            Ok(LocalDaemonResponse::ManagedEnvironment {
                environment: response.environment.into(),
                operations: response.operations.into_iter().map(Into::into).collect(),
            })
        }
        LocalDaemonRequest::GetManagedEnvironmentReimagePreflight(request) => {
            let path = format!(
                "/managed-environments/{}/reimage/preflight?{account_query}",
                cloud_url_component(&request.environment_id),
            );
            let preflight = get_cloud_json_authenticated::<ManagedEnvironmentReimagePreflight>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
            )
            .await?;
            if preflight.environment_id != request.environment_id {
                return Err(control_error(
                    "Cloud returned reimage preflight for another managed environment",
                ));
            }
            Ok(LocalDaemonResponse::ManagedEnvironmentReimagePreflight { preflight })
        }
        LocalDaemonRequest::GetManagedEnvironmentReimageReceipt(request) => {
            let path = format!(
                "/managed-environments/{}/reimage/receipt?{account_query}",
                cloud_url_component(&request.environment_id),
            );
            let receipt: crate::local::ManagedEnvironmentReimageReceipt =
                get_cloud_json_authenticated::<ReimageReceipt>(
                    cloud.api_url.clone(),
                    path,
                    token.to_string(),
                )
                .await?
                .into();
            if receipt.environment_id != request.environment_id {
                return Err(control_error(
                    "Cloud returned reimage receipt for another managed environment",
                ));
            }
            Ok(LocalDaemonResponse::ManagedEnvironmentReimageReceipt { receipt })
        }
        LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(request) => {
            let path = format!(
                "/managed-environments/{}/context-transfer?{account_query}",
                cloud_url_component(&request.environment_id),
            );
            let response = get_cloud_json_authenticated::<cloud_contract::ContextTransferTicket>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
            )
            .await?;
            let ticket = response.into_ticket()?;
            if ticket.environment_id != request.environment_id {
                return Err(control_error(
                    "Cloud returned a managed-context ticket for another environment",
                ));
            }
            crate::managed_context::outbound_service::validate_ticket(&config, &ticket)?;
            Ok(LocalDaemonResponse::ManagedEnvironmentContextTransferPrepared { ticket })
        }
        LocalDaemonRequest::PrepareManagedEnvironmentGitCredentialEnrollment(request) => {
            let crate::local::ManagedEnvironmentGitCredentials::Selected { credential_ids } =
                &request.git_credentials
            else {
                return Err(control_error(
                    "Git credential enrollment requires an explicit credential selection",
                ));
            };
            let selection =
                crate::managed_context::package::ManagedContextGitCredentialSelection::Selected {
                    credential_ids: credential_ids.clone(),
                };
            crate::managed_context::scm::validate_selection(&selection)?;
            let path = format!(
                "/managed-environments/{}/git-credential-enrollment",
                cloud_url_component(&request.environment_id),
            );
            let body = serde_json::json!({
                "accountId": account_id,
                "sourceTargetId": request.source_target_id,
                "gitCredentials": request.git_credentials,
            });
            let response = post_cloud_json_authenticated::<cloud_contract::ContextTransferTicket>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
                body,
            )
            .await?;
            let ticket = response.into_ticket()?;
            validate_git_credential_enrollment_ticket(&ticket, &request)?;
            crate::managed_context::outbound_service::validate_ticket(&config, &ticket)?;
            outbound_store.remember_prepared_git_enrollment_ticket(&ticket)?;
            Ok(LocalDaemonResponse::ManagedEnvironmentContextTransferPrepared { ticket })
        }
        LocalDaemonRequest::CreateManagedEnvironment(request) => {
            preflight_provider_account_exports(
                &config,
                cloud,
                &provider_account_profiles,
                &request.context_plan.provider_accounts,
            )?;
            let mut body =
                serde_json::to_value(request).map_err(|error| DaemonError::LocalTransport {
                    operation: "encode managed environment create request",
                    message: error.to_string(),
                })?;
            body.as_object_mut()
                .ok_or_else(|| control_error("managed environment create request is invalid"))?
                .insert(
                    "accountId".to_string(),
                    serde_json::Value::String(account_id.to_string()),
                );
            let result = post_cloud_json_authenticated::<EnvironmentResult>(
                cloud.api_url.clone(),
                "/managed-environments".to_string(),
                token.to_string(),
                body,
            )
            .await?;
            Ok(LocalDaemonResponse::ManagedEnvironmentCreated {
                result: result.into(),
            })
        }
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(request) => {
            let path = format!(
                "/managed-environments/{}/lifecycle",
                cloud_url_component(&request.environment_id),
            );
            let body = serde_json::json!({
                "accountId": account_id,
                "action": request.action,
                "idempotencyKey": request.idempotency_key,
            });
            let result = post_cloud_json_authenticated::<EnvironmentResult>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
                body,
            )
            .await?;
            Ok(LocalDaemonResponse::ManagedEnvironmentLifecycleRequested {
                result: result.into(),
            })
        }
        LocalDaemonRequest::RequestManagedEnvironmentReimage(request) => {
            validate_reimage_request(&request)?;
            preflight_provider_account_exports(
                &config,
                cloud,
                &provider_account_profiles,
                &request.context_plan.provider_accounts,
            )?;
            let path = format!(
                "/managed-environments/{}/reimage",
                cloud_url_component(&request.environment_id),
            );
            let body = serde_json::json!({
                "accountId": account_id,
                "expectedGeneration": request.expected_generation,
                "expectedProviderServerId": request.expected_provider_server_id,
                "expectedProviderImageId": request.expected_provider_image_id,
                "expectedProviderProfileId": request.expected_provider_profile_id,
                "expectedProviderProfileDigest": request.expected_provider_profile_digest,
                "expectedRuntimeReleaseDigest": request.expected_runtime_release_digest,
                "expectedRuntimeSourceCommit": request.expected_runtime_source_commit,
                "expectedRuntimeSourceTree": request.expected_runtime_source_tree,
                "contextPlan": request.context_plan,
                "idempotencyKey": request.idempotency_key,
            });
            let result: ManagedEnvironmentReimageResult =
                post_cloud_json_authenticated::<ReimageResult>(
                    cloud.api_url.clone(),
                    path,
                    token.to_string(),
                    body,
                )
                .await?
                .into();
            validate_reimage_result(&request, &result)?;
            Ok(LocalDaemonResponse::ManagedEnvironmentReimageRequested { result })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "managed environment control",
            message: "unsupported request".to_string(),
        }),
    }
}

fn preflight_provider_account_exports(
    config: &DaemonConfig,
    cloud: &PersistedCloudRelayProfile,
    provider_account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    selection: &ManagedEnvironmentProviderAccounts,
) -> Result<(), DaemonError> {
    let ManagedEnvironmentProviderAccounts::Selected { accounts } = selection else {
        return Ok(());
    };
    let owner_user_id = crate::account_profile::provider_account_authority_owner_user_id(
        config,
        cloud.user_id.as_str(),
    );
    for account in accounts {
        provider_account_profiles
            .export_managed_context_materialization(
                &owner_user_id,
                &account.provider,
                &account.account_profile,
            )
            .map_err(|_| {
                control_error(format!(
                    "selected {} provider account `{}` has no transferable credentials",
                    account.provider, account.account_profile
                ))
            })?;
    }
    Ok(())
}

fn authorized_cloud_profile<'a>(
    config: &'a DaemonConfig,
    caller_user_id: &str,
) -> Result<&'a PersistedCloudRelayProfile, DaemonError> {
    let cloud = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| control_error("kernel is not connected to Chariox Cloud"))?;
    if caller_user_id != cloud.user_id {
        return Err(control_error(
            "managed environment control belongs to another Cloud user",
        ));
    }
    Ok(cloud)
}

fn control_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed environment control",
        message: message.into(),
    }
}

fn validate_reimage_request(
    request: &RequestManagedEnvironmentReimageRequest,
) -> Result<(), DaemonError> {
    if !is_cloud_control_identifier(&request.environment_id)
        || !is_cloud_control_identifier(&request.idempotency_key)
        || request.expected_generation == 0
        || request.expected_generation > 9_007_199_254_740_991
        || !is_provider_resource_id(&request.expected_provider_server_id)
        || !is_provider_resource_id(&request.expected_provider_image_id)
        || !is_cloud_control_identifier(&request.expected_provider_profile_id)
        || !is_sha256_digest(&request.expected_provider_profile_digest)
        || !is_sha256_digest(&request.expected_runtime_release_digest)
        || !is_source_identity(&request.expected_runtime_source_commit)
        || !is_source_identity(&request.expected_runtime_source_tree)
    {
        return Err(control_error(
            "managed environment reimage request is invalid",
        ));
    }
    Ok(())
}

fn validate_reimage_result(
    request: &RequestManagedEnvironmentReimageRequest,
    result: &ManagedEnvironmentReimageResult,
) -> Result<(), DaemonError> {
    let expected_generation = request
        .expected_generation
        .checked_add(1)
        .ok_or_else(|| control_error("managed environment reimage generation is exhausted"))?;
    let receipt = &result.receipt;
    if result.environment.environment_id != request.environment_id
        || result.operation.environment_id != request.environment_id
        || result.operation.kind != ManagedEnvironmentOperationKind::Reimage
        || result.operation.idempotency_key != request.idempotency_key
        || receipt.environment_id != request.environment_id
        || receipt.operation_id != result.operation.operation_id
        || receipt.previous_generation != request.expected_generation
        || receipt.generation != expected_generation
        || receipt.receipt_id.trim().is_empty()
        || receipt.provider_server_id != request.expected_provider_server_id
        || receipt.provider_image_id != request.expected_provider_image_id
        || receipt.provider_profile_id != request.expected_provider_profile_id
        || receipt.provider_profile_digest != request.expected_provider_profile_digest
        || receipt.runtime_release_digest != request.expected_runtime_release_digest
        || !source_evidence_matches_request(&receipt.source_evidence, request)
    {
        return Err(control_error(
            "Cloud returned managed reimage evidence that does not match the requested generation or exact provider identity",
        ));
    }
    Ok(())
}

fn is_cloud_control_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 128
        || (!bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit())
    {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._:-".contains(byte))
}

fn is_provider_resource_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 18
        && bytes[0] != b'0'
        && bytes.iter().all(|byte| byte.is_ascii_digit())
}

fn is_source_identity(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(is_lower_hex_byte)
}

fn is_sha256_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(is_lower_hex_byte)
}

fn is_lower_hex_byte(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn source_evidence_matches_request(
    source_evidence: &serde_json::Value,
    request: &RequestManagedEnvironmentReimageRequest,
) -> bool {
    let Some(source_evidence) = source_evidence.as_object() else {
        return false;
    };
    source_evidence
        .get("providerProfileId")
        .and_then(serde_json::Value::as_str)
        == Some(request.expected_provider_profile_id.as_str())
        && source_evidence
            .get("providerProfileDigest")
            .and_then(serde_json::Value::as_str)
            == Some(request.expected_provider_profile_digest.as_str())
        && source_evidence
            .get("providerImageId")
            .and_then(serde_json::Value::as_str)
            == Some(request.expected_provider_image_id.as_str())
        && source_evidence
            .get("runtimeReleaseDigest")
            .and_then(serde_json::Value::as_str)
            == Some(request.expected_runtime_release_digest.as_str())
        && source_evidence
            .get("runtimeSourceCommit")
            .and_then(serde_json::Value::as_str)
            == Some(request.expected_runtime_source_commit.as_str())
        && source_evidence
            .get("runtimeSourceTree")
            .and_then(serde_json::Value::as_str)
            == Some(request.expected_runtime_source_tree.as_str())
}

fn validate_git_credential_enrollment_ticket(
    ticket: &crate::managed_context::outbound_service::ManagedContextTransferTicket,
    request: &crate::local::PrepareManagedEnvironmentGitCredentialEnrollmentRequest,
) -> Result<(), DaemonError> {
    let requested_selection = match &request.git_credentials {
        crate::local::ManagedEnvironmentGitCredentials::Selected { credential_ids } => {
            crate::managed_context::package::ManagedContextGitCredentialSelection::Selected {
                credential_ids: credential_ids.clone(),
            }
        }
        crate::local::ManagedEnvironmentGitCredentials::None => {
            return Err(control_error(
                "Git credential enrollment requires an explicit credential selection",
            ));
        }
    };
    let source_matches = ticket
        .context_plan
        .source_binding()
        .is_some_and(|source| source.source_target_id == request.source_target_id);
    let binding = ticket.context_plan.package_binding();
    if ticket.environment_id != request.environment_id
        || !ticket.context_plan.is_git_credential_enrollment()
        || !source_matches
        || binding.git_credentials != requested_selection
    {
        return Err(control_error(
            "Cloud returned a Git credential enrollment ticket that does not match the local selection",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        CreateManagedEnvironmentRequest, GetManagedEnvironmentReimagePreflightRequest,
        GetManagedEnvironmentRequest, ListManagedEnvironmentCatalogRequest,
        ManagedEnvironmentAutoStopPolicy, ManagedEnvironmentContextPlanInput,
        ManagedEnvironmentDevelopmentSetup, ManagedEnvironmentGitCredentials,
        ManagedEnvironmentKernelContextSelection, ManagedEnvironmentLifecycleAction,
        ManagedEnvironmentProviderAccountSelection, ManagedEnvironmentProviderAccounts,
        PrepareManagedEnvironmentContextTransferRequest,
        PrepareManagedEnvironmentGitCredentialEnrollmentRequest,
        RequestManagedEnvironmentLifecycleRequest, RequestManagedEnvironmentReimageRequest,
    };
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn empty_context_plan() -> ManagedEnvironmentContextPlanInput {
        ManagedEnvironmentContextPlanInput {
            source_target_id: None,
            kernel_context: ManagedEnvironmentKernelContextSelection::Empty,
            development_setup: ManagedEnvironmentDevelopmentSetup::Empty,
            provider_accounts: ManagedEnvironmentProviderAccounts::None,
            git_credentials: ManagedEnvironmentGitCredentials::None,
        }
    }

    fn selected_context_plan(
        provider_accounts: ManagedEnvironmentProviderAccounts,
    ) -> ManagedEnvironmentContextPlanInput {
        ManagedEnvironmentContextPlanInput {
            source_target_id: Some("source-target-test".to_string()),
            kernel_context: ManagedEnvironmentKernelContextSelection::SourceKernel,
            development_setup: ManagedEnvironmentDevelopmentSetup::SourceProject {
                project_id: "project-1".to_string(),
                repositories: vec![crate::local::ManagedEnvironmentRepositorySelection {
                    role: crate::local::ManagedEnvironmentRepositoryRole::Primary,
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: None,
                }],
            },
            provider_accounts,
            git_credentials: ManagedEnvironmentGitCredentials::None,
        }
    }

    fn reimage_request(
        context_plan: ManagedEnvironmentContextPlanInput,
    ) -> RequestManagedEnvironmentReimageRequest {
        RequestManagedEnvironmentReimageRequest {
            environment_id: "environment-1".to_string(),
            expected_generation: 1,
            expected_provider_server_id: "123456789".to_string(),
            expected_provider_image_id: "987654321".to_string(),
            expected_provider_profile_id: "hetzner-path1".to_string(),
            expected_provider_profile_digest: format!("sha256:{}", "b".repeat(64)),
            expected_runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
            expected_runtime_source_commit: "c".repeat(40),
            expected_runtime_source_tree: "d".repeat(40),
            context_plan,
            idempotency_key: "reimage-1".to_string(),
        }
    }

    #[test]
    fn managed_environment_control_rejects_callers_without_the_cloud_user_identity() {
        let mut config = DaemonConfig::for_tests();
        assert!(authorized_cloud_profile(&config, crate::session::DEFAULT_LOCAL_USER_ID).is_err());
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });

        authorized_cloud_profile(&config, "cloud-user-1").expect("Cloud owner");
        assert!(authorized_cloud_profile(&config, crate::session::DEFAULT_LOCAL_USER_ID).is_err());
        assert!(authorized_cloud_profile(&config, "cloud-user-2").is_err());
    }

    #[tokio::test]
    async fn managed_environment_reimage_preflight_requires_the_owner_cloud_session() {
        let server =
            ManagedEnvironmentCloudFixture::start(serde_json::Value::Null, serde_json::Value::Null);
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account-1".to_string(),
            user_id: "owner-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            machine_credential: Some(format!("mcred_{}", "a".repeat(40))),
            api_url: server.url(),
            ..PersistedCloudRelayProfile::default()
        });
        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )
            .expect("provider account registry");
        let request = LocalDaemonRequest::GetManagedEnvironmentReimagePreflight(
            GetManagedEnvironmentReimagePreflightRequest {
                environment_id: "environment-1".to_string(),
            },
        );

        let wrong_owner = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            ),
            "other-user",
            request.clone(),
        )
        .await
        .expect_err("another Cloud user must not read reimage preflight");
        assert!(wrong_owner
            .to_string()
            .contains("belongs to another Cloud user"));

        config
            .cloud_relay
            .as_mut()
            .expect("Cloud profile")
            .cloud_session_token = None;
        let missing_session = execute_managed_environment_control_request(
            config,
            provider_account_profiles,
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            ),
            "owner-1",
            request,
        )
        .await
        .expect_err("machine identity must not authorize preflight");
        assert!(missing_session
            .to_string()
            .contains("Cloud session is unavailable"));
        assert!(server.requests().is_empty());
    }

    #[test]
    fn managed_environment_create_preflights_selected_provider_exports() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-environment-preflight-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        fs::create_dir_all(&root).expect("test root");
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("provider-accounts.json"),
        )
        .expect("provider account registry");
        let profile = registry
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "Managed default",
            )
            .expect("managed provider account");
        let environment = registry
            .resolve_environment(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                &profile.profile_id,
            )
            .expect("provider account environment");
        fs::write(
            Path::new(&environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"source"}"#,
        )
        .expect("provider credential");
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });
        let cloud = config.cloud_relay.as_ref().expect("Cloud profile");

        preflight_provider_account_exports(
            &config,
            cloud,
            &registry,
            &ManagedEnvironmentProviderAccounts::Selected {
                accounts: vec![ManagedEnvironmentProviderAccountSelection {
                    provider: "codex".to_string(),
                    account_profile: profile.profile_id,
                }],
            },
        )
        .expect("transferable account");
        let error = preflight_provider_account_exports(
            &config,
            cloud,
            &registry,
            &ManagedEnvironmentProviderAccounts::Selected {
                accounts: vec![ManagedEnvironmentProviderAccountSelection {
                    provider: "claude".to_string(),
                    account_profile: "missing".to_string(),
                }],
            },
        )
        .expect_err("missing account must fail before create");
        assert!(error.to_string().contains(
            "selected claude provider account `missing` has no transferable credentials"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn managed_environment_cloud_summary_tolerates_pre_kernel_binding_responses() {
        let mut legacy = environment_json();
        legacy
            .as_object_mut()
            .expect("environment object")
            .remove("runtimeKernelId");
        let decoded: cloud_contract::EnvironmentSummary =
            serde_json::from_value(legacy).expect("legacy Cloud summary");
        let summary: crate::local::ManagedEnvironmentSummary = decoded.into();

        assert_eq!(
            summary.runtime_machine_id.as_deref(),
            Some("managed-machine-1")
        );
        assert_eq!(summary.runtime_kernel_id, None);
    }

    #[test]
    fn managed_environment_details_project_activity_and_exact_operation_history() {
        let mut environment = environment_json();
        environment["runningAgentCount"] = serde_json::json!(0);
        environment["lastActivityReportedAt"] =
            serde_json::json!("2026-09-26T05:00:02.000Z");
        environment["lastActivityChangedAt"] =
            serde_json::json!("2026-09-26T04:59:00.000Z");
        environment["autoStopWarningAt"] = serde_json::Value::Null;
        environment["autoStopDeadlineAt"] = serde_json::json!("2026-09-26T05:14:00.000Z");
        let mut operation = operation_json();
        operation["kind"] = serde_json::json!("stop");
        operation["status"] = serde_json::json!("succeeded");
        operation["desiredRevision"] = serde_json::json!(7);
        operation["completedAt"] = serde_json::json!("2026-09-26T05:14:03.000Z");
        let details: cloud_contract::EnvironmentDetailsResponse = serde_json::from_value(
            serde_json::json!({ "environment": environment, "operations": [operation] }),
        )
        .expect("owner-authorized Cloud details response");

        let summary: crate::local::ManagedEnvironmentSummary = details.environment.into();
        assert_eq!(summary.running_agent_count, Some(0));
        assert_eq!(
            summary.last_activity_reported_at.as_deref(),
            Some("2026-09-26T05:00:02.000Z")
        );
        assert_eq!(
            summary.last_activity_changed_at.as_deref(),
            Some("2026-09-26T04:59:00.000Z")
        );
        assert_eq!(summary.auto_stop_warning_at, None);
        assert_eq!(
            summary.auto_stop_deadline_at.as_deref(),
            Some("2026-09-26T05:14:00.000Z")
        );
        let operations: Vec<crate::local::ManagedEnvironmentOperationSummary> =
            details.operations.into_iter().map(Into::into).collect();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation_id, "operation-1");
        assert_eq!(operations[0].kind, crate::local::ManagedEnvironmentOperationKind::Stop);
        assert_eq!(operations[0].status, crate::local::ManagedEnvironmentOperationStatus::Succeeded);
        assert_eq!(operations[0].desired_revision, 7);
        assert_eq!(
            operations[0].completed_at.as_deref(),
            Some("2026-09-26T05:14:03.000Z")
        );
    }

    #[test]
    fn managed_environment_details_legacy_null_and_invalid_activity_are_distinguished() {
        let mut legacy_environment = environment_json();
        for field in [
            "runningAgentCount",
            "lastActivityReportedAt",
            "lastActivityChangedAt",
            "autoStopWarningAt",
            "autoStopDeadlineAt",
        ] {
            legacy_environment
                .as_object_mut()
                .expect("environment object")
                .remove(field);
        }
        let legacy: cloud_contract::EnvironmentDetailsResponse = serde_json::from_value(
            serde_json::json!({ "environment": legacy_environment }),
        )
        .expect("legacy Cloud details response");
        let summary: crate::local::ManagedEnvironmentSummary = legacy.environment.into();
        assert_eq!(summary.running_agent_count, None);
        assert_eq!(summary.last_activity_changed_at, None);
        assert!(legacy.operations.is_empty());

        let mut nullable = environment_json();
        for field in [
            "runningAgentCount",
            "lastActivityReportedAt",
            "lastActivityChangedAt",
            "autoStopWarningAt",
            "autoStopDeadlineAt",
        ] {
            nullable[field] = serde_json::Value::Null;
        }
        let decoded: cloud_contract::EnvironmentSummary =
            serde_json::from_value(nullable).expect("explicit null is unknown");
        let summary: crate::local::ManagedEnvironmentSummary = decoded.into();
        assert_eq!(summary.running_agent_count, None);
        assert_eq!(summary.last_activity_reported_at, None);
        assert_eq!(summary.last_activity_changed_at, None);
        assert_eq!(summary.auto_stop_warning_at, None);
        assert_eq!(summary.auto_stop_deadline_at, None);

        let mut invalid_count = environment_json();
        invalid_count["runningAgentCount"] = serde_json::json!(2);
        assert!(serde_json::from_value::<cloud_contract::EnvironmentSummary>(invalid_count).is_err());
        let mut invalid_timestamp = environment_json();
        invalid_timestamp["lastActivityChangedAt"] =
            serde_json::json!("2026-09-26T04:59:00Z");
        assert!(
            serde_json::from_value::<cloud_contract::EnvironmentSummary>(invalid_timestamp).is_err()
        );
    }

    #[test]
    fn git_credential_enrollment_ticket_matches_the_local_selection_and_source() {
        let thumbprint = "a".repeat(64);
        let ticket = crate::managed_context::outbound_service::ManagedContextTransferTicket {
            environment_id: "environment-1".to_string(),
            context_plan:
                crate::managed_bootstrap::ManagedKernelContextPlan::git_credential_enrollment_for_tests(
                    "managed_ctx_enroll",
                    "realm-1",
                    "source-kernel",
                    &thumbprint,
                ),
            target: crate::managed_context::outbound_service::ManagedContextTransferTarget {
                relay_realm_id: "realm-1".to_string(),
                machine_id: "target-machine".to_string(),
                kernel_id: "target-kernel".to_string(),
                relay_public_key: "target-public-key".to_string(),
                key_thumbprint: thumbprint,
            },
        };
        let request = |source_target_id: &str, credential_id: &str| {
            PrepareManagedEnvironmentGitCredentialEnrollmentRequest {
                environment_id: "environment-1".to_string(),
                source_target_id: source_target_id.to_string(),
                git_credentials: ManagedEnvironmentGitCredentials::Selected {
                    credential_ids: vec![credential_id.to_string()],
                },
            }
        };

        validate_git_credential_enrollment_ticket(
            &ticket,
            &request("source-target-test", "github"),
        )
        .expect("exact ticket binding");
        assert!(validate_git_credential_enrollment_ticket(
            &ticket,
            &request("source-target-test", "work")
        )
        .is_err());
        assert!(validate_git_credential_enrollment_ticket(
            &ticket,
            &request("other-source", "github")
        )
        .is_err());
    }

    #[test]
    fn managed_environment_reimage_request_requires_cloud_safe_identity_and_generation() {
        let valid = reimage_request(empty_context_plan());
        validate_reimage_request(&valid).expect("valid reimage request");

        let mut invalid = valid.clone();
        invalid.expected_generation = 0;
        assert!(validate_reimage_request(&invalid).is_err());

        let mut invalid = valid.clone();
        invalid.environment_id = "Environment-1".to_string();
        assert!(validate_reimage_request(&invalid).is_err());

        let mut invalid = valid.clone();
        invalid.idempotency_key = "reimage key".to_string();
        assert!(validate_reimage_request(&invalid).is_err());

        let mut invalid = valid.clone();
        invalid.expected_provider_server_id = "server-1".to_string();
        assert!(validate_reimage_request(&invalid).is_err());

        let mut invalid = valid.clone();
        invalid.expected_provider_profile_digest = format!("sha256:{}", "B".repeat(64));
        assert!(validate_reimage_request(&invalid).is_err());

        let mut invalid = valid;
        invalid.expected_runtime_source_commit = "not-a-commit".to_string();
        assert!(validate_reimage_request(&invalid).is_err());
    }

    #[tokio::test]
    async fn managed_machine_credential_never_authorizes_reimage_control() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account-1".to_string(),
            user_id: "owner-1".to_string(),
            machine_credential: Some(format!("mcred_{}", "a".repeat(40))),
            cloud_session_token: None,
            ..PersistedCloudRelayProfile::default()
        });
        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )
            .expect("provider account registry");
        let error = execute_managed_environment_control_request(
            config,
            provider_account_profiles,
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            ),
            "owner-1",
            LocalDaemonRequest::RequestManagedEnvironmentReimage(reimage_request(
                empty_context_plan(),
            )),
        )
        .await
        .expect_err("machine credential must not authorize reimage");
        assert!(error.to_string().contains("Cloud session is unavailable"));
    }

    #[tokio::test]
    async fn managed_environment_reimage_forwards_selected_fresh_context() {
        let server =
            ManagedEnvironmentCloudFixture::start(serde_json::Value::Null, serde_json::Value::Null);
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account-1".to_string(),
            user_id: "cloud-user-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            api_url: server.url(),
            ..PersistedCloudRelayProfile::default()
        });
        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )
            .expect("provider account registry");

        execute_managed_environment_control_request(
            config,
            provider_account_profiles,
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            ),
            "cloud-user-1",
            LocalDaemonRequest::RequestManagedEnvironmentReimage(reimage_request(
                selected_context_plan(ManagedEnvironmentProviderAccounts::None),
            )),
        )
        .await
        .expect("selected-context reimage request");

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        let body = requests[0]
            .split_once("\r\n\r\n")
            .map(|(_, body)| serde_json::from_str::<serde_json::Value>(body).expect("JSON body"))
            .expect("request body");
        assert_eq!(
            body.pointer("/contextPlan"),
            Some(&serde_json::json!({
                "sourceTargetId": "source-target-test",
                "kernelContext": "source_kernel",
                "developmentSetup": {
                    "kind": "source_project",
                    "projectId": "project-1",
                    "repositories": [{
                        "role": "primary",
                        "workspaceId": "workspace-primary",
                        "worktreeId": null,
                    }],
                },
                "providerAccounts": { "kind": "none" },
                "gitCredentials": { "kind": "none" },
            }))
        );
    }

    #[tokio::test]
    async fn managed_environment_reimage_rejects_unexportable_accounts_before_cloud() {
        let server =
            ManagedEnvironmentCloudFixture::start(serde_json::Value::Null, serde_json::Value::Null);
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account-1".to_string(),
            user_id: "cloud-user-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            api_url: server.url(),
            ..PersistedCloudRelayProfile::default()
        });
        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )
            .expect("provider account registry");
        let context_plan = selected_context_plan(ManagedEnvironmentProviderAccounts::Selected {
            accounts: vec![ManagedEnvironmentProviderAccountSelection {
                provider: "claude".to_string(),
                account_profile: "missing".to_string(),
            }],
        });

        let error = execute_managed_environment_control_request(
            config,
            provider_account_profiles,
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            ),
            "cloud-user-1",
            LocalDaemonRequest::RequestManagedEnvironmentReimage(reimage_request(context_plan)),
        )
        .await
        .expect_err("unexportable provider account must fail before reimage");

        assert!(error.to_string().contains(
            "selected claude provider account `missing` has no transferable credentials"
        ));
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn managed_environment_reimage_preflight_rejects_another_environment() {
        let server = ManagedEnvironmentCloudFixture::start_with_preflight_environment(
            serde_json::Value::Null,
            serde_json::Value::Null,
            "environment-other",
        );
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account-1".to_string(),
            user_id: "cloud-user-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            api_url: server.url(),
            ..PersistedCloudRelayProfile::default()
        });
        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )
            .expect("provider account registry");

        let error = execute_managed_environment_control_request(
            config,
            provider_account_profiles,
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            ),
            "cloud-user-1",
            LocalDaemonRequest::GetManagedEnvironmentReimagePreflight(
                GetManagedEnvironmentReimagePreflightRequest {
                    environment_id: "environment-1".to_string(),
                },
            ),
        )
        .await
        .expect_err("preflight for another environment must fail closed");

        assert!(error
            .to_string()
            .contains("preflight for another managed environment"));
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn managed_environment_reimage_receipt_is_read_only_owner_bound_and_environment_bound() {
        let server = ManagedEnvironmentCloudFixture::start_with_preflight_environment(
            serde_json::Value::Null,
            serde_json::Value::Null,
            "environment / one",
        );
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account / one".to_string(),
            user_id: "owner-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            api_url: server.url(),
            ..PersistedCloudRelayProfile::default()
        });
        let profiles = crate::account_profile::ProviderAccountProfileRegistry::open(
            config.account_profile_registry_path(),
        )
        .expect("provider account registry");
        let store =
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            );
        let request =
            LocalDaemonRequest::GetManagedEnvironmentReimageReceipt(GetManagedEnvironmentRequest {
                environment_id: "environment / one".to_string(),
            });
        let denied = execute_managed_environment_control_request(
            config.clone(),
            profiles.clone(),
            store.clone(),
            "other-user",
            request.clone(),
        )
        .await
        .expect_err("receipt requires the Cloud owner");
        assert!(denied.to_string().contains("belongs to another Cloud user"));
        let mut missing_session = config.clone();
        missing_session
            .cloud_relay
            .as_mut()
            .unwrap()
            .cloud_session_token = None;
        let denied = execute_managed_environment_control_request(
            missing_session,
            profiles.clone(),
            store.clone(),
            "owner-1",
            request.clone(),
        )
        .await
        .expect_err("receipt requires the Cloud session");
        assert!(denied.to_string().contains("Cloud session is unavailable"));
        assert!(server.requests().is_empty());
        let response = execute_managed_environment_control_request(
            config.clone(),
            profiles.clone(),
            store.clone(),
            "owner-1",
            request,
        )
        .await
        .expect("read receipt");
        let LocalDaemonResponse::ManagedEnvironmentReimageReceipt { receipt } = response else {
            panic!("unexpected receipt response");
        };
        assert_eq!(receipt.environment_id, "environment / one");
        assert_eq!(receipt.receipt_id, "receipt-reimage-1");
        assert!(!receipt.fresh_equivalent, "pending is not fresh-equivalent");
        let denied = execute_managed_environment_control_request(
            config,
            profiles,
            store,
            "owner-1",
            LocalDaemonRequest::GetManagedEnvironmentReimageReceipt(GetManagedEnvironmentRequest {
                environment_id: "environment-other".to_string(),
            }),
        )
        .await
        .expect_err("wrong environment receipt must fail");
        assert!(denied
            .to_string()
            .contains("receipt for another managed environment"));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /managed-environments/environment%20%2F%20one/reimage/receipt?accountId=account%20%2F%20one HTTP/1.1"));
        assert!(requests.iter().all(|request| request.starts_with("GET ")));
    }

    #[tokio::test]
    async fn managed_environment_control_uses_authenticated_cloud_profile_for_all_operations() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            account_id: "account / one".to_string(),
            user_id: "cloud-user-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            realm_id: "realm-1".to_string(),
            machine_id: Some("source-machine-test".to_string()),
            ..PersistedCloudRelayProfile::default()
        });
        let source_thumbprint =
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
        let context_plan =
            crate::managed_bootstrap::ManagedKernelContextPlan::source_project_for_tests(
                "context-1",
                "realm-1",
                &config.daemon_id,
                &source_thumbprint,
                "project-1",
            );
        let target_private_key = crate::transport::relay_crypto::generate_private_key_base64();
        let target_public_key =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&target_private_key)
                .expect("target public key");
        let target_thumbprint =
            crate::runtime::terminal_pairings::public_key_thumbprint(&target_public_key);
        let enrollment_plan =
            crate::managed_bootstrap::ManagedKernelContextPlan::git_credential_enrollment_for_tests(
                "managed_ctx_enroll",
                "realm-1",
                &config.daemon_id,
                &source_thumbprint,
            );
        let server = ManagedEnvironmentCloudFixture::start(
            serde_json::json!({
            "environmentId": "environment / one",
            "contextPlan": context_plan,
            "target": {
                "relayRealmId": "realm-1",
                "machineId": "target-machine",
                "kernelId": "target-kernel",
                "relayPublicKey": target_public_key.clone(),
                "keyThumbprint": target_thumbprint.clone(),
                "futureTargetField": true
            },
            "futureTicketField": true
            }),
            serde_json::json!({
                "environmentId": "environment / one",
                "contextPlan": enrollment_plan,
                "target": {
                    "relayRealmId": "realm-1",
                    "machineId": "target-machine",
                    "kernelId": "target-kernel",
                    "relayPublicKey": target_public_key,
                    "keyThumbprint": target_thumbprint,
                    "futureTargetField": true
                },
                "futureTicketField": true
            }),
        );
        config.cloud_relay.as_mut().expect("Cloud profile").api_url = server.url();
        let provider_account_profiles =
            crate::account_profile::ProviderAccountProfileRegistry::open(
                config.account_profile_registry_path(),
            )
            .expect("provider account registry");
        let outbound_store =
            crate::managed_context::outbound_service::ManagedContextOutboundOperationStore::default(
            );

        let catalog = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::ListManagedEnvironmentCatalog(ListManagedEnvironmentCatalogRequest),
        )
        .await
        .expect("catalog request");
        let LocalDaemonResponse::ManagedEnvironmentCatalog { catalog } = catalog else {
            panic!("unexpected catalog response");
        };
        assert_eq!(catalog.compute_classes[0].compute_class, "agent-small");
        assert_eq!(catalog.context_sources[0].source_target_id, "source-1");
        assert_eq!(catalog.environments[0].environment_id, "environment-1");
        assert_eq!(
            catalog.environments[0].runtime_kernel_id.as_deref(),
            Some("managed-kernel-1")
        );

        let create = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::CreateManagedEnvironment(CreateManagedEnvironmentRequest {
                client_request_id: "create-1".to_string(),
                name: "Managed agent".to_string(),
                region: "hel1".to_string(),
                compute_class: "agent-small".to_string(),
                managed_repository_root: Some("/srv/chariox/repos".to_string()),
                auto_stop_policy: ManagedEnvironmentAutoStopPolicy {
                    minimum_runtime_seconds: 0,
                    idle_delay_seconds: Some(900),
                },
                context_plan: ManagedEnvironmentContextPlanInput {
                    source_target_id: None,
                    kernel_context: ManagedEnvironmentKernelContextSelection::Empty,
                    development_setup: ManagedEnvironmentDevelopmentSetup::Empty,
                    provider_accounts: ManagedEnvironmentProviderAccounts::None,
                    git_credentials: ManagedEnvironmentGitCredentials::None,
                },
            }),
        )
        .await
        .expect("create request");
        let LocalDaemonResponse::ManagedEnvironmentCreated { result } = create else {
            panic!("unexpected create response");
        };
        assert_eq!(result.environment.managed_repository_root, "/home/chariox");
        let create_request = server
            .requests()
            .into_iter()
            .find(|request| request.starts_with("POST /managed-environments"))
            .expect("Cloud create request");
        let create_body = create_request
            .split_once("\r\n\r\n")
            .map(|(_, body)| serde_json::from_str::<serde_json::Value>(body).expect("JSON body"))
            .expect("request body");
        assert_eq!(
            create_body.get("managedRepositoryRoot"),
            Some(&serde_json::json!("/srv/chariox/repos"))
        );

        let get = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::GetManagedEnvironment(GetManagedEnvironmentRequest {
                environment_id: "environment / one".to_string(),
            }),
        )
        .await
        .expect("get request");
        let LocalDaemonResponse::ManagedEnvironment {
            environment,
            operations,
        } = get
        else {
            panic!("unexpected managed environment response");
        };
        assert_eq!(environment.running_agent_count, Some(0));
        assert_eq!(
            environment.last_activity_changed_at.as_deref(),
            Some("2026-08-21T00:00:00.000Z")
        );
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation_id, "operation-1");

        let preflight = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::GetManagedEnvironmentReimagePreflight(
                GetManagedEnvironmentReimagePreflightRequest {
                    environment_id: "environment / one".to_string(),
                },
            ),
        )
        .await
        .expect("reimage preflight request");
        let LocalDaemonResponse::ManagedEnvironmentReimagePreflight { preflight } = preflight
        else {
            panic!("unexpected reimage preflight response");
        };
        assert_eq!(preflight.environment_id, "environment / one");
        assert_eq!(preflight.retained.provider_server_id, "123456789");
        assert_eq!(preflight.retained.generation, 1);
        assert_eq!(preflight.desired_release.provider_image_id, "222222222");
        assert_eq!(
            preflight.desired_release.provider_id,
            crate::local::ManagedEnvironmentReimageProviderId::Hetzner
        );

        let prepared = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(
                PrepareManagedEnvironmentContextTransferRequest {
                    environment_id: "environment / one".to_string(),
                },
            ),
        )
        .await
        .expect("prepare context transfer");
        let LocalDaemonResponse::ManagedEnvironmentContextTransferPrepared { ticket } = prepared
        else {
            panic!("unexpected context transfer response");
        };
        assert_eq!(ticket.environment_id, "environment / one");
        assert_eq!(ticket.target.kernel_id, "target-kernel");

        let enrollment = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::PrepareManagedEnvironmentGitCredentialEnrollment(
                PrepareManagedEnvironmentGitCredentialEnrollmentRequest {
                    environment_id: "environment / one".to_string(),
                    source_target_id: "source-target-test".to_string(),
                    git_credentials: ManagedEnvironmentGitCredentials::Selected {
                        credential_ids: vec!["github".to_string()],
                    },
                },
            ),
        )
        .await
        .expect("prepare Git credential enrollment");
        assert!(matches!(
            enrollment,
            LocalDaemonResponse::ManagedEnvironmentContextTransferPrepared { .. }
        ));

        let reimage = execute_managed_environment_control_request(
            config.clone(),
            provider_account_profiles.clone(),
            outbound_store.clone(),
            "cloud-user-1",
            LocalDaemonRequest::RequestManagedEnvironmentReimage(reimage_request(
                empty_context_plan(),
            )),
        )
        .await
        .expect("reimage request");
        let LocalDaemonResponse::ManagedEnvironmentReimageRequested { result } = reimage else {
            panic!("unexpected reimage response");
        };
        assert_eq!(result.receipt.previous_generation, 1);
        assert_eq!(result.receipt.generation, 2);
        assert_eq!(result.receipt.provider_server_id, "123456789");

        let lifecycle = execute_managed_environment_control_request(
            config,
            provider_account_profiles,
            outbound_store,
            "cloud-user-1",
            LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
                RequestManagedEnvironmentLifecycleRequest {
                    environment_id: "environment / one".to_string(),
                    action: ManagedEnvironmentLifecycleAction::Start,
                    idempotency_key: "start-1".to_string(),
                },
            ),
        )
        .await
        .expect("lifecycle request");
        assert!(matches!(
            lifecycle,
            LocalDaemonResponse::ManagedEnvironmentLifecycleRequested { .. }
        ));

        let requests = server.requests();
        assert_eq!(requests.len(), 9);
        assert!(requests.iter().all(|request| request
            .to_ascii_lowercase()
            .contains("authorization: bearer session-secret")));
        assert!(requests.iter().any(|request| request.starts_with(
            "GET /managed-environments/options?accountId=account%20%2F%20one HTTP/1.1"
        )));
        assert!(requests.iter().any(|request| request
            .starts_with("GET /managed-environments?accountId=account%20%2F%20one HTTP/1.1")));
        assert!(requests.iter().any(|request| request.starts_with(
            "GET /managed-environments/environment%20%2F%20one?accountId=account%20%2F%20one HTTP/1.1"
        )));
        let preflight_request = requests
            .iter()
            .find(|request| request.starts_with(
                "GET /managed-environments/environment%20%2F%20one/reimage/preflight?accountId=account%20%2F%20one HTTP/1.1"
            ))
            .expect("reimage preflight HTTP request");
        assert_eq!(
            preflight_request
                .split_once("\r\n\r\n")
                .map(|(_, body)| body),
            Some("")
        );
        assert!(requests.iter().any(|request| request.starts_with(
            "GET /managed-environments/environment%20%2F%20one/context-transfer?accountId=account%20%2F%20one HTTP/1.1"
        )));
        let enrollment_request = requests
            .iter()
            .find(|request| request.contains("/git-credential-enrollment HTTP/1.1"))
            .expect("Git credential enrollment HTTP request");
        assert!(enrollment_request.contains(r#""sourceTargetId":"source-target-test""#));
        assert!(enrollment_request.contains(r#""credentialIds":["github"]"#));
        let create_request = requests
            .iter()
            .find(|request| request.starts_with("POST /managed-environments HTTP/1.1"))
            .expect("create HTTP request");
        assert!(create_request.contains(r#""accountId":"account / one""#));
        assert!(create_request.contains(r#""clientRequestId":"create-1""#));
        let lifecycle_request = requests
            .iter()
            .find(|request| request.contains("/lifecycle HTTP/1.1"))
            .expect("lifecycle HTTP request");
        assert!(lifecycle_request.contains(r#""action":"start""#));
        assert!(lifecycle_request.contains(r#""idempotencyKey":"start-1""#));
        let reimage_request = requests
            .iter()
            .find(|request| {
                request.contains("/managed-environments/environment-1/reimage HTTP/1.1")
            })
            .expect("reimage HTTP request");
        let reimage_body = reimage_request
            .split_once("\r\n\r\n")
            .map(|(_, body)| {
                serde_json::from_str::<serde_json::Value>(body).expect("reimage JSON body")
            })
            .expect("reimage request body");
        assert_eq!(
            reimage_body.as_object().expect("reimage JSON object").len(),
            11
        );
        assert!(reimage_request.contains(r#""accountId":"account / one""#));
        assert!(reimage_request.contains(r#""expectedGeneration":1"#));
        assert!(reimage_request.contains(r#""expectedProviderServerId":"123456789""#));
        assert!(reimage_request.contains(r#""expectedProviderImageId":"987654321""#));
        assert!(reimage_request.contains(r#""expectedProviderProfileId":"hetzner-path1""#));
        assert!(reimage_request.contains(&format!(
            r#""expectedProviderProfileDigest":"sha256:{}""#,
            "b".repeat(64)
        )));
        assert!(reimage_request.contains(&format!(
            r#""expectedRuntimeReleaseDigest":"sha256:{}""#,
            "a".repeat(64)
        )));
        assert!(reimage_request.contains(&format!(
            r#""expectedRuntimeSourceCommit":"{}""#,
            "c".repeat(40)
        )));
        assert!(reimage_request.contains(&format!(
            r#""expectedRuntimeSourceTree":"{}""#,
            "d".repeat(40)
        )));
        assert_eq!(
            reimage_body.pointer("/contextPlan"),
            Some(&serde_json::json!({
                "sourceTargetId": null,
                "kernelContext": "empty",
                "developmentSetup": { "kind": "empty" },
                "providerAccounts": { "kind": "none" },
                "gitCredentials": { "kind": "none" },
            }))
        );
        assert!(reimage_request.contains(r#""idempotencyKey":"reimage-1""#));
        assert!(!requests.join("\n").contains("session-secret\""));
    }

    struct ManagedEnvironmentCloudFixture {
        address: std::net::SocketAddr,
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<String>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ManagedEnvironmentCloudFixture {
        fn start(
            context_transfer_ticket: serde_json::Value,
            git_enrollment_ticket: serde_json::Value,
        ) -> Self {
            Self::start_with_preflight_environment(
                context_transfer_ticket,
                git_enrollment_ticket,
                "environment / one",
            )
        }

        fn start_with_preflight_environment(
            context_transfer_ticket: serde_json::Value,
            git_enrollment_ticket: serde_json::Value,
            preflight_environment_id: &str,
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind Cloud fixture");
            listener.set_nonblocking(true).expect("nonblocking fixture");
            let address = listener.local_addr().expect("fixture address");
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let thread_stop = Arc::clone(&stop);
            let thread_requests = Arc::clone(&requests);
            let preflight_environment_id = preflight_environment_id.to_string();
            let thread = std::thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let request = read_http_request(&mut stream);
                            if request.is_empty() {
                                continue;
                            }
                            let response = fixture_response(
                                &request,
                                &context_transfer_ticket,
                                &git_enrollment_ticket,
                                &preflight_environment_id,
                            );
                            thread_requests.lock().expect("requests lock").push(request);
                            write_http_response(&mut stream, &response);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("Cloud fixture failed: {error}"),
                    }
                }
            });
            Self {
                address,
                stop,
                requests,
                thread: Some(thread),
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.address)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    impl Drop for ManagedEnvironmentCloudFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(self.address);
            if let Some(thread) = self.thread.take() {
                thread.join().expect("Cloud fixture should stop");
            }
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("fixture timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let read = match stream.read(&mut buffer) {
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "timed out reading fixture request"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("read fixture request: {error}"),
            };
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if http_request_complete(&request) {
                break;
            }
        }
        String::from_utf8(request).expect("fixture request UTF-8")
    }

    fn http_request_complete(request: &[u8]) -> bool {
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        request.len() >= header_end + 4 + content_length
    }

    fn fixture_response(
        request: &str,
        context_transfer_ticket: &serde_json::Value,
        git_enrollment_ticket: &serde_json::Value,
        preflight_environment_id: &str,
    ) -> serde_json::Value {
        if request.starts_with("GET /managed-environments/options?") {
            return serde_json::json!({
                "computeClasses": [{
                    "computeClass": "agent-small",
                    "regions": ["hel1"],
                    "futureComputeField": true
                }],
                "contextSources": [{
                    "sourceTargetId": "source-1",
                    "machineId": "source-machine",
                    "kernelId": "source-kernel",
                    "label": "This machine",
                    "futureSourceField": true
                }],
                "futureOptionsField": true
            });
        }
        if request.starts_with("GET /managed-environments?") {
            return serde_json::json!({ "environments": [environment_json()] });
        }
        if request.contains("/context-transfer?") {
            return context_transfer_ticket.clone();
        }
        if request.contains("/git-credential-enrollment ") {
            return git_enrollment_ticket.clone();
        }
        if request.contains("/reimage/preflight?") {
            return reimage_preflight_json(preflight_environment_id);
        }
        if request.contains("/reimage/receipt?") {
            let mut receipt = reimage_receipt_json();
            receipt["environmentId"] = serde_json::json!(preflight_environment_id);
            return receipt;
        }
        if request.contains("/reimage HTTP/1.1") {
            return serde_json::json!({
                "environment": environment_json(),
                "operation": reimage_operation_json(),
                "receipt": reimage_receipt_json(),
                "futureReimageResultField": true
            });
        }
        if request.starts_with("GET /managed-environments/") {
            return serde_json::json!({
                "environment": environment_json(),
                "operations": [operation_json()],
                "futureDetailsField": true
            });
        }
        if request.starts_with("POST /managed-environments") {
            return serde_json::json!({
                "environment": environment_json(),
                "operation": operation_json(),
                "futureResultField": true
            });
        }
        panic!("unexpected Cloud fixture request: {request}");
    }

    fn environment_json() -> serde_json::Value {
        serde_json::json!({
            "environmentId": "environment-1",
            "accountId": "account / one",
            "createdByUserId": "cloud-user-1",
            "name": "Managed agent",
            "region": "hel1",
            "computeClass": "agent-small",
            "managedRepositoryRoot": "/home/chariox",
            "desiredState": "running",
            "observedState": "ready",
            "desiredRevision": 1,
            "observedRevision": 1,
            "runtimeMachineId": "managed-machine-1",
            "runtimeKernelId": "managed-kernel-1",
            "runtimeReleaseDigest": "sha256:release",
            "contextPlan": {
                "schemaVersion": 1,
                "contextId": "context-1",
                "planDigest": "sha256:plan",
                "source": null,
                "kernelContext": "empty",
                "developmentSetup": { "kind": "empty", "futureDevelopmentField": true },
                "providerAccounts": { "kind": "none" },
                "gitCredentials": { "kind": "none" },
                "futurePlanField": true
            },
            "contextManifestDigest": "sha256:manifest",
            "autoStopPolicy": { "minimumRuntimeSeconds": 0, "idleDelaySeconds": 900 },
            "runningAgentCount": 0,
            "lastActivityReportedAt": "2026-08-21T00:00:00.000Z",
            "lastActivityChangedAt": "2026-08-21T00:00:00.000Z",
            "autoStopWarningAt": null,
            "autoStopDeadlineAt": "2026-08-21T00:15:00.000Z",
            "lastErrorCode": null,
            "lastErrorMessage": null,
            "createdAt": "2026-08-21T00:00:00.000Z",
            "updatedAt": "2026-08-21T00:00:00.000Z",
            "futureEnvironmentField": true
        })
    }

    fn reimage_preflight_json(environment_id: &str) -> serde_json::Value {
        serde_json::json!({
            "environmentId": environment_id,
            "retained": {
                "providerServerId": "123456789",
                "generation": 1,
                "desiredRevision": 7,
                "observedRevision": 7,
                "runtimeMachineId": "managed-machine-1",
                "runtimeKernelId": "managed-kernel-1",
                "runtimeRelayRealmId": "managed-realm-1",
                "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
            },
            "desiredRelease": {
                "providerId": "hetzner",
                "providerImageId": "222222222",
                "providerProfileId": "hetzner-path1-v2",
                "providerProfileDigest": format!("sha256:{}", "b".repeat(64)),
                "runtimeReleaseDigest": format!("sha256:{}", "c".repeat(64)),
                "runtimeSourceCommit": "d".repeat(40),
                "runtimeSourceTree": "e".repeat(40),
            },
        })
    }

    fn operation_json() -> serde_json::Value {
        serde_json::json!({
            "operationId": "operation-1",
            "environmentId": "environment-1",
            "requestedByUserId": "cloud-user-1",
            "kind": "create",
            "idempotencyKey": "create-1",
            "requestDigest": "sha256:request",
            "desiredRevision": 1,
            "status": "pending",
            "attempt": 0,
            "retryable": false,
            "failureCode": null,
            "failureMessage": null,
            "completedAt": null,
            "createdAt": "2026-08-21T00:00:00.000Z",
            "updatedAt": "2026-08-21T00:00:00.000Z",
            "futureOperationField": true
        })
    }

    fn reimage_operation_json() -> serde_json::Value {
        serde_json::json!({
            "operationId": "operation-reimage-1",
            "environmentId": "environment-1",
            "requestedByUserId": "cloud-user-1",
            "kind": "reimage",
            "idempotencyKey": "reimage-1",
            "requestDigest": "sha256:request-reimage",
            "desiredRevision": 2,
            "generation": 2,
            "status": "pending",
            "attempt": 0,
            "retryable": false,
            "failureCode": null,
            "failureMessage": null,
            "completedAt": null,
            "createdAt": "2026-08-21T00:00:00.000Z",
            "updatedAt": "2026-08-21T00:00:00.000Z",
            "futureOperationField": true
        })
    }

    fn reimage_receipt_json() -> serde_json::Value {
        serde_json::json!({
            "receiptId": "receipt-reimage-1",
            "environmentId": "environment-1",
            "operationId": "operation-reimage-1",
            "previousGeneration": 1,
            "generation": 2,
            "status": "pending",
            "freshEquivalent": false,
            "providerServerId": "123456789",
            "previousProviderImageId": "987654321",
            "providerImageId": "987654321",
            "providerProfileId": "hetzner-path1",
            "providerProfileDigest": format!("sha256:{}", "b".repeat(64)),
            "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
            "oldMachineId": "managed-machine-1",
            "newMachineId": null,
            "oldKernelId": "managed-kernel-1",
            "newKernelId": null,
            "oldRelayRealmId": "realm-1",
            "newRelayRealmId": "realm-2",
            "oldRelayTargetId": "target-1",
            "newRelayTargetId": null,
            "oldBootstrapGrantId": "grant-1",
            "newBootstrapGrantId": null,
            "oldCredentialIds": ["credential-1"],
            "newCredentialIds": [],
            "runtimeEvidence": {"generation": 2},
            "sourceEvidence": {
                "providerProfileId": "hetzner-path1",
                "providerProfileDigest": format!("sha256:{}", "b".repeat(64)),
                "providerImageId": "987654321",
                "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
                "runtimeSourceCommit": "c".repeat(40),
                "runtimeSourceTree": "d".repeat(40)
            },
            "residueChecks": {"oldGenerationFenced": true},
            "revocations": {"machineId": "managed-machine-1"},
            "billingObservation": {"provider": "hetzner"},
            "resourceObservation": {"providerServerId": "123456789"},
            "cleanupState": {"oldGenerationFenced": true},
            "rollbackState": {"state": "fail_closed"},
            "receiptDigest": null,
            "failureCode": null,
            "failureMessage": null,
            "requestedAt": "2026-08-21T00:00:00.000Z",
            "completedAt": null,
            "createdAt": "2026-08-21T00:00:00.000Z",
            "updatedAt": "2026-08-21T00:00:00.000Z",
            "futureReceiptField": true
        })
    }

    fn write_http_response(stream: &mut TcpStream, body: &serde_json::Value) {
        let body = body.to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        stream
            .write_all(response.as_bytes())
            .expect("write fixture response");
    }
}
