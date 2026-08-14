use super::*;

#[test]
fn rebuilds_the_same_workflow_from_bound_source_mappings() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);
    let definition = workflow_code_definition();
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &BTreeMap::from([
                ("planner".to_string(), "agent-1".to_string()),
                ("worker".to_string(), "agent-2".to_string()),
            ]),
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow code should apply");
    let workflow_id = report.workflow_id.clone();
    let applied_revision = service
        .resolve_workflow_ref(session.id(), &workflow_id)
        .expect("workflow should resolve")
        .revision();
    service
        .bind_workflow_code_source(
            session.id(),
            &workflow_id,
            Some(applied_revision),
            crate::session::WorkflowCodeSourceDescriptor {
                artifact_name: "source-artifact".to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                source_sha256: "source-sha".to_string(),
                origin: crate::session::WorkflowCodeSourceOrigin::Authored,
            },
            report.clone(),
        )
        .expect("source should bind");
    let changed = service
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::WorkflowUpdate {
                workflow_id: workflow_id.clone(),
                patch: crate::local::WorkflowDesignWorkflowPatch {
                    alias: Some(Some("visually_changed".to_string())),
                    prompt: Some(Some("Visual prompt".to_string())),
                    flush_agent_context_before_run: None,
                    max_concurrent: None,
                    run_output_schema_ref: None,
                },
            },
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("visual edit should apply");

    let rebuilt = service
        .rebuild_workflow_code_definition(
            session.id(),
            &workflow_id,
            changed.revision(),
            &definition,
            crate::session::WorkflowCodeSourceDescriptor {
                artifact_name: "source-artifact".to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                source_sha256: "source-sha".to_string(),
                origin: crate::session::WorkflowCodeSourceOrigin::Authored,
            },
        )
        .expect("bound source should rebuild in place");

    assert_eq!(rebuilt.workflow_id, workflow_id);
    assert_eq!(rebuilt.node_ids, report.node_ids);
    assert_eq!(rebuilt.endpoint_ids, report.endpoint_ids);
    let workflow = service
        .resolve_workflow_ref(session.id(), &workflow_id)
        .expect("same workflow should remain");
    assert_eq!(workflow.alias(), Some("coded_flow"));
    assert_eq!(workflow.prompt(), Some("Run the coded flow."));
    assert!(workflow.revision() > changed.revision());
    assert_eq!(
        workflow
            .code_source()
            .expect("source should remain bound")
            .workflow_revision(),
        workflow.revision()
    );
}

#[test]
fn rebuild_revision_conflict_does_not_mutate_the_workflow() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);
    let definition = workflow_code_definition();
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &BTreeMap::from([
                ("planner".to_string(), "agent-1".to_string()),
                ("worker".to_string(), "agent-2".to_string()),
            ]),
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow code should apply");
    let revision = service
        .resolve_workflow_ref(session.id(), &report.workflow_id)
        .expect("workflow should resolve")
        .revision();
    service
        .bind_workflow_code_source(
            session.id(),
            &report.workflow_id,
            Some(revision),
            crate::session::WorkflowCodeSourceDescriptor {
                artifact_name: "source-artifact".to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                source_sha256: "source-sha".to_string(),
                origin: crate::session::WorkflowCodeSourceOrigin::Generated,
            },
            report.clone(),
        )
        .expect("source should bind");
    let before = service
        .resolve_workflow_ref(session.id(), &report.workflow_id)
        .expect("workflow should resolve before conflict");
    let error = service
        .rebuild_workflow_code_definition(
            session.id(),
            &report.workflow_id,
            before.revision() - 1,
            &definition,
            crate::session::WorkflowCodeSourceDescriptor {
                artifact_name: "source-artifact".to_string(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                source_sha256: "source-sha".to_string(),
                origin: crate::session::WorkflowCodeSourceOrigin::Generated,
            },
        )
        .expect_err("stale rebuild should fail");
    assert!(error.to_string().contains("revision conflict"));
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), &report.workflow_id)
            .expect("workflow should remain")
            .revision(),
        before.revision()
    );
}
