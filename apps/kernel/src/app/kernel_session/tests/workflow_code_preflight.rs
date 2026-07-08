use super::*;

#[test]
fn create_session_writes_durable_state_event() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");

    let events = app
        .durable_state_store()
        .load_events_after(0)
        .expect("durable state events should load");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "session.created");
    assert_eq!(events[0].subject_id.as_deref(), Some(session.id()));
    assert_eq!(events[0].payload["session"]["id"], session.id());
    assert_eq!(events[0].payload["default_agent"]["id"], agent.id());
}

#[test]
fn spawn_agent_and_end_session_write_durable_state_events() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let spawned = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
        .expect("agent should spawn");
    crate::app::KernelSessionService::new(&mut app)
        .end_session(session.id())
        .expect("session should end");

    let events = app
        .durable_state_store()
        .load_events_after(0)
        .expect("durable state events should load");
    assert_eq!(
        events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["session.created", "agent.created", "session.ended"]
    );
    assert_eq!(events[1].subject_id.as_deref(), Some(spawned.id()));
    assert_eq!(events[2].subject_id.as_deref(), Some(session.id()));
}

#[test]
fn destroy_agent_writes_durable_state_event() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let spawned = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
        .expect("agent should spawn");

    let destroyed = crate::app::KernelSessionService::new(&mut app)
        .destroy_agent(spawned.id())
        .expect("agent should destroy");

    assert_eq!(destroyed.id(), spawned.id());
    let events = app
        .durable_state_store()
        .load_events_after(0)
        .expect("durable state events should load");
    assert_eq!(
        events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["session.created", "agent.created", "agent.deleted"]
    );
    assert_eq!(events[2].subject_id.as_deref(), Some(spawned.id()));
}

#[test]
fn workflow_code_apply_spawns_generated_agents_and_creates_workflow() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");

    let definition = generated_workflow_code_definition();
    let report = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect("workflow-code should apply");

    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.nodes().len(), 2);
    assert_eq!(workflow.edges().len(), 1);
    assert_eq!(workflow.endpoints().len(), 1);
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 3);
    assert_eq!(report.agent_ids.len(), 2);
    assert!(report.queue_ids.contains_key("default"));
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"));
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.code == "canvas_auto_layout_applied"));

    let events = app
        .durable_state_store()
        .load_events_after(0)
        .expect("durable events should load");
    assert!(events
        .iter()
        .any(|event| event.kind == "workflow_code.applied"
            && event.subject_id.as_deref() == Some(report.workflow_id.as_str())));
}

#[test]
fn workflow_code_apply_rebinds_generated_agent_provider() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");

    let definition = generated_workflow_code_definition();
    let report = app
        .apply_workflow_code_definition_with_rebindings(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
            &[WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "opencode".to_string(),
                model: Some("qwen3-coder".to_string()),
                effort: Some("medium".to_string()),
                account_profile: Some("profile-a".to_string()),
            }],
            &[],
        )
        .expect("workflow-code should apply with provider rebinding");

    let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
    let planner = app
        .agents()
        .get_agent(planner_agent_id)
        .expect("planner agent should exist");
    assert_eq!(planner.provider(), "opencode");
    assert_eq!(planner.model(), Some("qwen3-coder"));
    assert_eq!(planner.effort(), Some("medium"));
    assert_eq!(planner.account_profile(), Some("profile-a"));
}

#[test]
fn workflow_code_apply_agent_rebinding_reuses_existing_entry_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");

    let definition = generated_workflow_code_definition();
    let report = app
        .apply_workflow_code_definition_with_rebindings(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
            &[],
            &[WorkflowCodeAgentRebinding {
                node: "planner".to_string(),
                agent_ref: default_agent.id().to_string(),
            }],
        )
        .expect("workflow-code should apply with agent rebinding");

    assert_eq!(
        report.agent_ids.get("planner").map(String::as_str),
        Some(default_agent.id())
    );
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 2);

    let endpoint_id = report.endpoint_ids.get("entry").expect("entry endpoint id");
    let workflow_run = app
        .session_state_store()
        .write()
        .invoke_workflow_endpoint(
            session.id(),
            &report.workflow_id,
            endpoint_id,
            Some("Use the current agent as planner.".to_string()),
        )
        .expect("workflow run should create");
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert_eq!(workflow_run.node_runs()[0].agent_id(), default_agent.id());

    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    let planner_node_id = report.node_ids.get("planner").expect("planner node id");
    let planner_node = workflow
        .node(planner_node_id)
        .expect("planner node should exist");
    assert_eq!(planner_node.agent_id(), default_agent.id());
}

#[test]
fn workflow_code_apply_rejects_unavailable_generated_agent_provider() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let mut definition = generated_workflow_code_definition();
    if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
        agent.provider = "missing-provider".to_string();
    }

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("workflow-code should reject unavailable generated-agent provider");

    let message = format!("{error}");
    assert!(message.contains("node `planner` requests unavailable provider `missing-provider`"));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
}

#[test]
fn workflow_code_apply_rebinding_can_replace_unavailable_provider() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let mut definition = generated_workflow_code_definition();
    if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
        agent.provider = "missing-provider".to_string();
    }

    let report = app
        .apply_workflow_code_definition_with_rebindings(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
            &[WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
            }],
            &[],
        )
        .expect("workflow-code should apply after rebinding unavailable provider");

    let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
    let planner = app
        .agents()
        .get_agent(planner_agent_id)
        .expect("planner agent should exist");
    assert_eq!(planner.provider(), "dev-stub");
}

#[test]
fn workflow_code_apply_rejects_unavailable_generated_agent_model_from_catalog() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    cache_test_provider_catalog(&mut app);
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let mut definition = generated_workflow_code_definition();
    if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
        agent.provider = "codex".to_string();
        agent.model = Some("missing-model".to_string());
    }

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("workflow-code should reject unavailable generated-agent model");

    let message = format!("{error}");
    assert!(message.contains("unavailable_model"));
    assert!(message.contains("missing-model"));
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(session.workflows().is_empty());
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
}

#[test]
fn workflow_code_apply_rebinding_can_replace_unavailable_model() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    cache_test_provider_catalog(&mut app);
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let mut definition = generated_workflow_code_definition();
    if let WorkflowCodeAgentBinding::Create(agent) = &mut definition.nodes[0].agent {
        agent.provider = "codex".to_string();
        agent.model = Some("missing-model".to_string());
    }

    let report = app
        .apply_workflow_code_definition_with_rebindings(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
            &[WorkflowCodeProviderRebinding {
                node: "planner".to_string(),
                provider: "codex".to_string(),
                model: Some("gpt-5".to_string()),
                effort: None,
                account_profile: None,
            }],
            &[],
        )
        .expect("workflow-code should apply after rebinding unavailable model");

    let planner_agent_id = report.agent_ids.get("planner").expect("planner agent id");
    let planner = app
        .agents()
        .get_agent(planner_agent_id)
        .expect("planner agent should exist");
    assert_eq!(planner.provider(), "codex");
    assert_eq!(planner.model(), Some("gpt-5"));
}

#[test]
fn workflow_code_apply_rejects_runtime_queue_limit_before_spawning_agents() {
    let mut config = DaemonConfig::for_tests();
    config.user_config.workflow.max_queues_per_workflow = Some(1);
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");

    let mut definition = generated_workflow_code_definition();
    definition.queues.push(WorkflowCodeQueueDefinition {
        handle: "urgent".to_string(),
        alias: "urgent".to_string(),
        priority: 5,
        enabled: true,
    });
    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("runtime queue limit should reject before spawning generated agents");

    let message = format!("{error:?}");
    assert!(message.contains("limit_exceeded"), "{message}");
    assert!(
        message.contains("runtime workflow queue limit 1"),
        "{message}"
    );
    assert_eq!(app.agents().get_session_agents(session.id()).len(), 1);
    let session_after = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert!(!session_after
        .workflows()
        .iter()
        .any(|workflow| workflow.alias() == Some("generated_agents")));
}

#[test]
fn workflow_code_apply_rejects_exhausted_alias_allocation_before_spawning_agents() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");

    for attempt in 0..crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS {
        let alias = if attempt == 0 {
            "generated_agents".to_string()
        } else {
            format!("generated_agents-{}", attempt + 1)
        };
        app.session_state_store()
            .write()
            .create_workflow(session.id(), Some(alias))
            .expect("workflow alias candidate should be created");
    }
    let agent_count_before = app.agents().get_session_agents(session.id()).len();
    let workflow_count_before = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .workflows()
        .len();

    let error = app
        .apply_workflow_code_definition(
            session.id(),
            &generated_workflow_code_definition(),
            &WorkflowCodeLimitsConfig::default(),
            "local-user".to_string(),
            None,
        )
        .expect_err("workflow-code should reject exhausted alias allocation before spawning");

    let message = format!("{error}");
    assert!(message.contains("workflow_alias_unavailable"), "{message}");
    assert_eq!(
        app.agents().get_session_agents(session.id()).len(),
        agent_count_before
    );
    let session_after = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert_eq!(session_after.workflows().len(), workflow_count_before);
}
