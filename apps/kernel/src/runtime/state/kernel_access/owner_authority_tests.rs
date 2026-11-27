//! Row 405: an owner's agent has the owner's authority on all the owner's kernels; only
//! payments and secrets go through the person. The bucket is read by name so this table
//! also runs against kernels from before the change.
use crate::local::*;
use crate::runtime::state::reach::reach_need;

fn bucket(request: &LocalDaemonRequest) -> String {
    format!("{:?}", reach_need(request))
}

fn config(path: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::SetUserConfigValue(SetUserConfigValueRequest {
        path: path.into(),
        value: "true".into(),
    })
}

fn owner_requests() -> Vec<LocalDaemonRequest> {
    vec![
        // PA1: reads and ordinary owner actions.
        LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: false,
        }),
        LocalDaemonRequest::SkillsV2(crate::skill::v2::SkillsV2Request::Catalog {
            project_id: None,
            agent_id: None,
        }),
        serde_json::from_value(serde_json::json!({"UpdateNotificationSubscription": {
            "subscription_id": "sub", "change": {"paused": {"paused": true}}}}))
        .expect("subscription update shape"),
        LocalDaemonRequest::KernelTerminal(KernelTerminalRequest {
            directory: None,
            operation: RoomTerminalOperation::List,
        }),
        LocalDaemonRequest::HideAgent(HideAgentRequest {
            session_id: "s".into(),
            agent_id: "a".into(),
            hidden: true,
        }),
        LocalDaemonRequest::ForgetGatewayKernel(ForgetGatewayKernelRequest { name: "k".into() }),
        LocalDaemonRequest::CloudRelayStatus(CloudRelayStatusRequest),
        serde_json::from_value(serde_json::json!({"GetProviderAccountProfile": {
            "provider": "codex", "account_profile": "default"}}))
        .expect("provider profile read shape"),
        config("agents.usage_limit_resume"),
        // PA8: deletes and machine admin.
        LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
            project_id: "p".into(),
        }),
        LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "s".into(),
            workspace_id: None,
        }),
        LocalDaemonRequest::DeleteKernel(DeleteKernelRequest),
        LocalDaemonRequest::ControlPlane(ControlPlaneRequest::DeleteSavedState {
            allocation_ref: "a".into(),
            saved_state_ref: "v".into(),
            capability_revision: "c".into(),
            request_id: "r".into(),
            expected_revision: 1,
        }),
        LocalDaemonRequest::ApproveRemoteMachine(ApproveRemoteMachineRequest {
            machine_ref: "m".into(),
        }),
        // PA7: a rental is governed by the rentals toggle alone, never by the classifier.
        LocalDaemonRequest::ControlPlane(ControlPlaneRequest::AcquireJob {
            capability_revision: "v1".into(),
            request_id: "r".into(),
            job_ref: "job".into(),
            needs: JobNeeds::default(),
            timeout_s: 60,
        }),
    ]
}

#[test]
fn an_owners_agent_has_the_owners_authority() {
    let refused: Vec<_> = owner_requests()
        .iter()
        .filter(|request| bucket(request) != "Owner")
        .map(|request| format!("{request:?} -> {}", bucket(request)))
        .collect();
    assert!(
        refused.is_empty(),
        "kept from the owner's agents: {refused:#?}"
    );
    assert!(!crate::runtime::state::sudo_request_forbidden(
        &LocalDaemonRequest::AcceptAppHostAction(AcceptAppHostActionRequest {
            session_id: "s".into(),
            operation_id: "o".into(),
        })
    ));
}

// Only payments and secrets go through the person (PA4).
#[test]
fn payments_and_secrets_stay_with_the_person() {
    for request in [
        LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Reveal { id: "e".into() }),
        LocalDaemonRequest::ListCredentials(ListCredentialsRequest),
        LocalDaemonRequest::ExportDebugBundle(ExportDebugBundleRequest {
            session_id: "s".into(),
            bundle_label: None,
            limit: None,
        }),
        config("agents.offload_without_asking"),
        config("agents"),
        config("credential_vault.path"),
        config("relay.url"),
    ] {
        assert_ne!(bucket(&request), "Owner", "{request:?}");
    }
}
