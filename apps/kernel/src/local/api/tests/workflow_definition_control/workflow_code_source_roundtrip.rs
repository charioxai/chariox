use super::*;

#[test]
fn source_rebuild_and_update_round_trip_the_same_workflow() {
    let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
        eprintln!("skipping workflow-code source round-trip because node is not available");
        return;
    };
    let workspace = std::env::temp_dir().join(format!(
        "chariox-workflow-source-roundtrip-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&workspace).expect("workspace should create");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace.display().to_string(),
                workspace.display().to_string(),
            ),
        ))
        .expect("session should create")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected session response"),
    };
    let source = r#"
workflow.define({ alias: "authored_review", prompt: "Review the change." })
const reviewer = workflow.node({
  handle: "reviewer",
  agent: workflow.newAgent({ alias: "reviewer", provider: "dev-stub" }),
  instructions: "Review carefully."
})
workflow.endpoint(reviewer, { handle: "entry", alias: "entry" })
"#;
    let apply = harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowCode(
            crate::local::ApplyWorkflowCodeRequest {
                session_id: session.id().to_string(),
                node_path: node_path.display().to_string(),
                source: source.to_string(),
                language: Some(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                provider_rebindings: Vec::new(),
                agent_rebindings: Vec::new(),
            },
        ))
        .expect("source workflow should apply");
    let (report, applied_session) = match apply {
        LocalDaemonResponse::WorkflowCodeApplied { result, session } => (result.apply, session),
        _ => panic!("unexpected apply response"),
    };
    let workflow_id = report.workflow_id.clone();
    let artifact_name = format!("roundtrip-{}", crate::session::unix_epoch_ms());
    harness
        .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
            crate::local::CreateWorkflowCodeArtifactRequest {
                session_id: session.id().to_string(),
                name: artifact_name.clone(),
                language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                node_path: node_path.display().to_string(),
                source: source.to_string(),
            },
        ))
        .expect("source artifact should create");
    let applied_revision = applied_session
        .workflow(&workflow_id)
        .expect("workflow should exist")
        .revision();
    let bound_session = match harness
        .dispatch(LocalDaemonRequest::BindWorkflowCodeSource(
            crate::local::BindWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                artifact_name,
                origin: crate::session::WorkflowCodeSourceOrigin::Authored,
                expected_workflow_revision: Some(applied_revision),
            },
        ))
        .expect("source should bind")
    {
        LocalDaemonResponse::WorkflowCodeSourceBound { session, .. } => session,
        _ => panic!("unexpected bind response"),
    };
    let bound_revision = bound_session
        .workflow(&workflow_id)
        .expect("bound workflow should exist")
        .revision();
    let changed_session = apply_alias_change(
        &harness,
        session.id(),
        &workflow_id,
        "visually_changed",
        "source-roundtrip-change-1",
    );
    let changed_revision = changed_session
        .workflow(&workflow_id)
        .expect("changed workflow should exist")
        .revision();
    assert!(changed_revision > bound_revision);

    let preview = match harness
        .dispatch(LocalDaemonRequest::RebuildWorkflowCodeSource(
            crate::local::RebuildWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                expected_workflow_revision: changed_revision,
                confirm: false,
            },
        ))
        .expect("rebuild preview should succeed")
    {
        LocalDaemonResponse::WorkflowCodeRebuildPreview { preview } => preview,
        _ => panic!("unexpected rebuild preview response"),
    };
    assert!(preview.diverged);
    let rebuilt_session = match harness
        .dispatch(LocalDaemonRequest::RebuildWorkflowCodeSource(
            crate::local::RebuildWorkflowCodeSourceRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                expected_workflow_revision: changed_revision,
                confirm: true,
            },
        ))
        .expect("confirmed rebuild should succeed")
    {
        LocalDaemonResponse::WorkflowCodeSourceRebuilt { session, .. } => session,
        _ => panic!("unexpected rebuild response"),
    };
    let rebuilt = rebuilt_session
        .workflow(&workflow_id)
        .expect("same workflow should remain");
    assert_eq!(rebuilt.alias(), Some("authored_review"));
    assert_eq!(rebuilt.id(), workflow_id);

    let changed_session = apply_alias_change(
        &harness,
        session.id(),
        &workflow_id,
        "visual_source",
        "source-roundtrip-change-2",
    );
    let changed_revision = changed_session
        .workflow(&workflow_id)
        .expect("changed workflow should exist")
        .revision();
    let update_preview = match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(
            crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                expected_workflow_revision: changed_revision,
                expected_generated_source_sha256: None,
                confirm: false,
            },
        ))
        .expect("source update preview should succeed")
    {
        LocalDaemonResponse::WorkflowCodeSourceUpdatePreview { preview } => preview,
        _ => panic!("unexpected source update preview response"),
    };
    assert!(update_preview.changed);
    assert!(update_preview.generated_source.contains("visual_source"));
    let rejected = harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(
            crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                expected_workflow_revision: changed_revision,
                expected_generated_source_sha256: Some("stale-preview".to_string()),
                confirm: true,
            },
        ))
        .expect_err("stale source preview should be rejected");
    assert!(rejected
        .to_string()
        .contains("generated workflow source changed after preview"));
    let after_rejection = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session should still load after rejected update");
    let after_rejection = match after_rejection {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected session response"),
    };
    assert_eq!(
        after_rejection
            .workflow(&workflow_id)
            .expect("workflow should remain after rejected update")
            .revision(),
        changed_revision
    );
    let updated_session = match harness
        .dispatch(LocalDaemonRequest::UpdateWorkflowCodeSourceFromWorkflow(
            crate::local::UpdateWorkflowCodeSourceFromWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                expected_workflow_revision: changed_revision,
                expected_generated_source_sha256: Some(
                    update_preview.generated_source_sha256.clone(),
                ),
                confirm: true,
            },
        ))
        .expect("source update should succeed")
    {
        LocalDaemonResponse::WorkflowCodeSourceUpdated { session, .. } => session,
        _ => panic!("unexpected source update response"),
    };
    let updated = updated_session
        .workflow(&workflow_id)
        .expect("updated workflow should remain");
    assert_eq!(updated.alias(), Some("visual_source"));
    assert_eq!(
        updated
            .code_source()
            .expect("source binding should remain")
            .origin(),
        crate::session::WorkflowCodeSourceOrigin::Generated
    );
    std::fs::remove_dir_all(workspace).expect("workspace should clean up");
}

fn apply_alias_change(
    harness: &LocalRouterTestHarness,
    session_id: &str,
    workflow_id: &str,
    alias: &str,
    op_id: &str,
) -> crate::session::RuntimeSession {
    match harness
        .dispatch(LocalDaemonRequest::ApplyWorkflowDesignOp(
            crate::local::ApplyWorkflowDesignOpRequest {
                session_id: session_id.to_string(),
                origin_client_id: "source-roundtrip-test".to_string(),
                op_id: op_id.to_string(),
                op: crate::local::WorkflowDesignOp::WorkflowUpdate {
                    workflow_id: workflow_id.to_string(),
                    patch: crate::local::WorkflowDesignWorkflowPatch {
                        alias: Some(Some(alias.to_string())),
                        prompt: None,
                        flush_agent_context_before_run: None,
                        max_concurrent: None,
                        run_output_schema_ref: None,
                    },
                },
            },
        ))
        .expect("workflow alias should update")
    {
        LocalDaemonResponse::WorkflowDesignOpAccepted { session, .. } => session,
        _ => panic!("unexpected design response"),
    }
}
