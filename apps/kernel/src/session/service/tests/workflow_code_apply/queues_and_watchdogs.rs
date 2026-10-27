use super::*;

#[test]
fn workflow_code_apply_auto_layouts_missing_canvas_coordinates() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    for node in &mut definition.nodes {
        node.canvas = None;
    }
    for endpoint in &mut definition.endpoints {
        endpoint.canvas = None;
    }
    for edge in &mut definition.edges {
        edge.canvas = None;
    }

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply");

    assert!(report.canvas_layout_applied);
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.code == "canvas_auto_layout_applied"));
    assert!(!report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"));

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    let layout = workflow
        .canvas_layout()
        .expect("auto-layout should create canvas layout");
    assert_eq!(layout.nodes.len(), 2);
    assert_eq!(layout.endpoints.len(), 1);
    assert!(layout.edges.is_empty());

    let planner_id = report.node_ids.get("planner").expect("planner id");
    let worker_id = report.node_ids.get("worker").expect("worker id");
    let entry_id = report.endpoint_ids.get("entry").expect("entry id");
    let planner_point = layout.nodes.get(planner_id).expect("planner point");
    let worker_point = layout.nodes.get(worker_id).expect("worker point");
    let entry_point = layout.endpoints.get(entry_id).expect("entry point");
    assert!(worker_point.x > planner_point.x);
    assert!(entry_point.x < planner_point.x);
    assert_eq!(entry_point.y, planner_point.y);
}

#[test]
fn workflow_code_apply_maps_omitted_queues_to_kernel_default_queue() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.queues.clear();
    definition.schedules.clear();

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply with the default prompt queue");

    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"
            && warning.handle.as_deref() == Some("default")));

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let default_queue = session
        .workflow_prompt_queue(&report.workflow_id, "default")
        .expect("kernel default workflow queue should exist");
    assert_eq!(
        report.queue_ids.get("default").map(String::as_str),
        Some(default_queue.id())
    );
    assert_eq!(default_queue.alias(), "default");
    assert_eq!(default_queue.priority(), 0);
    assert!(default_queue.enabled());
}

#[test]
fn workflow_code_apply_normalizes_explicit_default_queue_alias() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.queues.clear();
    definition.queues.push(WorkflowCodeQueueDefinition {
        handle: "script_default".to_string(),
        alias: " Default ".to_string(),
        priority: 7,
        enabled: false,
    });
    definition.schedules.clear();

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply with normalized default queue alias");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let default_queue = session
        .workflow_prompt_queue(&report.workflow_id, "default")
        .expect("kernel default workflow queue should exist");

    assert_eq!(
        report.queue_ids.get("script_default").map(String::as_str),
        Some(default_queue.id())
    );
    assert_eq!(default_queue.alias(), "default");
    assert_eq!(default_queue.priority(), 7);
    assert!(!default_queue.enabled());
    assert_eq!(
        session
            .workflow_prompt_queues_for_workflow(&report.workflow_id)
            .len(),
        1
    );
}

#[test]
fn workflow_code_apply_maps_watchdogs_to_implicit_default_queue_when_other_queues_exist() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.schedules[0].queue = Some("default".to_string());

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply with explicit and implicit queues");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let default_queue = session
        .workflow_prompt_queue(&report.workflow_id, "default")
        .expect("kernel default workflow queue should exist");
    let urgent_queue = session
        .workflow_prompt_queue(
            &report.workflow_id,
            report.queue_ids.get("urgent").expect("urgent queue id"),
        )
        .expect("scripted urgent queue should exist");
    let watchdog = session
        .workflow_watchdog(
            report
                .schedule_ids
                .get("entry_watchdog")
                .expect("watchdog id"),
        )
        .expect("watchdog should exist");

    assert_eq!(
        report.queue_ids.get("default").map(String::as_str),
        Some(default_queue.id())
    );
    assert_ne!(default_queue.id(), urgent_queue.id());
    assert_eq!(watchdog.queue_id(), Some(default_queue.id()));
    assert!(!report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"));
}
