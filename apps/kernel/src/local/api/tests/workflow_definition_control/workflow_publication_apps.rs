//! Protocol 366: App-bound workflow publications carry their pinned App plan.
use super::*;
use crate::local::{
    ConfigureAppAutomationRequest, GetWorkflowPublicationRequest, GrantAgentExtensionRequest,
    UninstallAppRequest,
};
use crate::session::DEFAULT_LOCAL_USER_ID;

/// A harness whose local owner has the fixture App `installed`
/// (`com.example.state`, outgoing event `changed`) with its stored release.
fn harness_with_app(root: &std::path::Path) -> LocalRouterTestHarness {
    let mut config = crate::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    let harness = LocalRouterTestHarness::with_config(config);
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

fn export(
    harness: &LocalRouterTestHarness,
    graph: &PublicationTestGraph,
    publication_id: &str,
) -> Result<(String, Vec<crate::local::WorkflowPublicationPackageFile>), crate::DaemonError> {
    match harness.dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
        ExportWorkflowPublicationPackageRequest {
            session_id: graph.session_id.clone(),
            publication_ref: publication_id.into(),
            kernel_url: None,
            agent_app: None,
            agent_app_assets_dir: None,
        },
    ))? {
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
fn an_app_granted_to_a_workflow_agent_is_pinned_and_packaged() {
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
    // The plan is pinned on the publication.
    let LocalDaemonResponse::WorkflowPublication {
        publication: pinned,
    } = harness
        .dispatch(LocalDaemonRequest::GetWorkflowPublication(
            GetWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
            },
        ))
        .unwrap()
    else {
        panic!("publication")
    };
    assert_eq!(pinned.apps(), Some(&apps));

    // An App change after preparation leaves the pinned package unchanged:
    // the deployment bind re-exports and compares this digest.
    harness
        .dispatch(LocalDaemonRequest::UninstallApp(UninstallAppRequest {
            installation_id: "installed".into(),
            expected_generation: "1".into(),
            delete_data: false,
        }))
        .expect("uninstall");
    let (after, after_files) = export(&harness, &graph, publication.id()).expect("re-export");
    assert_eq!(after, digest);
    assert_eq!(after_files, files);
    // Uninstalling revoked the grant: a new trigger has no App.
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
    let (_, files) = export(&harness, &graph, publication.id()).expect("export");
    let apps = package_json_file(&files, "apps.json");
    let app = &apps["apps"][0];
    assert_eq!(app["installation_id"], "installed");
    assert_eq!(app["grants"], serde_json::json!([]));
    assert_eq!(app["automations"][0]["automation_id"], "reminders");
    assert_eq!(app["automations"][0]["event_name"], "changed");
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

/// Protocol 367: one prompt asks the owner to deploy with the pinned Apps.
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
    let prepare = |request_id: &str, publication_ref: &str, release_id: &str| {
        harness.dispatch(LocalDaemonRequest::PrepareDeploymentApps(
            crate::local::PrepareDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                request_id: request_id.into(),
                publication_ref: publication_ref.into(),
                deployment_id: "deployment-1".into(),
                release_id: release_id.into(),
                package_digest: format!("sha256:{}", "a".repeat(64)),
            },
        ))
    };
    let consent = |request_id: &str| -> DeploymentAppsConsent {
        match prepare(request_id, publication.id(), "release-1").unwrap() {
            LocalDaemonResponse::DeploymentAppsConsent { consent } => consent,
            response => panic!("unexpected response: {response:?}"),
        }
    };
    let failed = |response: LocalDaemonResponse| match response {
        LocalDaemonResponse::AppRequestFailed { code } => code,
        response => panic!("unexpected response: {response:?}"),
    };
    // Without a pinned plan there is nothing to consent to.
    assert_eq!(
        failed(prepare("early", publication.id(), "release-1").unwrap()),
        crate::local::AppRequestErrorCode::InvalidRequest
    );
    export(&harness, &graph, publication.id()).expect("prepare the package");
    let answer = |interaction_id: &str, choice: &str| {
        harness
            .dispatch(LocalDaemonRequest::RespondToInteraction(
                crate::local::RespondToInteractionRequest {
                    session_id: graph.session_id.clone(),
                    interaction_id: interaction_id.into(),
                    choice_id: choice.into(),
                    custom_reply: None,
                },
            ))
            .expect("the owner answers");
    };
    let settled = |request_id: &str| {
        for _ in 0..200 {
            let current = consent(request_id);
            if current.status != DeploymentAppsConsentStatus::AwaitingApproval {
                return current.status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the answer was not recorded");
    };

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
    answer(&declined.interaction_id, "decline");
    assert_eq!(settled("consent-1"), DeploymentAppsConsentStatus::Declined);

    let approved = consent("consent-2");
    assert_ne!(approved.interaction_id, declined.interaction_id);
    answer(&approved.interaction_id, "approve");
    assert_eq!(settled("consent-2"), DeploymentAppsConsentStatus::Approved);

    let expired = consent("consent-3");
    harness.with_app(|app| {
        app.durable_state_store()
            .fixture_expire_deployment_consent(DEFAULT_LOCAL_USER_ID, "consent-3")
    });
    assert_eq!(
        consent("consent-3").status,
        DeploymentAppsConsentStatus::Expired
    );
    answer(&expired.interaction_id, "approve");
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(
        consent("consent-3").status,
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
    let preview = || match harness
        .dispatch(LocalDaemonRequest::PreviewDeploymentApps(
            crate::local::PreviewDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::DeploymentAppsPreview {
            publication_id,
            pinned,
            plan,
        } => {
            assert_eq!(publication_id, publication.id());
            (pinned, plan)
        }
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(preview(), (false, None), "no App yet");
    harness
        .dispatch(LocalDaemonRequest::ConfigureAppAutomation(
            ConfigureAppAutomationRequest {
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
    // Previewing pins nothing; the export does.
    let (_, files) = export(&harness, &graph, publication.id()).expect("export");
    let (pinned, pinned_plan) = preview();
    assert!(pinned);
    let pinned_plan = pinned_plan.unwrap();
    assert_eq!(pinned_plan, plan);
    let mut packaged = pinned_plan;
    packaged["apps"][0]
        .as_object_mut()
        .unwrap()
        .remove("capabilities");
    assert_eq!(package_json_file(&files, "apps.json"), packaged);
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}

mod copy;
