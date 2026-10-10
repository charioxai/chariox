//! Protocol 366/377: App-bound workflow publications carry each release's App plan.
use super::*;
use crate::local::{
    ConfigureAppAutomationRequest, GetWorkflowPublicationRequest, GrantAgentExtensionRequest,
    UninstallAppRequest,
};
use crate::session::DEFAULT_LOCAL_USER_ID;

/// A harness whose local owner has the fixture App `installed`
/// (`com.example.state`, outgoing event `changed`) with its stored release.
/// Its kernel deletes App storage from fixture storage: on Linux, App storage
/// belongs to the root storage helper, which tests do not run.
fn harness_with_app(root: &std::path::Path) -> LocalRouterTestHarness {
    let mut config = crate::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    let harness = LocalRouterTestHarness::with_config(config);
    harness.runtime_state().app_control().fixture_app_storage();
    let store = harness.with_app(|app| app.durable_state_store());
    stage_release(
        &harness,
        crate::durable_state::app_state::fixture_inbox_installation(&store, DEFAULT_LOCAL_USER_ID),
    );
    harness
}

/// Stores a release in the local release store, as an install leaves it.
fn stage_release(
    harness: &LocalRouterTestHarness,
    (bytes, publisher): (Vec<u8>, chariox_app_package::TrustedPublisher),
) {
    let store = harness.with_app(|app| app.durable_state_store());
    let verified = chariox_app_package::verify(
        &bytes,
        &chariox_app_package::VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![publisher],
        ),
    )
    .unwrap();
    chariox_app_runtime::release_store::ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &verified,
            &bytes,
            chariox_app_runtime::release_store::StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chariox-publication-apps-{label}-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::canonicalize(root).unwrap()
}

fn publish(
    harness: &LocalRouterTestHarness,
    graph: &PublicationTestGraph,
    alias: &str,
    kind: &str,
) -> crate::session::WorkflowPublicationDefinition {
    let http = kind == "ingress";
    match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                expected_workflow_revision: None,
                operation_key: None,
                queue_ref: Some("default".into()),
                alias: Some(alias.into()),
                kind: Some(kind.into()),
                route: http.then(|| format!("/{alias}")),
                methods: if http { vec!["POST".into()] } else { vec![] },
                transport: http.then(|| serde_json::json!({ "kind": "human_http" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: http.then(|| "async".into()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        response => panic!("unexpected response: {response:?}"),
    }
}

fn export_request(graph: &PublicationTestGraph, publication_id: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::ExportWorkflowPublicationPackage(ExportWorkflowPublicationPackageRequest {
        session_id: graph.session_id.clone(),
        publication_ref: publication_id.into(),
        kernel_url: None,
        agent_app: None,
        agent_app_assets_dir: None,
    })
}

fn export(
    harness: &LocalRouterTestHarness,
    graph: &PublicationTestGraph,
    publication_id: &str,
) -> Result<(String, Vec<crate::local::WorkflowPublicationPackageFile>), crate::DaemonError> {
    match harness.dispatch(export_request(graph, publication_id))? {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_digest,
            package_files,
            ..
        } => Ok((package_digest, package_files)),
        response => panic!("unexpected response: {response:?}"),
    }
}

fn contract_schema() -> jsonschema::JSONSchema {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../schema/workflow-publication-deployment-contract-v1.schema.json"
    ))
    .unwrap();
    jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .unwrap()
}

#[test]
fn an_app_granted_to_a_workflow_agent_is_packaged_per_release() {
    let root = temp_root("grant");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "app-grant");
    harness
        .dispatch(LocalDaemonRequest::GrantAgentExtension(
            GrantAgentExtensionRequest {
                workspace_id: None,
                agent_ref: graph.agent_id.clone(),
                kind: crate::local::ExtensionKind::App,
                name: "installed".into(),
                environment: None,
                credential: None,
                max_safety: None,
            },
        ))
        .expect("the workflow agent is granted the App");
    let publication = publish(&harness, &graph, "todo-grant", "ingress");

    let (digest, files) = export(&harness, &graph, publication.id()).expect("export");
    let apps = package_json_file(&files, "apps.json");
    assert_eq!(apps["schema"], "chariox.publication-apps.v1");
    let app = &apps["apps"][0];
    assert_eq!(apps["apps"].as_array().map(Vec::len), Some(1));
    assert_eq!(app["installation_id"], "installed");
    assert_eq!(app["app_id"], "com.example.state");
    assert_eq!(app["publisher_key_id"], "state-key");
    assert!(app["package_digest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("sha256:")));
    assert!(app["capabilities_digest"].as_str().is_some());
    assert!(app["publisher_key_fingerprint"].as_str().is_some());
    assert_eq!(app["grants"][0]["agent_id"], graph.agent_id.as_str());
    assert_eq!(app["automations"], serde_json::json!([]));
    // Apps are not extension requirements.
    let requirements = package_json_file(&files, "requirements.json");
    assert_eq!(requirements["extensions"], serde_json::json!([]));
    // The contract carries the same plan and satisfies its schema.
    let contract = package_json_file(&files, "deployment-contract.json");
    assert_eq!(contract["capabilities"]["apps"], apps["apps"]);
    assert!(contract_schema().is_valid(&contract));
    // Protocol 377: the plan is recorded as this release's.
    let get = || match harness
        .dispatch(LocalDaemonRequest::GetWorkflowPublication(
            GetWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::WorkflowPublication { publication } => publication,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(get().apps(), Some(&apps));
    assert_eq!(get().release_app_plan(&digest), Some(&apps));

    // An owner App update is packaged by the next release; the first release
    // keeps its own plan, which its bind and a rollback re-export.
    let newer = crate::durable_state::app_state::fixture_inbox_package_version("1.1.0", 0);
    stage_release(&harness, newer.clone());
    harness.with_app(|app| {
        crate::durable_state::app_state::fixture_update_installation(
            &app.durable_state_store(),
            DEFAULT_LOCAL_USER_ID,
            "installed",
            newer,
        )
    });
    let (next, next_files) = export(&harness, &graph, publication.id()).expect("next release");
    assert_ne!(next, digest);
    let next_apps = package_json_file(&next_files, "apps.json");
    assert_eq!(next_apps["apps"][0]["version"], "1.1.0");
    assert_eq!(apps["apps"][0]["version"], "1.0.0");
    assert_eq!(get().apps(), Some(&next_apps));
    assert_eq!(get().release_app_plan(&digest), Some(&apps));
    assert_eq!(get().release_app_plan(&next), Some(&next_apps));
    // A deployment bound to either release still verifies after the owner's
    // App update: its bind and recovery re-export with the release's plan.
    let rebound = |expected: &str| {
        harness
            .runtime_state()
            .fixture_verify_bound_release(&graph.session_id, publication.id(), expected)
            .expect("re-export the bound release")
    };
    assert_eq!(rebound(&digest), Ok(()));
    assert_eq!(rebound(&next), Ok(()));
    // Each export recorded its release's inputs digest (protocol 378).
    assert!(get().release_inputs_digest(&digest).is_some());
    assert_ne!(
        get().release_inputs_digest(&digest),
        get().release_inputs_digest(&next)
    );

    // A workflow whose App was uninstalled cannot export a new release; its
    // exported releases keep their plans.
    harness
        .dispatch(LocalDaemonRequest::UninstallApp(UninstallAppRequest {
            installation_id: "installed".into(),
            expected_generation: "2".into(),
            delete_data: false,
        }))
        .expect("uninstall");
    let error = export(&harness, &graph, publication.id()).expect_err("App not installed");
    assert!(error.to_string().contains("not installed"), "{error}");
    assert_eq!(get().release_app_plan(&digest), Some(&apps));
    assert_eq!(rebound(&digest), Ok(()));
    // A new release refuses the retained missing binding until explicitly revoked.
    let missing = publish(&harness, &graph, "todo-grant-missing", "ingress");
    let error = export(&harness, &graph, missing.id()).expect_err("missing App binding");
    assert!(error.to_string().contains("is not installed"), "{error}");
    harness
        .dispatch(LocalDaemonRequest::RevokeAgentExtension(
            crate::local::RevokeAgentExtensionRequest {
                agent_ref: graph.agent_id.clone(),
                kind: crate::local::ExtensionKind::App,
                name: "installed".into(),
            },
        ))
        .expect("explicitly revoke the missing binding");
    let later = publish(&harness, &graph, "todo-grant-2", "ingress");
    let (_, later_files) = export(&harness, &graph, later.id()).expect("export");
    assert!(later_files.iter().all(|file| file.path != "apps.json"));
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_app_automation_feeding_an_event_trigger_is_pinned_and_packaged() {
    let root = temp_root("automation");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "app-event");
    let publication = publish(&harness, &graph, "todo-due", "event_based");
    // Without an App the package has no plan.
    let unrelated = publish(&harness, &graph, "no-apps", "event_based");
    let (_, files) = export(&harness, &graph, unrelated.id()).expect("export");
    assert!(files.iter().all(|file| file.path != "apps.json"));
    assert!(
        package_json_file(&files, "deployment-contract.json")["capabilities"]
            .get("apps")
            .is_none()
    );

    match harness
        .dispatch(LocalDaemonRequest::ConfigureAppAutomation(
            ConfigureAppAutomationRequest {
                delivery_mode: crate::local::NotificationDeliveryMode::Inject,
                installation_id: "installed".into(),
                automation_id: "reminders".into(),
                expected_revision: 0,
                event_name: "changed".into(),
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                queue_ref: None,
                scheduled: false,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::AppAutomation { .. } => {}
        response => panic!("unexpected response: {response:?}"),
    }
    // The automation makes the trigger App-bound: only its owner prepares it.
    let invite = match harness
        .dispatch(LocalDaemonRequest::CreateSessionInvite(
            crate::local::CreateSessionInviteRequest {
                session_id: graph.session_id.clone(),
                expires_in_ms: None,
                max_uses: Some(1),
                collaboration_level: crate::session::CollaborationLevel::Full,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionInviteCreated { invite, .. } => invite,
        response => panic!("unexpected response: {response:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::JoinSessionInvite(
            crate::local::JoinSessionInviteRequest {
                invite_token: invite.invite_token,
                user_id: "someone-else".into(),
            },
        ))
        .unwrap();
    let error = harness
        .dispatch_as_user("someone-else", export_request(&graph, publication.id()))
        .expect_err("an App-bound trigger without a plan does not export");
    assert!(
        error.to_string().contains("uses Apps but has no App plan"),
        "{error}"
    );
    let (_, files) = export(&harness, &graph, publication.id()).expect("export");
    let apps = package_json_file(&files, "apps.json");
    let app = &apps["apps"][0];
    assert_eq!(app["installation_id"], "installed");
    assert_eq!(app["grants"], serde_json::json!([]));
    assert_eq!(app["automations"][0]["automation_id"], "reminders");
    assert_eq!(app["automations"][0]["event_name"], "changed");
    assert_eq!(app["automations"][0]["delivery_mode"], "inject");
    assert_eq!(
        app["automations"][0]["endpoint_id"],
        graph.endpoint_id.as_str()
    );
    let contract = package_json_file(&files, "deployment-contract.json");
    assert_eq!(contract["capabilities"]["apps"], apps["apps"]);
    assert!(contract_schema().is_valid(&contract));
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// A publication that had Apps and uses none now: its next release records an
/// explicit empty plan (never the previous release's) and binds without a copy.
#[test]
fn a_release_after_its_last_app_is_removed_records_an_empty_plan() {
    let root = temp_root("apps-removed");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "apps-removed");
    let publication = publish(&harness, &graph, "todo-due", "event_based");
    harness
        .dispatch(LocalDaemonRequest::ConfigureAppAutomation(
            ConfigureAppAutomationRequest {
                delivery_mode: crate::local::NotificationDeliveryMode::Queue,
                installation_id: "installed".into(),
                automation_id: "reminders".into(),
                expected_revision: 0,
                event_name: "changed".into(),
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                queue_ref: None,
                scheduled: false,
            },
        ))
        .expect("automation");
    let (with_apps, files) = export(&harness, &graph, publication.id()).expect("release 1");
    assert_eq!(
        package_json_file(&files, "apps.json")["apps"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    harness
        .dispatch(LocalDaemonRequest::DisableAppAutomation(
            crate::local::DisableAppAutomationRequest {
                installation_id: "installed".into(),
                automation_id: "reminders".into(),
                expected_revision: 1,
            },
        ))
        .expect("disable");
    let (without, files) = export(&harness, &graph, publication.id()).expect("release 2");
    assert_ne!(without, with_apps);
    assert_eq!(
        package_json_file(&files, "apps.json")["apps"],
        serde_json::json!([])
    );
    let publication = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowPublication(
            GetWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::WorkflowPublication { publication } => publication,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        publication
            .release_app_plan(&without)
            .map(|plan| &plan["apps"]),
        Some(&serde_json::json!([]))
    );
    // Both releases verify for a bind: each re-exports with its own plan.
    for digest in [&with_apps, &without] {
        assert_eq!(
            harness
                .runtime_state()
                .fixture_verify_bound_release(&graph.session_id, publication.id(), digest)
                .expect("re-export"),
            Ok(())
        );
    }
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// A release exported before the publication used any App still verifies for
/// its restart and recovery after a later release records a plan.
#[test]
fn a_release_without_apps_still_binds_after_a_later_release_records_a_plan() {
    let root = temp_root("apps-added");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "apps-added");
    let publication = publish(&harness, &graph, "todo-due", "event_based");
    let (without, files) = export(&harness, &graph, publication.id()).expect("release 1");
    assert!(files.iter().all(|file| file.path != "apps.json"));
    harness
        .dispatch(LocalDaemonRequest::ConfigureAppAutomation(
            ConfigureAppAutomationRequest {
                delivery_mode: crate::local::NotificationDeliveryMode::Queue,
                installation_id: "installed".into(),
                automation_id: "reminders".into(),
                expected_revision: 0,
                event_name: "changed".into(),
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                queue_ref: None,
                scheduled: false,
            },
        ))
        .expect("automation");
    let (with_apps, _) = export(&harness, &graph, publication.id()).expect("release 2");
    assert_ne!(without, with_apps);
    // Each release re-exports to its own digest: release 1 with no plan.
    for digest in [&without, &with_apps] {
        assert_eq!(
            harness
                .runtime_state()
                .fixture_verify_bound_release(&graph.session_id, publication.id(), digest)
                .expect("re-export"),
            Ok(())
        );
    }
    // A release this kernel never exported is still refused.
    let unknown = format!("sha256:{}", "f".repeat(64));
    let publication = match harness
        .dispatch(LocalDaemonRequest::GetWorkflowPublication(
            GetWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::WorkflowPublication { publication } => publication,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(publication.release_without_apps(&without));
    assert!(!publication.release_without_apps(&with_apps));
    assert!(
        publication.release_without_apps(&unknown),
        "the digest check refuses it instead"
    );
    // Its deploy consent covers no App: approved at once, with no prompt.
    let consent = match harness
        .dispatch(LocalDaemonRequest::PrepareDeploymentApps(
            crate::local::PrepareDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                request_id: "no-apps".into(),
                publication_ref: publication.id().into(),
                deployment_id: "deployment-1".into(),
                release_id: "release-1".into(),
                package_digest: without.clone(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::DeploymentAppsConsent { consent } => consent,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(
        consent.status,
        crate::local::DeploymentAppsConsentStatus::Approved
    );
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// A failed export records nothing; the next successful one records the plan
/// it packaged and persists it.
#[test]
fn only_a_successful_export_records_and_persists_the_app_plan() {
    let root = temp_root("failed-export");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "app-failed-export");
    harness
        .dispatch(LocalDaemonRequest::GrantAgentExtension(
            GrantAgentExtensionRequest {
                workspace_id: None,
                agent_ref: graph.agent_id.clone(),
                kind: crate::local::ExtensionKind::App,
                name: "installed".into(),
                environment: None,
                credential: None,
                max_safety: None,
            },
        ))
        .unwrap();
    let publication = publish(&harness, &graph, "todo-failed-export", "ingress");
    let pinned = || match harness
        .dispatch(LocalDaemonRequest::GetWorkflowPublication(
            GetWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::WorkflowPublication { publication } => publication.apps().cloned(),
        response => panic!("unexpected response: {response:?}"),
    };
    let mut failing = export_request(&graph, publication.id());
    if let LocalDaemonRequest::ExportWorkflowPublicationPackage(request) = &mut failing {
        request.agent_app = Some(serde_json::json!({ "enabled": true }));
        request.agent_app_assets_dir = Some(root.join("missing").display().to_string());
    }
    // The plan in the durable workflow state.
    let store = harness.with_app(|app| app.durable_state_store());
    let persisted = || {
        let owner = store
            .load_subject_events(&graph.session_id, 100)
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "workflow.runtime.updated")
            .expect("the session is persisted")
            .payload["owner_id"]
            .as_str()
            .unwrap()
            .to_owned();
        store
            .load_workflow_hot_states(&owner)
            .unwrap()
            .into_iter()
            .find(|(session_id, _)| *session_id == graph.session_id)
            .and_then(|(_, state)| {
                state
                    .workflow_publications
                    .into_iter()
                    .find(|candidate| candidate.id() == publication.id())
            })
            .expect("the publication is persisted")
            .apps()
            .cloned()
    };
    harness
        .dispatch(failing)
        .expect_err("missing agent app assets fail the export");
    assert_eq!(pinned(), None);
    assert_eq!(persisted(), None);

    let (_, files) = export(&harness, &graph, publication.id()).expect("export");
    let apps = package_json_file(&files, "apps.json");
    assert_eq!(pinned(), Some(apps.clone()));
    assert_eq!(persisted(), Some(apps));
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// Protocol 367: one prompt asks the owner to deploy with the release's Apps;
/// since 377 the same App releases are not asked about again.
#[test]
fn preparing_deployment_apps_asks_once_and_records_the_answer() {
    use crate::local::{DeploymentAppsConsent, DeploymentAppsConsentStatus};
    let root = temp_root("consent");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "app-consent");
    harness
        .dispatch(LocalDaemonRequest::GrantAgentExtension(
            GrantAgentExtensionRequest {
                workspace_id: None,
                agent_ref: graph.agent_id.clone(),
                kind: crate::local::ExtensionKind::App,
                name: "installed".into(),
                environment: None,
                credential: None,
                max_safety: None,
            },
        ))
        .unwrap();
    let publication = publish(&harness, &graph, "todo-consent", "ingress");
    let digest = std::cell::RefCell::new(format!("sha256:{}", "a".repeat(64)));
    let prepare_for = |request_id: &str, deployment_id: &str, release_id: &str| {
        harness.dispatch(LocalDaemonRequest::PrepareDeploymentApps(
            crate::local::PrepareDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                request_id: request_id.into(),
                publication_ref: publication.id().into(),
                deployment_id: deployment_id.into(),
                release_id: release_id.into(),
                package_digest: digest.borrow().clone(),
            },
        ))
    };
    let prepare = |request_id: &str, _publication_ref: &str, release_id: &str| {
        prepare_for(request_id, "deployment-1", release_id)
    };
    let consent_for = |request_id: &str, deployment_id: &str| -> DeploymentAppsConsent {
        match prepare_for(request_id, deployment_id, "release-1").unwrap() {
            LocalDaemonResponse::DeploymentAppsConsent { consent } => consent,
            response => panic!("unexpected response: {response:?}"),
        }
    };
    let consent = |request_id: &str| consent_for(request_id, "deployment-1");
    let failed = |response: LocalDaemonResponse| match response {
        LocalDaemonResponse::AppRequestFailed { code } => code,
        response => panic!("unexpected response: {response:?}"),
    };
    // Without the release's App plan there is nothing to consent to.
    assert_eq!(
        failed(prepare("early", publication.id(), "release-1").unwrap()),
        crate::local::AppRequestErrorCode::InvalidRequest
    );
    let (exported, _) = export(&harness, &graph, publication.id()).expect("prepare the package");
    *digest.borrow_mut() = exported;
    let answer = |interaction_id: &str, choice: &str| {
        harness
            .dispatch(LocalDaemonRequest::RespondToInteraction(
                crate::local::RespondToInteractionRequest {
                    session_id: graph.session_id.clone(),
                    interaction_id: interaction_id.into(),
                    choice_id: choice.into(),
                    custom_reply: None,
                    passkey: None,
                    passkey_remember_minutes: None,
                },
            ))
            .expect("the owner answers");
    };
    let settled_for = |request_id: &str, deployment_id: &str| {
        for _ in 0..200 {
            let current = consent_for(request_id, deployment_id);
            if current.status != DeploymentAppsConsentStatus::AwaitingApproval {
                return current.status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the answer was not recorded");
    };
    let settled = |request_id: &str| settled_for(request_id, "deployment-1");

    let declined = consent("consent-1");
    assert_eq!(
        declined.status,
        DeploymentAppsConsentStatus::AwaitingApproval
    );
    assert!(declined.interaction_id.starts_with("app_deploy_"));
    assert_eq!(consent("consent-1"), declined, "a replay asks nothing new");
    assert_eq!(
        failed(prepare("consent-1", publication.id(), "release-2").unwrap()),
        crate::local::AppRequestErrorCode::Conflict
    );
    // One decision per deployment at a time: another release waits.
    assert_eq!(
        failed(prepare("consent-other", publication.id(), "release-2").unwrap()),
        crate::local::AppRequestErrorCode::Busy
    );
    answer(&declined.interaction_id, "decline");
    assert_eq!(settled("consent-1"), DeploymentAppsConsentStatus::Declined);

    let approved = consent("consent-2");
    assert_ne!(approved.interaction_id, declined.interaction_id);
    answer(&approved.interaction_id, "approve");
    assert_eq!(settled("consent-2"), DeploymentAppsConsentStatus::Approved);

    // Protocol 377: the same App releases for this deployment are approved
    // without asking again.
    assert_eq!(
        consent("consent-3").status,
        DeploymentAppsConsentStatus::Approved
    );

    // Another deployment asks; an unanswered prompt expires.
    let expired = consent_for("consent-4", "deployment-2");
    assert_eq!(
        expired.status,
        DeploymentAppsConsentStatus::AwaitingApproval
    );
    harness.with_app(|app| {
        app.durable_state_store()
            .fixture_expire_deployment_consent(DEFAULT_LOCAL_USER_ID, "consent-4")
    });
    assert_eq!(
        consent_for("consent-4", "deployment-2").status,
        DeploymentAppsConsentStatus::Expired
    );
    answer(&expired.interaction_id, "approve");
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        consent_for("consent-4", "deployment-2").status,
        DeploymentAppsConsentStatus::Expired
    );
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// Protocol 367: a deployment copy's installation belongs to its deployment.
#[test]
fn a_deployment_copy_is_hidden_from_the_app_list_and_marked_in_the_app_set() {
    let root = temp_root("copy");
    let harness = harness_with_app(&root);
    harness.with_app(|app| {
        app.durable_state_store().fixture_tag_app_installation(
            DEFAULT_LOCAL_USER_ID,
            "installed",
            "deployment-1",
        )
    });
    match harness
        .dispatch(LocalDaemonRequest::ListAppInstallations(
            crate::local::ListAppInstallationsRequest {
                after: None,
                limit: None,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::AppInstallationsListed { installations, .. } => {
            assert!(installations.is_empty())
        }
        response => panic!("unexpected response: {response:?}"),
    }
    match harness
        .dispatch(LocalDaemonRequest::GetAppSet(
            crate::local::GetAppSetRequest {},
        ))
        .unwrap()
    {
        LocalDaemonResponse::AppSet { installations, .. } => {
            assert_eq!(installations.len(), 1);
            assert_eq!(
                installations[0].deployment_id.as_deref(),
                Some("deployment-1")
            );
        }
        response => panic!("unexpected response: {response:?}"),
    }
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

/// Protocol 367: the web previews the Apps before the first export.
#[test]
fn a_publication_previews_its_apps_before_and_after_preparation() {
    let root = temp_root("preview");
    let harness = harness_with_app(&root);
    let graph = create_publication_test_graph(&harness, "app-preview");
    let publication = publish(&harness, &graph, "todo-preview", "event_based");
    let preview_release = |package_digest: Option<String>| match harness
        .dispatch(LocalDaemonRequest::PreviewDeploymentApps(
            crate::local::PreviewDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                package_digest,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::DeploymentAppsPreview {
            publication_id,
            pinned,
            plan,
            release_plan,
        } => {
            assert_eq!(publication_id, publication.id());
            (pinned, plan, release_plan)
        }
        response => panic!("unexpected response: {response:?}"),
    };
    let preview = || {
        let (pinned, plan, release_plan) = preview_release(None);
        assert_eq!(release_plan, None);
        (pinned, plan)
    };
    assert_eq!(preview(), (false, None), "no App yet");
    harness
        .dispatch(LocalDaemonRequest::ConfigureAppAutomation(
            ConfigureAppAutomationRequest {
                delivery_mode: crate::local::NotificationDeliveryMode::Queue,
                installation_id: "installed".into(),
                automation_id: "reminders".into(),
                expected_revision: 0,
                event_name: "changed".into(),
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                queue_ref: None,
                scheduled: false,
            },
        ))
        .unwrap();
    let (pinned, plan) = preview();
    assert!(!pinned);
    let plan = plan.expect("the App-event trigger uses the App");
    let app = &plan["apps"][0];
    assert_eq!(app["app_id"], "com.example.state");
    assert_eq!(app["automations"][0]["event_name"], "changed");
    assert!(app["capabilities"].is_object());
    // Previewing prepares nothing; the export records the release's plan.
    let (digest, files) = export(&harness, &graph, publication.id()).expect("export");
    let (pinned, current) = preview();
    assert!(pinned);
    assert_eq!(current.unwrap(), plan);
    let (_, _, release_plan) = preview_release(Some(digest));
    let release_plan = release_plan.expect("the release's plan");
    assert_eq!(release_plan, plan);
    let mut packaged = release_plan;
    packaged["apps"][0]
        .as_object_mut()
        .unwrap()
        .remove("capabilities");
    assert_eq!(package_json_file(&files, "apps.json"), packaged);
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

mod copy;
