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
    let (bytes, publisher) =
        crate::durable_state::app_state::fixture_inbox_installation(&store, DEFAULT_LOCAL_USER_ID);
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
    harness
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
