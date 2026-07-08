use super::*;

#[test]
fn workflow_code_apply_rejects_missing_agent_resolution() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1"]);

    let definition = workflow_code_definition();
    let agent_ids = BTreeMap::from([("planner".to_string(), "agent-1".to_string())]);
    let error = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect_err("unresolved worker should fail");

    assert!(format!("{error}").contains("worker"));
}

#[test]
fn workflow_code_canonical_patterns_compile_and_apply_with_provider_rebindings() {
    let node_path = match discover_workflow_code_node_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("skipping workflow-code canonical pattern apply test: {error}");
            return;
        }
    };

    for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
        let mut compiled = compile_workflow_code_javascript(
            &node_path,
            example.source,
            &WorkflowCodeLimitsConfig::default(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "workflow-code pattern example `{}` at `{}` should compile: {error}",
                example.slug, example.path
            )
        })
        .definition;

        let rebindings = compiled
            .nodes
            .iter()
            .filter_map(|node| match node.agent {
                WorkflowCodeAgentBinding::Create(_) => Some(WorkflowCodeProviderRebinding {
                    node: node.handle.clone(),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                WorkflowCodeAgentBinding::Existing(_) => None,
            })
            .collect::<Vec<_>>();
        apply_workflow_code_provider_rebindings(&mut compiled, &rebindings).unwrap_or_else(
            |error| {
                panic!(
                    "`{}` provider rebindings should apply: {error}",
                    example.slug
                )
            },
        );
        assert!(
            compiled
                .validate_with_limits(&WorkflowCodeLimitsConfig::default())
                .ok,
            "`{}` should remain valid after provider rebinding",
            example.slug
        );

        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let agent_ids = compiled
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.handle.clone(),
                    format!("agent-{}-{index}", example.slug),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let seeded_agent_ids = agent_ids.values().map(String::as_str).collect::<Vec<_>>();
        seed_agents(&mut service, session.id(), &seeded_agent_ids);

        let report = service
            .apply_workflow_code_definition(
                session.id(),
                &compiled,
                &agent_ids,
                &WorkflowCodeLimitsConfig::default(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                Some("meta-1".to_string()),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "`{}` should apply to session primitives: {error}",
                    example.slug
                )
            });

        assert_eq!(
            report.node_ids.len(),
            compiled.nodes.len(),
            "`{}` should allocate one kernel node id per script node",
            example.slug
        );
        assert_eq!(
            report.edge_ids.len(),
            compiled.edges.len(),
            "`{}` should allocate one kernel edge id per script edge",
            example.slug
        );
        assert_eq!(
            report.endpoint_ids.len(),
            compiled.endpoints.len(),
            "`{}` should allocate one kernel endpoint id per script endpoint",
            example.slug
        );
        assert_eq!(
            report.schema_refs.len(),
            compiled.schemas.len(),
            "`{}` should allocate one kernel schema ref per script schema",
            example.slug
        );
        for node in &compiled.nodes {
            assert_ne!(
                report.node_ids.get(&node.handle).map(String::as_str),
                Some(node.handle.as_str()),
                "`{}` should not reuse script node handle as kernel node id",
                example.slug
            );
        }
        let applied_session = service
            .get_session(session.id())
            .expect("session should remain readable");
        assert!(
            applied_session
                .workflows()
                .iter()
                .any(|workflow| workflow.id() == report.workflow_id),
            "`{}` workflow should appear in the session projection",
            example.slug
        );
    }
}

#[test]
fn planner_worker_reviewer_pattern_preserves_goal_workflow_contract() {
    let node_path = match discover_workflow_code_node_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("skipping planner-worker-reviewer pattern test: {error}");
            return;
        }
    };
    let example = WORKFLOW_CODE_PATTERN_EXAMPLES
        .iter()
        .find(|example| example.slug == "planner-worker-reviewer")
        .expect("planner-worker-reviewer pattern should be bundled");
    let compiled = compile_workflow_code_javascript(
        &node_path,
        example.source,
        &WorkflowCodeLimitsConfig::default(),
    )
    .expect("planner-worker-reviewer should compile")
    .definition;

    assert_eq!(
        compiled.workflow.alias.as_deref(),
        Some("pattern-planner-worker-reviewer")
    );
    assert_eq!(compiled.nodes.len(), 3);
    assert_eq!(compiled.edges.len(), 4);
    assert_eq!(compiled.endpoints.len(), 1);
    assert_eq!(compiled.schemas.len(), 5);
    assert_eq!(
        compiled.workflow.run_output_schema.as_deref(),
        Some("final_output")
    );

    let planner = compiled
        .nodes
        .iter()
        .find(|node| node.handle == "planner")
        .expect("planner node should exist");
    assert_eq!(planner.can_complete_workflow_run, Some(true));
    assert!(
        planner
            .instructions
            .as_deref()
            .unwrap_or_default()
            .contains("only node allowed to finish")
    );
    for node in compiled
        .nodes
        .iter()
        .filter(|node| node.handle == "worker" || node.handle == "reviewer")
    {
        assert_ne!(
            node.can_complete_workflow_run,
            Some(true),
            "{} must not complete the workflow",
            node.handle
        );
    }

    let edge_pairs = compiled
        .edges
        .iter()
        .map(|edge| {
            (
                edge.handle.as_str(),
                edge.from_node.as_str(),
                edge.to_node.as_str(),
                edge.handoff_schema.as_deref(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert!(edge_pairs.contains(&(
        "planner_to_worker",
        "planner",
        "worker",
        Some("implementation_assignment"),
    )));
    assert!(edge_pairs.contains(&(
        "worker_to_reviewer",
        "worker",
        "reviewer",
        Some("implementation_result"),
    )));
    assert!(edge_pairs.contains(&(
        "reviewer_to_worker",
        "reviewer",
        "worker",
        Some("revision_request"),
    )));
    assert!(edge_pairs.contains(&(
        "reviewer_to_planner",
        "reviewer",
        "planner",
        Some("accepted_step_report"),
    )));
    assert_eq!(compiled.endpoints[0].handle, "entry");
    assert_eq!(compiled.endpoints[0].entry_node, "planner");
    assert!(
        compiled
            .parameters_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/max_review_cycles_per_step/default"))
            .is_some_and(|value| value == 6)
    );
}
