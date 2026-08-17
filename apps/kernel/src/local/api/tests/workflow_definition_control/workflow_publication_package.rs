use super::*;

#[test]
fn publication_creation_is_revision_safe_idempotent_and_source_independent() {
    let harness = LocalRouterTestHarness::new();
    let graph = create_publication_test_graph(&harness, "immutable-source");
    let workflow = match harness
        .dispatch(LocalDaemonRequest::ResolveWorkflow(
            ResolveWorkflowRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
            },
        ))
        .expect("workflow should resolve")
    {
        LocalDaemonResponse::WorkflowResolved { workflow } => workflow,
        _ => panic!("unexpected local response"),
    };

    let mut request = CreateWorkflowPublicationRequest {
        session_id: graph.session_id.clone(),
        workflow_ref: graph.workflow_id.clone(),
        endpoint_ref: graph.endpoint_id.clone(),
        expected_workflow_revision: Some(workflow.revision()),
        operation_key: Some("publish-immutable-source".to_string()),
        queue_ref: Some("default".to_string()),
        alias: Some("immutable-source".to_string()),
        kind: Some("ingress".to_string()),
        route: Some("/immutable/*".to_string()),
        methods: vec!["POST".to_string()],
        transport: Some(serde_json::json!({ "kind": "human_http" })),
        parser: None,
        input_schema: None,
        trace_exposure: None,
        mode: Some("async".to_string()),
        sync_timeout_ms: None,
        poll_ms: None,
    };
    let mut stale_request = request.clone();
    stale_request.expected_workflow_revision = Some(workflow.revision() + 1);
    stale_request.operation_key = Some("publish-stale-source".to_string());
    let stale_error = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(stale_request))
        .expect_err("stale workflow publication should be rejected");
    assert!(matches!(
        stale_error,
        DaemonError::WorkflowRevisionConflict {
            expected_revision,
            current_revision,
            ..
        } if expected_revision == workflow.revision() + 1
            && current_revision == workflow.revision()
    ));

    let (publication, projected_session) = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            request.clone(),
        ))
        .expect("publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated {
            publication,
            session,
        } => (publication, session),
        _ => panic!("unexpected local response"),
    };
    assert_eq!(
        publication.source_workflow_revision(),
        Some(workflow.revision())
    );
    assert!(publication
        .source_snapshot_digest()
        .is_some_and(|digest| digest.starts_with("sha256:")));
    assert_eq!(
        publication.creation_operation_key(),
        Some("publish-immutable-source")
    );
    let projected_json =
        serde_json::to_value(&projected_session).expect("projected session should serialize");
    assert!(
        projected_json
            .get("workflow_publication_snapshots")
            .is_none(),
        "private publication snapshots leaked into response session: {projected_json}"
    );

    let export = || match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_digest,
            package_archive_base64,
            package_files,
            ..
        } => (package_digest, package_archive_base64, package_files),
        _ => panic!("unexpected local response"),
    };
    let before_source_removal = export();

    harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowDesignOp(
            ApplyWorkflowDesignOpRequest {
                session_id: graph.session_id.clone(),
                origin_client_id: "immutable-source-test".to_string(),
                op_id: "remove-source-workflow".to_string(),
                op: WorkflowDesignOp::WorkflowRemove {
                    workflow_id: graph.workflow_id.clone(),
                },
            },
        ))
        .expect("source workflow should be removable after publication");

    let replayed = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            request.clone(),
        ))
        .expect("idempotent publication should replay after source removal")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(replayed.id(), publication.id());
    let after_source_removal = export();
    assert_eq!(after_source_removal.0, before_source_removal.0);
    assert_eq!(after_source_removal.1, before_source_removal.1);
    let frozen_snapshot = package_json_file(&after_source_removal.2, "workflow.snapshot.json");
    assert_eq!(
        frozen_snapshot["workflow"]["id"],
        serde_json::json!(graph.workflow_id)
    );
    assert_eq!(
        frozen_snapshot["workflow"]["revision"],
        serde_json::json!(workflow.revision())
    );

    request.route = Some("/different/*".to_string());
    let conflict = harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(request))
        .expect_err("operation key reuse with different choices should fail");
    assert!(conflict
        .to_string()
        .contains("operation key is already bound to different publication choices"));
    let listed = match harness
        .dispatch(LocalDaemonRequest::ListWorkflowPublications(
            ListWorkflowPublicationsRequest {
                session_id: graph.session_id.clone(),
            },
        ))
        .expect("publication list should load")
    {
        LocalDaemonResponse::WorkflowPublicationsListed { publications } => publications,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(listed.len(), 1);

    let durable_session = harness.with_app(|app| {
        app.sessions()
            .get_session(&graph.session_id)
            .expect("durable session should load")
    });
    let durable_json = serde_json::to_value(&durable_session)
        .expect("durable publication session should serialize");
    assert!(durable_json
        .get("workflow_publication_snapshots")
        .and_then(|snapshots| snapshots.get(publication.id()))
        .is_some());
    let restored: crate::session::RuntimeSession = serde_json::from_value(durable_json)
        .expect("durable publication session should deserialize");
    let restored_snapshot = restored
        .workflow_publication_snapshot(publication.id())
        .expect("immutable source snapshot should survive restoration");
    assert_eq!(
        restored_snapshot
            .digest()
            .expect("restored snapshot should hash"),
        publication
            .source_snapshot_digest()
            .expect("publication should expose a source digest")
    );
    assert!(
        serde_json::to_value(restored.redacted_for_user(publication.created_by_user_id()))
            .expect("redacted session should serialize")
            .get("workflow_publication_snapshots")
            .is_none()
    );

    let mut tampered_json =
        serde_json::to_value(&durable_session).expect("durable session should serialize again");
    tampered_json["workflow_publication_snapshots"][publication.id()]["workflow"]["alias"] =
        serde_json::json!("tampered-after-publication");
    let tampered_session: crate::session::RuntimeSession = serde_json::from_value(tampered_json)
        .expect("tampered durable session should still decode");
    harness.with_app_mut(|app| {
        app.sessions_mut().restore_session(tampered_session);
    });
    let tampered_error = harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect_err("tampered immutable source should not export");
    assert!(tampered_error.to_string().contains("workflow publication"));
    assert!(tampered_error.to_string().contains("was not found"));
}

#[test]
fn restored_sessions_drop_invalid_publications_without_immutable_snapshots() {
    let harness = LocalRouterTestHarness::new();
    let graph = create_publication_test_graph(&harness, "legacy-source");
    let publication = crate::session::WorkflowPublicationDefinition::new(
        "invalid-publication",
        graph.session_id.clone(),
        graph.workflow_id.clone(),
        graph.endpoint_id.clone(),
        Some("default".to_string()),
        Some("invalid-source".to_string()),
        "ingress",
        Some("/legacy/*".to_string()),
        vec!["POST".to_string()],
        Some(serde_json::json!({ "kind": "human_http" })),
        None,
        None,
        None,
        Some("async".to_string()),
        None,
        None,
        crate::session::DEFAULT_LOCAL_USER_ID,
    );
    harness.with_app_mut(|app| {
        app.sessions_mut()
            .restore_workflow_publication(&graph.session_id, publication.clone(), None)
            .expect("invalid publication fixture should restore");
    });
    harness.with_app_mut(|app| {
        let session = app
            .sessions()
            .get_session(&graph.session_id)
            .expect("session should exist before restore cleanup");
        let restored = app.sessions_mut().restore_session(session);
        assert!(restored.workflow_publications().is_empty());
    });
}

#[test]
fn publication_package_omits_runtime_agent_state_and_remains_stable() {
    let harness = LocalRouterTestHarness::new();
    let graph = create_publication_test_graph(&harness, "stable-runtime-state");
    let publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                expected_workflow_revision: None,
                operation_key: None,
                queue_ref: Some("default".to_string()),
                alias: Some("stable-runtime-state".to_string()),
                kind: Some("ingress".to_string()),
                route: Some("/prompt/*".to_string()),
                methods: vec!["GET".to_string()],
                transport: Some(serde_json::json!({ "kind": "human_http" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.agents()
            .bind_remote_execution(
                &graph.agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("provider-run-1".to_string()),
                    relay_url: Some("wss://relay.example.test".to_string()),
                    relay_token: Some("relay-secret-must-not-ship".to_string()),
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        app.agents()
            .set_agent_state(&graph.agent_id, crate::agent::AgentState::Working)
            .expect("agent should enter working state");
        app.agents()
            .set_agent_processing(&graph.agent_id, true)
            .expect("agent should enter processing state");
        app.agents()
            .note_prompt_sent_at(&graph.agent_id, 42)
            .expect("agent prompt activity should be recorded");
    });

    let export = || match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_digest,
            package_archive_base64,
            package_files,
            ..
        } => (package_digest, package_archive_base64, package_files),
        _ => panic!("unexpected local response"),
    };
    let first = export();

    harness.with_app_mut(|app| {
        app.agents()
            .set_remote_execution_active_worker_provider_run_id(&graph.agent_id, None)
            .expect("worker provider run should settle");
        app.agents()
            .set_agent_state(&graph.agent_id, crate::agent::AgentState::Idle)
            .expect("agent should return to idle");
        app.agents()
            .set_agent_processing(&graph.agent_id, false)
            .expect("agent processing should settle");
        app.agents()
            .note_prompt_sent_at(&graph.agent_id, 84)
            .expect("later prompt activity should be recorded");
    });
    let second = export();

    assert_eq!(
        second.0, first.0,
        "runtime-only agent changes altered the package digest"
    );
    assert_eq!(
        second.1, first.1,
        "runtime-only agent changes altered the package archive"
    );
    let snapshot = package_json_file(&second.2, "workflow.snapshot.json");
    let exported_agent = &snapshot["agents"][0];
    assert!(exported_agent.get("remote_execution").is_none());
    assert!(exported_agent.get("provider_resume_state").is_none());
    assert!(exported_agent.get("external_provider_import").is_none());
    assert!(exported_agent
        .get("remote_extension_manifest_sync")
        .is_none());
    assert_eq!(exported_agent["state"], serde_json::json!("Idle"));
    assert_eq!(exported_agent["is_processing"], serde_json::json!(false));
    assert!(exported_agent.get("last_prompt_sent_at_ms").is_none());
    assert_eq!(
        exported_agent["last_activity_at_ms"],
        exported_agent["created_at_ms"]
    );
    assert!(!serde_json::to_string(&snapshot)
        .expect("snapshot should serialize")
        .contains("relay-secret-must-not-ship"));
}

#[test]
fn local_request_api_exports_agent_app_publication_package() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-agent-app", "worktree-agent-app"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("shopper".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("workflow agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("shopping".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let node = match harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: agent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        _ => panic!("unexpected local response"),
    };
    let endpoint = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                entry_node_id: node.id().to_string(),
                alias: Some("add".to_string()),
                expected_workflow_revision: None,
            },
        ))
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        _ => panic!("unexpected local response"),
    };
    let publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                endpoint_ref: endpoint.id().to_string(),
                expected_workflow_revision: None,
                operation_key: None,
                queue_ref: Some("default".to_string()),
                alias: Some("shopping-app".to_string()),
                kind: Some("ingress".to_string()),
                route: Some("/add/*".to_string()),
                methods: vec!["GET".to_string()],
                transport: Some(serde_json::json!({ "kind": "human_http" })),
                parser: Some(serde_json::json!({
                    "kind": "path_template",
                    "template": "/add/:prompt"
                })),
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("workflow publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let assets_root = std::env::temp_dir().join(format!(
        "chariox-agent-app-assets-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(assets_root.join("assets")).expect("asset directory should be created");
    std::fs::write(
        assets_root.join("index.html"),
        "<!doctype html><main>shop</main>",
    )
    .expect("index asset should write");
    std::fs::write(assets_root.join("assets/catalog.json"), "{\"items\":[]}")
        .expect("nested asset should write");

    let agent_app = serde_json::json!({
        "enabled": true,
        "assets": {
            "public_dir": "app",
            "index": "index.html"
        },
        "routes": [{
            "path": "/add/*",
            "hook_id": format!("{}-hook", publication.id()),
            "prompt_source": "path_tail",
            "response": "streaming_shell",
            "required_role": "public",
            "manipulation": {
                "level": "state_and_overlay",
                "scope": "session",
                "allowed_actions": ["cart.search", "cart.add"]
            }
        }],
        "replicas": {
            "count": 1,
            "per_caller_ordering": true
        },
        "network": {
            "destinations": [{
                "id": "integration:catalog-api",
                "host": "api.catalog.example",
                "credential_slot_ids": []
            }]
        },
        "persistent_patch": {
            "enabled": false
        }
    });
    let export_request = || {
        LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: session.id().to_string(),
                publication_ref: publication.id().to_string(),
                kernel_url: Some("ws://127.0.0.1:43118".to_string()),
                agent_app: Some(agent_app.clone()),
                agent_app_assets_dir: Some(assets_root.to_string_lossy().to_string()),
            },
        )
    };
    let (package_digest, package_archive_base64, exported) = match harness
        .dispatch(export_request())
        .expect("agent app publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_version,
            package_digest,
            package_archive_base64,
            package_files,
            ..
        } => {
            assert_eq!(package_version, 3);
            (package_digest, package_archive_base64, package_files)
        }
        _ => panic!("unexpected local response"),
    };
    match harness
        .dispatch(export_request())
        .expect("repeated agent app publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_digest: repeated_digest,
            package_archive_base64: repeated_archive,
            ..
        } => {
            assert_eq!(repeated_digest, package_digest);
            assert_eq!(repeated_archive, package_archive_base64);
        }
        _ => panic!("unexpected repeated local response"),
    }
    let publication_json = package_json_file(&exported, "publication.json");
    let workflow_snapshot = package_json_file(&exported, "workflow.snapshot.json");
    assert_eq!(
        workflow_snapshot["source_session"]["workspace_id"],
        serde_json::json!("/workspace")
    );
    assert_eq!(
        workflow_snapshot["source_session"]["worktree_id"],
        serde_json::json!("/workspace")
    );
    assert_eq!(
        workflow_snapshot["agents"][0]["workspace_id"],
        serde_json::json!("/workspace")
    );
    assert_eq!(
        workflow_snapshot["agents"][0]["worktree_id"],
        serde_json::json!("/workspace")
    );
    assert_eq!(publication_json["package_version"], serde_json::json!(3));
    assert_eq!(
        publication_json["deployment_contract"],
        serde_json::json!({
            "path": "deployment-contract.json",
            "schema_version": 1,
        })
    );
    let deployment_contract = package_json_file(&exported, "deployment-contract.json");
    let deployment_contract_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../schema/workflow-publication-deployment-contract-v1.schema.json"
    ))
    .expect("deployment contract schema should parse");
    let compiled_schema = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&deployment_contract_schema)
        .expect("deployment contract schema should compile");
    assert!(
        compiled_schema.is_valid(&deployment_contract),
        "exported deployment contract should satisfy its versioned schema"
    );
    assert_eq!(deployment_contract["schema_version"], serde_json::json!(1));
    assert_eq!(
        deployment_contract["compatibility"]["package_version"],
        serde_json::json!(3)
    );
    assert_eq!(
        deployment_contract["compatibility"]["minimum_local_daemon_protocol_version"],
        serde_json::json!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION)
    );
    assert_eq!(
        deployment_contract["provider_requirements"][0]["provider"],
        serde_json::json!("dev-stub")
    );
    assert_eq!(
        deployment_contract["credential_slots"][0]["slot_id"],
        serde_json::json!("provider:dev-stub")
    );
    assert_eq!(
        deployment_contract["credential_slots"][0]["allowed_destination_ids"],
        serde_json::json!([])
    );
    let provider_configuration = &deployment_contract["configuration"][0];
    let configured_agent_id = provider_configuration["agent_id"]
        .as_str()
        .expect("provider configuration should name its immutable agent");
    assert_eq!(
        provider_configuration["key"],
        serde_json::json!(format!("provider_profile:{configured_agent_id}"))
    );
    assert_eq!(
        provider_configuration["kind"],
        serde_json::json!("provider_profile")
    );
    assert_eq!(provider_configuration["required"], serde_json::json!(true));
    assert_eq!(provider_configuration["secret"], serde_json::json!(false));
    assert_eq!(
        provider_configuration["allowed_providers"],
        serde_json::json!(["dev-stub"])
    );
    assert_eq!(
        provider_configuration["captured"],
        serde_json::json!({
            "provider": "dev-stub",
            "model": "default",
            "effort": null,
        })
    );
    assert_eq!(
        provider_configuration["node_ids"],
        deployment_contract["provider_requirements"][0]["node_ids"]
    );
    assert_eq!(
        deployment_contract["capabilities"]["network"],
        serde_json::json!({
            "policy_version": 1,
            "default_action": "deny",
            "destinations": [{
                "id": "integration:catalog-api",
                "host": { "kind": "exact_dns", "value": "api.catalog.example" },
                "ports": [443],
                "protocols": ["tls"],
                "credential_slot_ids": [],
            }],
            "provider_access": [{
                "slot_id": "provider:dev-stub",
                "bundle_kind": "development_stub",
                "bundle_id": "dev-stub-v1",
            }],
        })
    );
    assert_eq!(
        deployment_contract["presentation"]["kind"],
        serde_json::json!("agent_app")
    );
    assert_eq!(
        deployment_contract["routes"][0]["id"],
        serde_json::json!(format!("{}-hook", publication.id()))
    );
    assert_eq!(
        deployment_contract["routes"][0]["path"],
        serde_json::json!("/add/*")
    );
    assert_eq!(
        deployment_contract["routes"][0]["methods"],
        serde_json::json!(["GET"])
    );
    assert_eq!(
        deployment_contract["routes"][0]["required_roles"],
        serde_json::json!(["public"])
    );
    assert_eq!(
        deployment_contract["routes"][0]["session"],
        serde_json::json!({
            "scope": "session",
            "per_caller_ordering": true,
        })
    );
    assert_eq!(
        deployment_contract["resources"]["replicas"],
        serde_json::json!(1)
    );
    assert!(deployment_contract["presentation"]["assets"]
        .as_array()
        .is_some_and(|assets| assets.iter().any(|asset| {
            asset["path"] == serde_json::json!("app/index.html")
                && asset["sha256"]
                    .as_str()
                    .is_some_and(|digest| digest.starts_with("sha256:"))
        })));
    assert_eq!(publication_json["kind"], serde_json::json!("ingress"));
    assert_eq!(
        publication_json["agent_app"]["routes"][0]["path"],
        serde_json::json!("/add/*")
    );
    assert_eq!(
        package_text_file(&exported, "app/index.html"),
        "<!doctype html><main>shop</main>"
    );
    assert_eq!(
        package_text_file(&exported, "app/assets/catalog.json"),
        "{\"items\":[]}"
    );
    std::fs::remove_dir_all(assets_root).expect("asset directory should clean up");
}

#[test]
fn local_request_api_validates_publication_transport_options() {
    let harness = LocalRouterTestHarness::new();
    let graph = create_publication_test_graph(&harness, "publication-validation");

    let human_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                expected_workflow_revision: None,
                operation_key: None,
                queue_ref: Some("default".to_string()),
                alias: Some("human-defaults".to_string()),
                kind: Some("ingress".to_string()),
                route: None,
                methods: Vec::new(),
                transport: Some(serde_json::json!({ "kind": "human_http" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("human_http publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let exported_human = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: human_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("human_http publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let human_json = package_json_file(&exported_human, "publication.json");
    assert_eq!(human_json["hooks"][0]["route"], serde_json::json!("/"));
    assert_eq!(
        human_json["hooks"][0]["methods"],
        serde_json::json!(["GET", "POST"])
    );
    assert_eq!(
        human_json["hooks"][0]["parser"],
        serde_json::json!({"kind": "query_params"})
    );
    assert_eq!(human_json["hooks"][0]["mode"], serde_json::json!("async"));
    let human_app_js = package_text_file(&exported_human, "public/app.js");
    assert!(human_app_js.contains("const routePattern = \"/\""));
    assert!(human_app_js.contains("routePattern.indexOf('*')"));
    assert!(human_app_js.contains("window.location.href = invocationUrl(prompt)"));
    assert!(!human_app_js.contains("window.location.href = `/${encodeURIComponent(prompt)}`"));

    for (alias, parser) in [
        ("human-json-parser", serde_json::json!({ "kind": "json" })),
        (
            "human-query-parser",
            serde_json::json!({ "kind": "query_params" }),
        ),
        (
            "human-webhook-parser",
            serde_json::json!({ "kind": "webhook" }),
        ),
    ] {
        harness
            .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
                CreateWorkflowPublicationRequest {
                    session_id: graph.session_id.clone(),
                    workflow_ref: graph.workflow_id.clone(),
                    endpoint_ref: graph.endpoint_id.clone(),
                    expected_workflow_revision: None,
                    operation_key: None,
                    queue_ref: Some("default".to_string()),
                    alias: Some(alias.to_string()),
                    kind: Some("ingress".to_string()),
                    route: Some("/prompt/*".to_string()),
                    methods: vec!["GET".to_string(), "POST".to_string()],
                    transport: Some(serde_json::json!({ "kind": "human_http" })),
                    parser: Some(parser),
                    input_schema: None,
                    trace_exposure: None,
                    mode: Some("async".to_string()),
                    sync_timeout_ms: None,
                    poll_ms: None,
                },
            ))
            .expect("supported human_http parser should be created");
    }

    let human_regex_parser = harness.dispatch(LocalDaemonRequest::CreateWorkflowPublication(
        CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            expected_workflow_revision: None,
            operation_key: None,
            queue_ref: Some("default".to_string()),
            alias: Some("human-regex-parser".to_string()),
            kind: Some("ingress".to_string()),
            route: Some("/prompt/*".to_string()),
            methods: vec!["GET".to_string()],
            transport: Some(serde_json::json!({ "kind": "human_http" })),
            parser: Some(serde_json::json!({
                "kind": "regex",
                "source": "path",
                "pattern": "^/prompt/(?<prompt>.+)$"
            })),
            input_schema: None,
            trace_exposure: None,
            mode: Some("async".to_string()),
            sync_timeout_ms: None,
            poll_ms: None,
        },
    ));
    assert!(human_regex_parser
        .expect_err("human_http regex parser should fail")
        .to_string()
        .contains("human_http publications do not support parser `regex`"));

    let schedule_without_watchdog = harness.dispatch(
        LocalDaemonRequest::CreateWorkflowPublication(CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            expected_workflow_revision: None,
            operation_key: None,
            queue_ref: Some("default".to_string()),
            alias: Some("schedule-without-watchdog".to_string()),
            kind: Some("schedule_only".to_string()),
            route: None,
            methods: Vec::new(),
            transport: None,
            parser: None,
            input_schema: None,
            trace_exposure: None,
            mode: None,
            sync_timeout_ms: None,
            poll_ms: None,
        }),
    );
    assert!(schedule_without_watchdog
        .expect_err("schedule_only publication without enabled schedule should fail")
        .to_string()
        .contains("require an enabled schedule"));

    let conflicting_kind_and_transport = harness.dispatch(
        LocalDaemonRequest::CreateWorkflowPublication(CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            expected_workflow_revision: None,
            operation_key: None,
            queue_ref: Some("default".to_string()),
            alias: Some("conflicting-publication-kind".to_string()),
            kind: Some("ingress".to_string()),
            route: None,
            methods: Vec::new(),
            transport: Some(serde_json::json!({ "kind": "schedule_only" })),
            parser: None,
            input_schema: None,
            trace_exposure: None,
            mode: None,
            sync_timeout_ms: None,
            poll_ms: None,
        }),
    );
    assert!(conflicting_kind_and_transport
        .expect_err("ingress publication with schedule_only transport should fail")
        .to_string()
        .contains("ingress publications must use an ingress transport"));

    harness
        .dispatch(LocalDaemonRequest::CreateWorkflowSchedule(
            CreateWorkflowScheduleRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                trigger: crate::session::WorkflowScheduleTrigger::interval(300),
                invocation_prompt: "scheduled prompt".to_string(),
                overlap_policy: crate::session::WorkflowScheduleOverlapPolicy::Skip,
                max_runs_configured: false,
                max_runs: None,
            },
        ))
        .expect("workflow schedule should be created");
    let schedule_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                expected_workflow_revision: None,
                operation_key: None,
                queue_ref: Some("default".to_string()),
                alias: Some("schedule-only".to_string()),
                kind: Some("schedule_only".to_string()),
                route: None,
                methods: Vec::new(),
                transport: None,
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("schedule_only publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let exported_schedule = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: schedule_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("schedule_only publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let schedule_json = package_json_file(&exported_schedule, "publication.json");
    assert_eq!(schedule_publication.kind(), "schedule_only");
    assert_eq!(schedule_json["kind"], serde_json::json!("schedule_only"));
    assert_eq!(
        schedule_json["hooks"][0]["transport"],
        serde_json::json!("schedule_only")
    );
    assert!(schedule_json["hooks"][0].get("route").is_none());
    assert!(schedule_json["hooks"][0].get("methods").is_none());
    assert!(schedule_json["hooks"][0].get("parser").is_none());
    assert!(schedule_json["hooks"][0].get("mode").is_none());
}

#[test]
fn workflow_node_add_rejects_metaagents() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-meta-workflow", "worktree-meta-workflow"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let metaagent = harness.spawn_workflow_test_agent(session.id(), "meta");
    let metaagent = harness.with_app_mut(|app| {
        app.agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("test agent should enter meta mode")
    });
    let workflow = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session.id().to_string(),
            alias: Some("graph".to_string()),
        }))
        .expect("workflow create should succeed")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        _ => panic!("unexpected local response"),
    };
    let error = harness
        .dispatch(LocalDaemonRequest::AddWorkflowNode(
            AddWorkflowNodeRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                agent_id: metaagent.id().to_string(),
                expected_workflow_revision: None,
            },
        ))
        .expect_err("metaagent workflow node should be rejected");
    assert!(
        error
            .to_string()
            .contains("metaagents cannot be added as workflow nodes"),
        "unexpected error: {error}"
    );
}
