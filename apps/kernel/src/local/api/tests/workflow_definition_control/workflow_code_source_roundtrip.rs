use super::*;

struct WorkflowCodeRebuildActivityFixture {
    harness: LocalRouterTestHarness,
    _workspace: crate::test_support::TestWorktree,
    session_id: String,
    workflow_id: String,
    queue_id: String,
}

impl WorkflowCodeRebuildActivityFixture {
    fn new(source_queue_enabled: bool, current_queue_enabled: bool, label: &str) -> Option<Self> {
        let Some(node_path) = find_node_for_workflow_code_local_api_test() else {
            eprintln!("skipping workflow-code rebuild activity test because node is not available");
            return None;
        };
        let workspace = crate::test_support::TestWorktree::new(label);
        let harness = LocalRouterTestHarness::new();
        harness
            .runtime_state()
            .ensure_managed_activity_tracking(&format!("kernel-{label}"))
            .expect("managed activity tracking should activate before fixture mutations");
        let session = match harness
            .dispatch(LocalDaemonRequest::CreateSession(
                workspace.session_request(),
            ))
            .expect("activity fixture session should create")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            other => panic!("unexpected session response: {other:?}"),
        };
        let source = format!(
            r#"
workflow.define({{ alias: "rebuild_activity" }})
const worker = workflow.node({{
  handle: "worker",
  agent: workflow.newAgent({{ alias: "rebuild-worker", provider: "dev-stub" }}),
  instructions: "Hold queued work."
}})
workflow.queue({{
  handle: "pending",
  alias: "default",
  priority: 0,
  enabled: {source_queue_enabled}
}})
workflow.endpoint(worker, {{ handle: "entry", alias: "entry" }})
"#
        );
        let applied = harness
            .dispatch(LocalDaemonRequest::ApplyWorkflowCode(
                crate::local::ApplyWorkflowCodeRequest {
                    session_id: session.id().to_string(),
                    node_path: node_path.display().to_string(),
                    source: source.clone(),
                    language: Some(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                    provider_rebindings: Vec::new(),
                    agent_rebindings: Vec::new(),
                },
            ))
            .expect("activity fixture workflow code should apply");
        let report = match applied {
            LocalDaemonResponse::WorkflowCodeApplied { result, .. } => result.apply,
            other => panic!("unexpected workflow-code apply response: {other:?}"),
        };
        let artifact_name = format!("rebuild-activity-{label}");
        harness
            .dispatch(LocalDaemonRequest::CreateWorkflowCodeArtifact(
                crate::local::CreateWorkflowCodeArtifactRequest {
                    session_id: session.id().to_string(),
                    name: artifact_name.clone(),
                    language: crate::workflow_code::WorkflowCodeLanguage::JavaScript,
                    node_path: node_path.display().to_string(),
                    source,
                },
            ))
            .expect("activity fixture artifact should create");
        let workflow_revision = harness.with_app(|app| {
            app.sessions()
                .resolve_workflow_ref(session.id(), &report.workflow_id)
                .expect("activity fixture workflow should resolve")
                .revision()
        });
        harness
            .dispatch(LocalDaemonRequest::BindWorkflowCodeSource(
                crate::local::BindWorkflowCodeSourceRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: report.workflow_id.clone(),
                    artifact_name,
                    origin: crate::session::WorkflowCodeSourceOrigin::Authored,
                    expected_workflow_revision: Some(workflow_revision),
                },
            ))
            .expect("activity fixture source should bind");
        harness
            .dispatch(LocalDaemonRequest::UpdateWorkflowPromptQueue(
                UpdateWorkflowPromptQueueRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: Some(report.workflow_id.clone()),
                    queue_ref: report.queue_ids["pending"].clone(),
                    alias: None,
                    priority: None,
                    enabled: Some(false),
                },
            ))
            .expect("activity fixture queue should disable before enqueue");
        harness
            .dispatch(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: report.workflow_id.clone(),
                    endpoint_ref: report.endpoint_ids["entry"].clone(),
                    prompt: Some("preserve this queued prompt".to_string()),
                    queue_ref: Some(report.queue_ids["pending"].clone()),
                    publication_invocation: None,
                },
            ))
            .expect("activity fixture prompt should enqueue while disabled");
        if current_queue_enabled {
            harness
                .dispatch(LocalDaemonRequest::UpdateWorkflowPromptQueue(
                    UpdateWorkflowPromptQueueRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: Some(report.workflow_id.clone()),
                        queue_ref: report.queue_ids["pending"].clone(),
                        alias: None,
                        priority: None,
                        enabled: Some(true),
                    },
                ))
                .expect("activity fixture queue should enable after enqueue");
        }
        Some(Self {
            harness,
            _workspace: workspace,
            session_id: session.id().to_string(),
            workflow_id: report.workflow_id.clone(),
            queue_id: report.queue_ids["pending"].clone(),
        })
    }

    fn revision(&self) -> u64 {
        self.harness.with_app(|app| {
            app.sessions()
                .resolve_workflow_ref(&self.session_id, &self.workflow_id)
                .expect("activity fixture workflow should resolve")
                .revision()
        })
    }

    fn rebuild(&self, confirm: bool, expected_workflow_revision: u64) -> LocalDaemonResponse {
        self.harness
            .dispatch(LocalDaemonRequest::RebuildWorkflowCodeSource(
                crate::local::RebuildWorkflowCodeSourceRequest {
                    session_id: self.session_id.clone(),
                    workflow_ref: self.workflow_id.clone(),
                    expected_workflow_revision,
                    confirm,
                },
            ))
            .expect("workflow-code source rebuild should succeed")
    }

    fn session(&self) -> crate::session::RuntimeSession {
        self.harness.with_app(|app| {
            app.sessions()
                .get_session(&self.session_id)
                .expect("activity fixture session should resolve")
        })
    }
}

#[test]
fn source_rebuild_enables_nonempty_queue_under_activity_boundary() {
    let Some(fixture) = WorkflowCodeRebuildActivityFixture::new(true, false, "rebuild-enable")
    else {
        return;
    };
    let (_, before) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("idle activity should be readable before enabling rebuild");
    assert_eq!(before.running_agent_count, 0);

    let response = fixture.rebuild(true, fixture.revision());
    assert!(matches!(
        response,
        LocalDaemonResponse::WorkflowCodeSourceRebuilt { .. }
    ));
    let session = fixture.session();
    assert!(session
        .workflow_prompt_queue(&fixture.workflow_id, &fixture.queue_id)
        .expect("rebuilt queue should remain")
        .enabled());
    assert_eq!(session.workflow_queued_prompts().len(), 1);
    let (_, after) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("busy activity should be readable after enabling rebuild");
    assert_eq!(after.running_agent_count, 1);
}

#[test]
fn source_rebuild_disables_nonempty_queue_under_activity_boundary() {
    let Some(fixture) = WorkflowCodeRebuildActivityFixture::new(false, true, "rebuild-disable")
    else {
        return;
    };
    let (_, before) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("busy activity should be readable before disabling rebuild");
    assert_eq!(before.running_agent_count, 1);

    fixture.rebuild(true, fixture.revision());
    let session = fixture.session();
    assert!(!session
        .workflow_prompt_queue(&fixture.workflow_id, &fixture.queue_id)
        .expect("rebuilt queue should remain")
        .enabled());
    assert_eq!(session.workflow_queued_prompts().len(), 1);
    let (_, after) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("idle activity should be readable after disabling rebuild");
    assert_eq!(after.running_agent_count, 0);
}

#[test]
fn source_rebuild_preview_and_activity_noop_preserve_idle_epoch() {
    let Some(fixture) = WorkflowCodeRebuildActivityFixture::new(false, false, "rebuild-noop")
    else {
        return;
    };
    let (_, before) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("idle activity should be readable before no-op rebuild");
    assert_eq!(before.running_agent_count, 0);
    let revision = fixture.revision();

    assert!(matches!(
        fixture.rebuild(false, revision),
        LocalDaemonResponse::WorkflowCodeRebuildPreview { .. }
    ));
    let (_, after_preview) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("preview activity should remain readable");
    assert_eq!(after_preview, before);

    fixture.rebuild(true, revision);
    let (_, after_noop) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("no-op rebuild activity should remain readable");
    assert_eq!(after_noop, before);
    assert_eq!(fixture.session().workflow_queued_prompts().len(), 1);
}

#[test]
fn failed_source_rebuild_persistence_rolls_back_and_retries() {
    let Some(fixture) =
        WorkflowCodeRebuildActivityFixture::new(true, false, "rebuild-append-failure")
    else {
        return;
    };
    let revision = fixture.revision();
    let (_, activity_before) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("idle activity should be readable before failed rebuild");
    let state_path = fixture
        .harness
        .with_app(|app| app.durable_state_store().path().to_path_buf());
    let connection = rusqlite::Connection::open(state_path)
        .expect("durable database should open for rebuild failure injection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_workflow_code_rebuild_append
             BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'workflow.runtime.updated'
             BEGIN
               SELECT RAISE(FAIL, 'injected workflow-code rebuild append failure');
             END;",
        )
        .expect("workflow-code rebuild failure trigger should install");

    let error = fixture
        .harness
        .dispatch(LocalDaemonRequest::RebuildWorkflowCodeSource(
            crate::local::RebuildWorkflowCodeSourceRequest {
                session_id: fixture.session_id.clone(),
                workflow_ref: fixture.workflow_id.clone(),
                expected_workflow_revision: revision,
                confirm: true,
            },
        ))
        .expect_err("failed durable rebuild append should reject the request");
    assert!(error
        .to_string()
        .contains("injected workflow-code rebuild append failure"));
    let rolled_back = fixture.session();
    assert_eq!(
        rolled_back
            .workflow(&fixture.workflow_id)
            .expect("rolled-back workflow should remain")
            .revision(),
        revision
    );
    assert!(!rolled_back
        .workflow_prompt_queue(&fixture.workflow_id, &fixture.queue_id)
        .expect("rolled-back queue should remain")
        .enabled());
    assert_eq!(rolled_back.workflow_queued_prompts().len(), 1);
    let (_, activity_after_failure) = fixture
        .harness
        .runtime_state()
        .managed_activity_report_snapshot()
        .expect("rolled-back activity should remain readable");
    assert_eq!(activity_after_failure, activity_before);

    connection
        .execute_batch("DROP TRIGGER fail_workflow_code_rebuild_append;")
        .expect("workflow-code rebuild failure trigger should be removed");
    fixture.rebuild(true, revision);
    let retried = fixture.session();
    assert!(retried
        .workflow_prompt_queue(&fixture.workflow_id, &fixture.queue_id)
        .expect("retried queue should remain")
        .enabled());
    assert_eq!(retried.workflow_queued_prompts().len(), 1);
}

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
