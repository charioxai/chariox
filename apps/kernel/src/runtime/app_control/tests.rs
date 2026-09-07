use super::*;
use crate::durable_state::apps::{AppRegistryMutation, AppRegistryOutcome};
use crate::runtime::command::KernelCaller;
use chariox_app_runtime::installation::{CapabilityApproval, ReleaseMetadata};

fn seed(store: &DurableKernelStateStore, owner: &str, id: &str) {
    let result = store
        .mutate_app_installation(
            owner,
            AppRegistryMutation::CreateAndStage {
                installation_id: id.into(),
                release: ReleaseMetadata {
                    app_id: "com.chariox.todo".into(),
                    version: "1.0.0".into(),
                    publisher_id: "publisher".into(),
                    package_digest: format!("sha256:{:064x}", 1),
                    schema_version: 1,
                    capabilities_digest: format!("sha256:{:064x}", 2),
                    catalog_digest: format!("sha256:{:064x}", 3),
                    view_digest: format!("sha256:{:064x}", 4),
                },
                now_ms: 1,
            },
        )
        .unwrap();
    let AppRegistryOutcome::Update(record) = result else {
        panic!("expected update")
    };
    store
        .mutate_app_installation(
            owner,
            AppRegistryMutation::Decide {
                token: record.token,
                decision: CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "private-decision".into(),
                        authority_ref: "private-authority".into(),
                    },
                },
                now_ms: 2,
            },
        )
        .unwrap();
}

#[test]
fn app_control_shared_router_projects_only_the_authenticated_owners_installations() {
    let harness = crate::local::test_support::LocalRouterTestHarness::new();
    harness.with_app(|app| {
        seed(&app.durable_state_store(), "alice", "todo-alice");
        seed(&app.durable_state_store(), "bob", "todo-bob");
    });
    let list = LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
        after: None,
        limit: Some(1),
    });
    let local = harness.dispatch_as_user("alice", list.clone()).unwrap();
    let mut remote = KernelCaller::for_source(&KernelCommandSource::RelayClient);
    remote.user_id = Some("alice".into());
    remote.client_id = Some("paired-client".into());
    // Round-trip the same wire shape used by local and relayed terminal requests.
    let relayed = serde_json::from_slice(&serde_json::to_vec(&list).unwrap()).unwrap();
    assert_eq!(
        harness
            .dispatch_with_caller(relayed, remote.clone())
            .unwrap(),
        local
    );
    let LocalDaemonResponse::AppInstallationsListed {
        installations,
        next_cursor,
    } = local
    else {
        panic!("wrong response")
    };
    assert_eq!(installations.len(), 1);
    assert_eq!(installations[0].installation_id, "todo-alice");
    assert_eq!(installations[0].generation, "0");
    assert_eq!(installations[0].pending_generation.as_deref(), Some("1"));
    assert_eq!(next_cursor, None);
    for request in [
        LocalDaemonRequest::GetAppInstallation(AppInstallationRequest {
            installation_id: "todo-bob".into(),
        }),
        LocalDaemonRequest::GetAppInstallationJournal(AppInstallationRequest {
            installation_id: "todo-bob".into(),
        }),
    ] {
        assert_eq!(
            harness
                .dispatch_with_caller(request, remote.clone())
                .unwrap(),
            failed(AppRequestErrorCode::NotFound)
        );
    }
    let journal = harness
        .dispatch_as_user(
            "alice",
            LocalDaemonRequest::GetAppInstallationJournal(AppInstallationRequest {
                installation_id: "todo-alice".into(),
            }),
        )
        .unwrap();
    let json = serde_json::to_string(&journal).unwrap();
    assert!(json.contains("approved"));
    assert!(!json.contains("private-decision") && !json.contains("private-authority"));
    let unknown = KernelCaller::for_source(&KernelCommandSource::RelayClient);
    assert_eq!(
        harness.dispatch_with_caller(list, unknown).unwrap(),
        failed(AppRequestErrorCode::Unauthorized)
    );
}

#[test]
fn app_control_rejects_unverified_remote_and_oversized_or_forged_identity() {
    assert_eq!(
        registry_error(AppRegistryError::Registry(InstallationError::Invalid(
            "negative stored generation"
        ))),
        AppRequestErrorCode::StorageUnavailable
    );
    let request = LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
        after: None,
        limit: None,
    });
    for source in [
        KernelCommandSource::RelayClient,
        KernelCommandSource::RelayPeer,
        KernelCommandSource::DaemonBackground,
    ] {
        let mut command =
            KernelCommand::from_local_request_with_source("id", source, None, None, &request);
        assert_eq!(owner(&command), Err(AppRequestErrorCode::Unauthorized));
        command.caller.user_id = Some("x".repeat(129));
        assert_eq!(owner(&command), Err(AppRequestErrorCode::Unauthorized));
    }
    let local = KernelCommand::from_local_request("id", None, None, &request);
    assert_eq!(owner(&local).unwrap(), DEFAULT_LOCAL_USER_ID);
    assert!(serde_json::from_str::<LocalDaemonRequest>(
        r#"{"ListAppInstallations":{"owner_id":"other"}}"#
    )
    .is_err());
}

#[tokio::test]
async fn app_control_admission_is_bounded_and_invalid_pages_do_not_read() {
    let root = std::env::temp_dir().join(format!(
        "chariox-app-control-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir(&root).unwrap();
    {
        let store = DurableKernelStateStore::open_owned(root.join("kernel.db")).unwrap();
        let service = AppControlService::new(store);
        let request = LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
            after: None,
            limit: Some(101),
        });
        let command = KernelCommand::from_local_request("id", None, None, &request);
        assert_eq!(
            service.execute(&command, &request).await,
            Some(failed(AppRequestErrorCode::InvalidRequest))
        );
        let permit = service.admission.acquire_many(8).await.unwrap();
        assert_eq!(
            service.execute(&command, &request).await,
            Some(failed(AppRequestErrorCode::Busy))
        );
        drop(permit);
        assert_eq!(
            service.execute(&command, &request).await,
            Some(failed(AppRequestErrorCode::InvalidRequest))
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
