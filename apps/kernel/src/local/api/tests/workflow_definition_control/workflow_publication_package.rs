use super::*;

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
        "arroba-agent-app-assets-{}",
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

    let exported = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: session.id().to_string(),
                publication_ref: publication.id().to_string(),
                kernel_url: Some("ws://127.0.0.1:43118".to_string()),
                agent_app: Some(serde_json::json!({
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
                    "persistent_patch": {
                        "enabled": false
                    }
                })),
                agent_app_assets_dir: Some(assets_root.to_string_lossy().to_string()),
            },
        ))
        .expect("agent app publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_version,
            package_files,
            ..
        } => {
            assert_eq!(package_version, 2);
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let publication_json = package_json_file(&exported, "publication.json");
    assert_eq!(publication_json["package_version"], serde_json::json!(2));
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
    assert_eq!(
        human_json["hooks"][0]["route"],
        serde_json::json!("/prompt/*")
    );
    assert_eq!(
        human_json["hooks"][0]["methods"],
        serde_json::json!(["GET", "POST"])
    );
    assert_eq!(
        human_json["hooks"][0]["parser"],
        serde_json::json!({"kind": "path_template", "template": "/prompt/:prompt"})
    );
    assert_eq!(human_json["hooks"][0]["mode"], serde_json::json!("async"));
    let human_app_js = package_text_file(&exported_human, "public/app.js");
    assert!(human_app_js.contains("const routePattern = \"/prompt/*\""));
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

    let api_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                alias: Some("api-default-route".to_string()),
                kind: Some("ingress".to_string()),
                route: None,
                methods: vec!["POST".to_string()],
                transport: Some(serde_json::json!({ "kind": "api_sse_json" })),
                parser: Some(serde_json::json!({ "kind": "json" })),
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: Some(30_000),
                poll_ms: Some(250),
            },
        ))
        .expect("api publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };

    let exported = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: api_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("api publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let publication_json = package_json_file(&exported, "publication.json");
    assert_eq!(publication_json["kind"], serde_json::json!("ingress"));
    assert_eq!(
        publication_json["hooks"][0]["route"],
        serde_json::json!("/invoke")
    );
    assert_eq!(
        publication_json["hooks"][0]["queue_ref"],
        serde_json::json!("default")
    );
    assert_eq!(
        publication_json["hooks"][0]["methods"],
        serde_json::json!(["POST"])
    );
    assert_eq!(
        publication_json["hooks"][0]["parser"],
        serde_json::json!({ "kind": "json" })
    );
    assert_eq!(
        publication_json["hooks"][0]["mode"],
        serde_json::json!("async")
    );
    let mcp_publication = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id.clone(),
                workflow_ref: graph.workflow_id.clone(),
                endpoint_ref: graph.endpoint_id.clone(),
                queue_ref: Some("default".to_string()),
                alias: Some("mcp-defaults".to_string()),
                kind: Some("ingress".to_string()),
                route: None,
                methods: Vec::new(),
                transport: Some(serde_json::json!({ "kind": "mcp" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: None,
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("mcp publication should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    let exported_mcp = match harness
        .dispatch(LocalDaemonRequest::ExportWorkflowPublicationPackage(
            ExportWorkflowPublicationPackageRequest {
                session_id: graph.session_id.clone(),
                publication_ref: mcp_publication.id().to_string(),
                kernel_url: None,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        ))
        .expect("mcp publication package should export")
    {
        LocalDaemonResponse::WorkflowPublicationPackageExported { package_files, .. } => {
            package_files
        }
        _ => panic!("unexpected local response"),
    };
    let mcp_json = package_json_file(&exported_mcp, "publication.json");
    assert_eq!(mcp_json["hooks"][0]["route"], serde_json::json!("/mcp"));
    assert_eq!(mcp_json["hooks"][0]["methods"], serde_json::json!(["POST"]));
    assert_eq!(mcp_json["hooks"][0]["mode"], serde_json::json!("sync"));
    assert!(mcp_json["hooks"][0].get("parser").is_none());

    let schedule_without_watchdog = harness.dispatch(
        LocalDaemonRequest::CreateWorkflowPublication(CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
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

    let api_sync = harness.dispatch(LocalDaemonRequest::CreateWorkflowPublication(
        CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            queue_ref: Some("default".to_string()),
            alias: Some("api-sync".to_string()),
            kind: Some("ingress".to_string()),
            route: Some("/api".to_string()),
            methods: vec!["POST".to_string()],
            transport: Some(serde_json::json!({ "kind": "api_sse_json" })),
            parser: Some(serde_json::json!({ "kind": "json" })),
            input_schema: None,
            trace_exposure: None,
            mode: Some("sync".to_string()),
            sync_timeout_ms: None,
            poll_ms: None,
        },
    ));
    assert!(api_sync
        .expect_err("api_sse_json sync mode should fail")
        .to_string()
        .contains("api_sse_json publications always use async"));

    let mcp_parser = harness.dispatch(LocalDaemonRequest::CreateWorkflowPublication(
        CreateWorkflowPublicationRequest {
            session_id: graph.session_id.clone(),
            workflow_ref: graph.workflow_id.clone(),
            endpoint_ref: graph.endpoint_id.clone(),
            queue_ref: Some("default".to_string()),
            alias: Some("mcp-json".to_string()),
            kind: Some("ingress".to_string()),
            route: Some("/mcp".to_string()),
            methods: vec!["POST".to_string()],
            transport: Some(serde_json::json!({ "kind": "mcp" })),
            parser: Some(serde_json::json!({ "kind": "json" })),
            input_schema: None,
            trace_exposure: None,
            mode: Some("sync".to_string()),
            sync_timeout_ms: None,
            poll_ms: None,
        },
    ));
    assert!(mcp_parser
        .expect_err("mcp parser override should fail")
        .to_string()
        .contains("mcp publications read input from MCP tool arguments"));

    let websocket_custom_route = match harness
        .dispatch(LocalDaemonRequest::CreateWorkflowPublication(
            CreateWorkflowPublicationRequest {
                session_id: graph.session_id,
                workflow_ref: graph.workflow_id,
                endpoint_ref: graph.endpoint_id,
                queue_ref: Some("default".to_string()),
                alias: Some("custom-ws".to_string()),
                kind: Some("ingress".to_string()),
                route: Some("/socket".to_string()),
                methods: Vec::new(),
                transport: Some(serde_json::json!({ "kind": "websocket_json" })),
                parser: None,
                input_schema: None,
                trace_exposure: None,
                mode: Some("async".to_string()),
                sync_timeout_ms: None,
                poll_ms: None,
            },
        ))
        .expect("websocket_json custom route should be created")
    {
        LocalDaemonResponse::WorkflowPublicationCreated { publication, .. } => publication,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(websocket_custom_route.route(), Some("/socket"));
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
