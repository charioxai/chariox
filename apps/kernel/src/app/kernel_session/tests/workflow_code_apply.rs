use super::*;

#[test]
fn workflow_code_apply_rejects_missing_node_extension_requirement() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let mut definition = generated_workflow_code_definition();
    definition.nodes[0].extensions.push(ExtensionGrant::new(
        ExtensionKind::Skill,
        "missing-workflow-code-skill",
    ));

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("workflow-code should reject missing extension requirements");

    let message = format!("{error}");
    assert!(message.contains(
        "node `planner` extension requirement `skill:missing-workflow-code-skill` cannot be satisfied"
    ));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
}

#[test]
fn workflow_code_apply_preflights_existing_agent_authorization_without_partial_mutation() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"))
        .expect("peer worker should spawn");
    let agent_count_before = app.agents().get_session_agents(session.id()).len();

    let mut definition = generated_workflow_code_definition();
    definition.workflow.alias = Some("partial_mutation_guard".to_string());
    definition.nodes[1].agent = WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
        agent_ref: peer_worker.id().to_string(),
    });

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            metaagent.owner_user_id().to_string(),
            Some(metaagent.id().to_string()),
        )
        .expect_err("workflow-code should reject unauthorized existing-agent binding");

    let message = format!("{error}");
    assert!(message.contains("unauthorized_existing_agent_binding"));
    assert!(message.contains(peer_worker.id()));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
    assert_eq!(
        app.agents().get_session_agents(session.id()).len(),
        agent_count_before
    );
}

#[test]
fn workflow_code_apply_rejects_metaagent_as_existing_node_agent_without_partial_mutation() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let agent_count_before = app.agents().get_session_agents(session.id()).len();
    let definition = existing_agent_workflow_code_definition(metaagent.id());

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("workflow-code should reject metaagent as workflow node agent");

    let message = format!("{error}");
    assert!(message.contains("invalid_existing_agent_binding"));
    assert!(message.contains(metaagent.id()));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
    assert_eq!(
        app.agents().get_session_agents(session.id()).len(),
        agent_count_before
    );
}

#[test]
fn workflow_code_apply_allows_direct_user_to_bind_active_meta_task_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let agent = app
        .agents_mut()
        .activate_agent_meta_mode(agent.id(), None)
        .expect("agent should enter meta mode");
    app.sessions_mut()
        .start_or_update_metaagent_task(session.id(), agent.id(), "finish the active task")
        .expect("meta task should start");
    let definition = existing_agent_workflow_code_definition(agent.id());

    let report = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect(
            "direct user workflow should bind the active Meta task agent for deferred execution",
        );

    assert_eq!(
        report.agent_ids.get("planner").map(String::as_str),
        Some(agent.id())
    );
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert_eq!(session.workflows().len(), 1);
    assert!(session.has_active_metaagent_task());
}

#[test]
fn workflow_code_apply_grants_satisfied_node_extension_requirement() {
    let workspace = unique_workflow_code_test_workspace("extension-satisfied");
    install_test_skill(&workspace, "workflow-code-skill");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.display().to_string(),
            "worktree",
        ))
        .expect("session should create");
    let mut definition = generated_workflow_code_definition();
    definition.nodes[0].extensions.push(ExtensionGrant::new(
        ExtensionKind::Skill,
        "workflow-code-skill",
    ));

    let report = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect("workflow-code should apply when extension requirement exists");

    let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
    let planner = app
        .agents()
        .get_agent(planner_agent_id)
        .expect("planner agent should exist");
    assert!(planner
        .extension_grants()
        .iter()
        .any(|grant| grant.matches(&ExtensionKind::Skill, "workflow-code-skill")));
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn workflow_code_apply_grants_extensions_to_authorized_existing_agent() {
    let workspace = unique_workflow_code_test_workspace("existing-extension-satisfied");
    install_test_skill(&workspace, "workflow-code-skill");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.display().to_string(),
            "worktree",
        ))
        .expect("session should create");
    let existing_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("existing worker should spawn");
    let mut definition = existing_agent_workflow_code_definition(existing_agent.id());
    definition.nodes[0].extensions.push(ExtensionGrant::new(
        ExtensionKind::Skill,
        "workflow-code-skill",
    ));

    let report = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect("workflow-code should grant extensions to an existing bound agent");

    assert_eq!(
        report.agent_ids.get("planner").map(String::as_str),
        Some(existing_agent.id())
    );
    let existing_agent = app
        .agents()
        .get_agent(existing_agent.id())
        .expect("existing worker should still exist");
    assert!(existing_agent
        .extension_grants()
        .iter()
        .any(|grant| grant.matches(&ExtensionKind::Skill, "workflow-code-skill")));
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn workflow_code_javascript_compile_and_apply_creates_generated_workflow() {
    let Some(node_path) = find_node_for_workflow_code_test() else {
        eprintln!("skipping workflow-code JS apply test because node is not available");
        return;
    };
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let source = r#"
workflow.define({ alias: "js_coded_flow", maxConcurrent: 2 })
const planner = workflow.node({
  agent: workflow.newAgent({ alias: "js-planner", provider: "dev-stub", model: "default" }),
  publicLabel: "JS Planner",
  instructions: "Plan from JS."
})
const finisher = workflow.node({
  agent: workflow.newAgent({ alias: "js-finisher", provider: "dev-stub", model: "default" }),
  publicLabel: "JS Finisher",
  instructions: "Finish from JS.",
  canCompleteWorkflowRun: true
})
workflow.edge(planner, finisher)
workflow.endpoint(planner, { alias: "entry" })
"#;

    let result = app
        .compile_and_apply_workflow_code_javascript(
            session.id(),
            &node_path,
            source,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect("workflow-code JS should compile and apply");

    assert!(result.compile.validation.ok);
    assert_eq!(
        result.compile.definition.workflow.alias.as_deref(),
        Some("js_coded_flow")
    );
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    let workflow = session
        .workflow(&result.apply.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.alias(), Some("js_coded_flow"));
    assert_eq!(workflow.nodes().len(), 2);
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 3);
}

#[test]
fn workflow_code_javascript_apply_rejects_invalid_source_without_mutating_session() {
    let Some(node_path) = find_node_for_workflow_code_test() else {
        eprintln!("skipping invalid workflow-code JS apply test because node is not available");
        return;
    };
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let source = r#"
workflow.define({ alias: "invalid_coded_flow" })
workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
"#;

    let error = app
        .compile_and_apply_workflow_code_javascript(
            session.id(),
            &node_path,
            source,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("invalid workflow-code should not apply");

    assert!(format!("{error}").contains("missing_endpoint"));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
}

#[test]
fn workflow_code_canonical_patterns_compile_and_apply_with_provider_rebindings() {
    let Some(node_path) = find_node_for_workflow_code_test() else {
        eprintln!("skipping workflow-code pattern apply test because node is not available");
        return;
    };
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let limits = WorkflowCodeLimitsConfig::default();

    for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
        let compiled = crate::workflow_code::compile_workflow_code_javascript(
            &node_path,
            example.source,
            &limits,
        )
        .unwrap_or_else(|error| {
            panic!(
                "workflow-code pattern `{}` at `{}` should compile: {error}",
                example.slug, example.path
            )
        });
        assert!(
            compiled.validation.ok,
            "workflow-code pattern `{}` should validate before apply: {:?}",
            example.slug, compiled.validation.diagnostics
        );
        let provider_rebindings = compiled
            .definition
            .nodes
            .iter()
            .map(|node| WorkflowCodeProviderRebinding {
                node: node.handle.clone(),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
            })
            .collect::<Vec<_>>();

        let result = app
            .compile_and_apply_workflow_code_javascript_with_rebindings(
                session.id(),
                &node_path,
                example.source,
                &limits,
                "local-user".to_string(),
                None,
                &provider_rebindings,
                &[],
            )
            .unwrap_or_else(|error| {
                panic!(
                    "workflow-code pattern `{}` should apply after provider rebinding: {error}",
                    example.slug
                )
            });

        assert!(
            result.compile.validation.ok,
            "workflow-code pattern `{}` should compile with valid diagnostics",
            example.slug
        );
        assert_eq!(
            result.apply.node_ids.len(),
            result.compile.definition.nodes.len(),
            "workflow-code pattern `{}` should report every node id",
            example.slug
        );
        assert_eq!(
            result.apply.agent_ids.len(),
            result.compile.definition.nodes.len(),
            "workflow-code pattern `{}` should report every node agent id",
            example.slug
        );
        assert_eq!(
            result.apply.edge_ids.len(),
            result.compile.definition.edges.len(),
            "workflow-code pattern `{}` should report every edge id",
            example.slug
        );
        assert_eq!(
            result.apply.endpoint_ids.len(),
            result.compile.definition.endpoints.len(),
            "workflow-code pattern `{}` should report every endpoint id",
            example.slug
        );
        assert_eq!(
            result.apply.schema_refs.len(),
            result.compile.definition.schemas.len(),
            "workflow-code pattern `{}` should report every schema id",
            example.slug
        );
        assert!(
            result.apply.canvas_layout_applied,
            "workflow-code pattern `{}` should apply canvas layout",
            example.slug
        );

        let session_snapshot = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let workflow = session_snapshot
            .workflow(&result.apply.workflow_id)
            .unwrap_or_else(|| {
                panic!(
                    "workflow-code pattern `{}` should create a workflow",
                    example.slug
                )
            });
        assert_eq!(
            workflow.nodes().len(),
            result.compile.definition.nodes.len(),
            "workflow-code pattern `{}` should materialize all nodes",
            example.slug
        );
        assert_eq!(
            workflow.edges().len(),
            result.compile.definition.edges.len(),
            "workflow-code pattern `{}` should materialize all edges",
            example.slug
        );
        assert_eq!(
            workflow.endpoints().len(),
            result.compile.definition.endpoints.len(),
            "workflow-code pattern `{}` should materialize all endpoints",
            example.slug
        );
        assert_eq!(
            workflow.schemas().len(),
            result.compile.definition.schemas.len(),
            "workflow-code pattern `{}` should materialize all schemas",
            example.slug
        );
        assert!(
            workflow.run_output_schema_ref().is_some(),
            "workflow-code pattern `{}` should have final output schema",
            example.slug
        );
    }
}

#[test]
fn workflow_code_apply_rejects_metaagent_binding_unowned_existing_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let definition = existing_agent_workflow_code_definition(default_agent.id());

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            Some("meta-1".to_string()),
        )
        .expect_err("metaagent should not bind an agent it does not control");

    assert!(format!("{error}").contains("not authorized"));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
}
