use super::SessionService;
use crate::agent::{AgentInstance, GridPosition};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::session::{
    unix_epoch_ms, CreateSessionRequest, PromptSubmissionOutcome, QueuedWorkflowLaunchSource,
    SchedulerState, SessionAgentDefaults, SessionStatus, WorkflowCompletionSnapshot,
    WorkflowHandoffPayload, WorkflowLaunchAdmission, WorkflowLaunchPolicy, WorkflowNodeRunStatus,
    WorkflowRunStatus, WorkflowWatchdogPolicy, WorktreeIsolationMode, DEFAULT_LOCAL_USER_ID,
};
use std::collections::BTreeMap;

fn test_config() -> DaemonConfig {
    DaemonConfig::for_tests()
}

#[test]
fn creates_gets_and_lists_sessions() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    assert_eq!(created.id().len(), 16);
    assert!(created.id().chars().all(|char| char.is_ascii_hexdigit()));
    assert_eq!(created.alias(), None);
    assert_eq!(created.workspace_id(), "workspace-1");
    assert_eq!(created.worktree_id(), "worktree-1");
    assert_eq!(created.host_machine_id(), "machine-test");
    assert_eq!(created.host_daemon_id(), "daemon-test");
    assert_eq!(created.status(), SessionStatus::Created);
    assert!(created.active_provider_run_id().is_none());
    assert!(created.attachment_ids().is_empty());
    assert!(created.active_prompt().is_none());
    assert!(created.queued_prompts().is_empty());
    assert_eq!(created.scheduler_state(), SchedulerState::Idle);
    assert_eq!(created.config_state().version(), 0);
    assert_eq!(created.worktree_assignments().len(), 1);
    assert_eq!(
        created.worktree_assignments()[0].isolation_mode(),
        WorktreeIsolationMode::SharedSession
    );
    assert_eq!(service.active_session_count(), 1);

    let fetched = service
        .get_session(created.id())
        .expect("lookup should succeed");
    assert_eq!(fetched, created);
    assert_eq!(service.list_sessions(), vec![created]);
}

#[test]
fn create_session_stores_agent_defaults() {
    let mut service = SessionService::new(&test_config());
    let defaults = SessionAgentDefaults::new("opencode")
        .with_model("moonshotai/kimi-k2")
        .with_effort("high")
        .with_account_profile("default")
        .with_execution_mode(AgentExecutionMode::Plan)
        .with_permission_level(AgentPermissionLevel::Required);

    let created = service
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_agent_defaults(defaults.clone()),
        )
        .expect("session should be created");

    assert_eq!(created.agent_defaults(), &defaults);
    assert_eq!(
        service
            .get_session(created.id())
            .expect("session should be persisted")
            .agent_defaults(),
        &defaults
    );
}

#[test]
fn manages_session_membership_invites() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    assert_eq!(session.owner_user_id(), DEFAULT_LOCAL_USER_ID);
    assert_eq!(session.members().len(), 1);
    assert!(session.has_member(DEFAULT_LOCAL_USER_ID));

    let (session, invite) = service
        .create_session_invite(
            session.id(),
            "invite-1".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
        )
        .expect("member should create invite");
    assert_eq!(session.invites().len(), 1);
    assert_eq!(invite.used_count(), 0);

    let (session, member) = service
        .join_session_invite(
            session.id(),
            invite.invite_id(),
            "user-2".to_string(),
            unix_epoch_ms(),
        )
        .expect("invite should be joinable");
    assert_eq!(member.user_id(), "user-2");
    assert_eq!(member.invited_by_user_id(), Some(DEFAULT_LOCAL_USER_ID));
    assert!(session.has_member("user-2"));
    assert_eq!(session.invites()[0].used_count(), 1);

    let exhausted = service
        .join_session_invite(
            session.id(),
            invite.invite_id(),
            "user-3".to_string(),
            unix_epoch_ms(),
        )
        .expect_err("single-use invite should be exhausted");
    assert!(exhausted.to_string().contains("no uses remaining"));

    let (session, revoked) = service
        .revoke_session_invite(session.id(), invite.invite_id())
        .expect("invite should revoke");
    assert!(revoked.is_revoked());
    assert!(session
        .invite(invite.invite_id())
        .is_some_and(|invite| invite.is_revoked()));
}

#[test]
fn normalizes_aliases_and_resolves_ids_and_aliases() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1").with_alias(" Feature_Main "),
        )
        .expect("session should be created");

    assert_eq!(created.alias(), Some("feature_main"));
    assert_eq!(
        service
            .resolve_session_ref(created.id(), Some("workspace-1"))
            .expect("full id should resolve")
            .id(),
        created.id()
    );
    assert_eq!(
        service
            .resolve_session_ref(&created.id()[..8], Some("workspace-1"))
            .expect("id prefix should resolve")
            .id(),
        created.id()
    );
    assert_eq!(
        service
            .resolve_session_ref("feature_main", Some("workspace-1"))
            .expect("alias should resolve")
            .id(),
        created.id()
    );
    assert_eq!(
        service
            .resolve_session_ref("feature", Some("workspace-1"))
            .expect("alias prefix should resolve")
            .id(),
        created.id()
    );
}

#[test]
fn rejects_duplicate_alias_in_same_workspace() {
    let mut service = SessionService::new(&test_config());
    service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"))
        .expect("first session should be created");

    let error = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-2").with_alias("MAIN"))
        .expect_err("duplicate alias should be rejected");

    match error {
        DaemonError::SessionAliasConflict {
            workspace_id,
            alias,
        } => {
            assert_eq!(workspace_id, "workspace-1");
            assert_eq!(alias, "main");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn can_assign_alias_to_existing_session() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let updated = service
        .assign_session_alias(session.id(), "dev_env".to_string())
        .expect("alias should be assigned");

    assert_eq!(updated.alias(), Some("dev_env"));
    assert_eq!(
        service
            .resolve_session_ref("dev_env", Some("workspace-1"))
            .expect("alias should resolve")
            .id(),
        session.id()
    );
}

#[test]
fn rejects_duplicate_session_alias_on_assignment() {
    let mut service = SessionService::new(&test_config());
    service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"))
        .expect("first session should be created");
    let second = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-2"))
        .expect("second session should be created");

    let error = service
        .assign_session_alias(second.id(), "MAIN".to_string())
        .expect_err("duplicate alias should be rejected");

    match error {
        DaemonError::SessionAliasConflict {
            workspace_id,
            alias,
        } => {
            assert_eq!(workspace_id, "workspace-1");
            assert_eq!(alias, "main");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn normalizes_aliases_when_assigned() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let updated = service
        .assign_session_alias(session.id(), " Feature Main ".to_string())
        .expect("alias should be assigned");

    assert_eq!(updated.alias(), Some("feature_main"));
}

#[test]
fn creates_lists_and_resolves_workflows_by_id_and_alias_prefix() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let first = service
        .create_workflow(session.id(), Some("review_loop".to_string()))
        .expect("workflow should be created");
    let second = service
        .create_workflow(session.id(), Some("deploy".to_string()))
        .expect("workflow should be created");

    let workflows = service
        .list_workflows(session.id())
        .expect("workflow list should succeed");
    assert_eq!(workflows.len(), 2);
    assert_eq!(workflows[0], first);
    assert_eq!(workflows[1], second);

    let unique_prefix_len = (1..=first.id().len())
        .find(|length| {
            let prefix = &first.id()[..*length];
            workflows
                .iter()
                .filter(|workflow| workflow.id().starts_with(prefix))
                .count()
                == 1
        })
        .expect("workflow id should have a unique prefix");
    let unique_prefix = &first.id()[..unique_prefix_len];

    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), first.id())
            .expect("workflow id should resolve")
            .id(),
        first.id()
    );
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), unique_prefix)
            .expect("workflow id prefix should resolve")
            .id(),
        first.id()
    );
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), "review_loop")
            .expect("workflow alias should resolve")
            .id(),
        first.id()
    );
    assert_eq!(
        service
            .resolve_workflow_ref(session.id(), "review")
            .expect("workflow alias prefix should resolve")
            .id(),
        first.id()
    );
    assert!(first.flush_agent_context_before_run());
}

#[test]
fn creates_lists_resolves_and_disables_workflow_publications() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("main".to_string()),
        )
        .expect("workflow endpoint should be created");

    let publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("public_review".to_string()),
            Some("/review".to_string()),
            vec!["POST".to_string()],
            Some(serde_json::json!({"kind": "http"})),
            Some(serde_json::json!({"mode": "anonymous"})),
            Some(serde_json::json!({"kind": "webhook"})),
            None,
            Some("async".to_string()),
            "local".to_string(),
        )
        .expect("workflow publication should be created");

    assert_eq!(publication.workflow_id(), workflow.id());
    assert_eq!(publication.endpoint_id(), endpoint.id());
    assert_eq!(publication.alias(), Some("public_review"));
    assert!(publication.enabled());

    let publications = service
        .list_workflow_publications(session.id())
        .expect("publication list should succeed");
    assert_eq!(publications, vec![publication.clone()]);
    assert_eq!(
        service
            .resolve_workflow_publication_ref(session.id(), "public")
            .expect("publication alias prefix should resolve")
            .id(),
        publication.id()
    );

    let disabled = service
        .disable_workflow_publication(session.id(), publication.id())
        .expect("publication should be disabled");
    assert!(!disabled.enabled());
}

#[test]
fn workflow_publication_pairing_codes_manage_trusted_senders() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
        .expect("workflow endpoint should be created");
    let publication = service
        .create_workflow_publication(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("public_review".to_string()),
            None,
            vec![],
            None,
            Some(serde_json::json!({"mode": "arroba", "paired_senders": {"enabled": true}})),
            None,
            None,
            None,
            "local".to_string(),
        )
        .expect("workflow publication should be created");

    let pair_code = service
        .create_workflow_publication_pairing_code(
            session.id(),
            publication.id(),
            "pair-code-hash",
            "local".to_string(),
            None,
            Some(1),
        )
        .expect("pairing code should be created");
    let sender_credential = service
        .redeem_workflow_publication_pairing_code(
            session.id(),
            publication.id(),
            pair_code.code_id(),
            "pair-code-hash",
            "sender-secret",
            Some("Marketing".to_string()),
            vec!["http".to_string()],
            None,
            100,
        )
        .expect("pairing code should redeem");
    assert_eq!(sender_credential.credential, "sender-secret");
    assert_eq!(sender_credential.sender.display_name(), Some("Marketing"));
    assert!(service
        .redeem_workflow_publication_pairing_code(
            session.id(),
            publication.id(),
            pair_code.code_id(),
            "pair-code-hash",
            "second-secret",
            None,
            vec!["http".to_string()],
            None,
            101,
        )
        .is_err());

    let sender = service
        .authenticate_workflow_publication_sender(
            session.id(),
            publication.id(),
            "sender-secret",
            "http",
            102,
        )
        .expect("sender should authenticate");
    assert_eq!(sender.sender_id(), sender_credential.sender.sender_id());
    assert!(service
        .authenticate_workflow_publication_sender(
            session.id(),
            publication.id(),
            "sender-secret",
            "slack",
            103,
        )
        .is_err());

    let senders = service
        .list_workflow_publication_senders(session.id(), publication.id())
        .expect("senders should list");
    assert_eq!(senders.len(), 1);
    let revoked = service
        .revoke_workflow_publication_sender(
            session.id(),
            publication.id(),
            sender_credential.sender.sender_id(),
            104,
        )
        .expect("sender should revoke");
    assert!(revoked.is_revoked());
    assert!(service
        .authenticate_workflow_publication_sender(
            session.id(),
            publication.id(),
            "sender-secret",
            "http",
            105,
        )
        .is_err());
}

#[test]
fn workflow_flush_context_defaults_true_and_can_be_updated() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    assert!(workflow.flush_agent_context_before_run());

    let updated = service
        .set_workflow_flush_agent_context_before_run(session.id(), workflow.id(), false)
        .expect("workflow flush setting should update");
    assert!(!updated.flush_agent_context_before_run());
}

#[test]
fn workflow_run_output_and_node_completion_settings_can_be_updated() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");

    let updated_workflow = service
        .set_workflow_run_output_schema_ref(
            session.id(),
            workflow.id(),
            Some("/tmp/workflow-run-output-schema.json".to_string()),
        )
        .expect("workflow run output schema should update");
    assert_eq!(
        updated_workflow.run_output_schema_ref(),
        Some("/tmp/workflow-run-output-schema.json")
    );
    let updated_workflow = service
        .set_workflow_intermediate_output_schema_ref(
            session.id(),
            workflow.id(),
            Some("/tmp/workflow-intermediate-output-schema.json".to_string()),
        )
        .expect("workflow intermediate output schema should update");
    assert_eq!(
        updated_workflow.intermediate_output_schema_ref(),
        Some("/tmp/workflow-intermediate-output-schema.json")
    );

    let updated_node = service
        .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
        .expect("node completion setting should update");
    assert!(updated_node.can_complete_workflow_run());
    let updated_node = service
        .set_workflow_node_can_emit_intermediate_output(
            session.id(),
            workflow.id(),
            node.id(),
            true,
        )
        .expect("node intermediate output capability should update");
    assert!(updated_node.can_emit_intermediate_run_output());
    let updated_node = service
        .set_workflow_node_intermediate_output_schema_ref(
            session.id(),
            workflow.id(),
            node.id(),
            Some("/tmp/node-intermediate-output-schema.json".to_string()),
        )
        .expect("node intermediate output schema should update");
    assert_eq!(
        updated_node.intermediate_output_schema_ref(),
        Some("/tmp/node-intermediate-output-schema.json")
    );

    let updated_node = service
        .set_workflow_node_max_turns(session.id(), workflow.id(), node.id(), Some(3))
        .expect("node max turns should update");
    assert_eq!(updated_node.max_turns(), Some(3));
}

#[test]
fn manages_workflow_nodes_edges_and_endpoints() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let planner = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("planner node should be added");
    let duplicate_node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect_err("duplicate workflow node should be rejected");
    assert!(matches!(
        duplicate_node,
        DaemonError::WorkflowNodeConflict { .. }
    ));
    let reviewer = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("reviewer node should be added");

    let edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            reviewer.id(),
            None,
            None,
        )
        .expect("edge should be added");
    assert_eq!(edge.from_node_id(), planner.id());
    assert_eq!(edge.to_node_id(), reviewer.id());

    let duplicate_edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            reviewer.id(),
            None,
            None,
        )
        .expect_err("duplicate edge should be rejected");
    assert!(matches!(
        duplicate_edge,
        DaemonError::WorkflowEdgeConflict { .. }
    ));

    let self_edge = service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            planner.id(),
            planner.id(),
            None,
            None,
        )
        .expect_err("self edge should be rejected");
    assert!(matches!(
        self_edge,
        DaemonError::InvalidWorkflowGraphReference { .. }
    ));

    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            planner.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    assert_eq!(endpoint.entry_node_id(), planner.id());

    assert_eq!(
        service
            .resolve_workflow_endpoint_ref(session.id(), workflow.id(), "entry")
            .expect("endpoint alias should resolve")
            .id(),
        endpoint.id()
    );

    let rebound = service
        .bind_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), reviewer.id())
        .expect("endpoint should be rebound");
    assert_eq!(rebound.entry_node_id(), reviewer.id());

    let aliased = service
        .assign_workflow_endpoint_alias(
            session.id(),
            workflow.id(),
            endpoint.id(),
            "review-entry".to_string(),
        )
        .expect("endpoint alias should be updated");
    assert_eq!(aliased.alias(), Some("review-entry"));

    let removed_edge = service
        .remove_workflow_edge(session.id(), workflow.id(), edge.id())
        .expect("edge should be removed");
    assert_eq!(removed_edge.id(), edge.id());

    let removed_node = service
        .remove_workflow_node(session.id(), workflow.id(), planner.id())
        .expect("node should be removed");
    assert_eq!(removed_node.id(), planner.id());
}

#[test]
fn creates_lists_resolves_and_cancels_workflow_runs() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    assert_eq!(workflow_run.workflow_id(), workflow.id());
    assert_eq!(workflow_run.endpoint_id(), endpoint.id());
    assert_eq!(workflow_run.entry_node_id(), node.id());
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert_eq!(
        workflow_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Ready
    );
    assert_eq!(workflow_run.messages().len(), 1);
    assert_eq!(workflow_run.messages()[0].target_node_id(), node.id());

    let listed = service
        .list_workflow_runs(session.id(), Some(workflow.id()))
        .expect("workflow runs should list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id(), workflow_run.id());

    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.id(), workflow_run.id());

    let cancelled = service
        .cancel_workflow_run(session.id(), workflow_run.id())
        .expect("workflow run should cancel");
    assert_eq!(cancelled.status(), WorkflowRunStatus::Stopped);
    assert_eq!(cancelled.active_node_run_id(), None);
    assert_eq!(
        cancelled.node_runs()[0].status(),
        WorkflowNodeRunStatus::Stopped
    );

    let error = service
        .cancel_workflow_run(session.id(), workflow_run.id())
        .expect_err("terminal workflow run should reject a second cancellation");
    assert!(matches!(error, DaemonError::InvalidWorkflowRunState { .. }));
}

#[test]
fn provider_failure_marks_workflow_and_node_failed() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();

    let failed = service
        .fail_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node should fail");

    assert_eq!(failed.status(), WorkflowRunStatus::Failed);
    assert_eq!(failed.active_node_run_id(), None);
    assert_eq!(
        failed.node_runs()[0].status(),
        WorkflowNodeRunStatus::Failed
    );
}

#[test]
fn node_turn_budget_exhaustion_stops_the_whole_run() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("workflow node should be added");
    service
        .set_workflow_node_max_turns(session.id(), workflow.id(), node.id(), Some(1))
        .expect("node max turns should update");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let node_run = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist");

    let update = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            node_run.id(),
            Some(WorkflowCompletionSnapshot::new("done", None)),
            None,
        )
        .expect("node completion should succeed");

    assert_eq!(update.workflow_run.status(), WorkflowRunStatus::Stopped);
    assert!(update.dispatches.is_empty());
    assert!(update.workflow_run.final_output().is_none());
}

#[test]
fn manual_workflow_launch_rejects_while_any_session_workflow_run_is_active() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let first_workflow = service
        .create_workflow(session.id(), Some("first".to_string()))
        .expect("first workflow should be created");
    let first_node = service
        .add_workflow_node(session.id(), first_workflow.id(), "agent-1")
        .expect("first node should be added");
    let first_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_node.id(),
            Some("entry".to_string()),
        )
        .expect("first endpoint should be created");

    let second_workflow = service
        .create_workflow(session.id(), Some("second".to_string()))
        .expect("second workflow should be created");
    let second_node = service
        .add_workflow_node(session.id(), second_workflow.id(), "agent-2")
        .expect("second node should be added");
    let second_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            second_workflow.id(),
            second_node.id(),
            Some("entry".to_string()),
        )
        .expect("second endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("first workflow run should be created");
    assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);

    let error = service
        .admit_manual_workflow_launch(
            session.id(),
            second_workflow.id(),
            second_endpoint.id(),
            Some("later".to_string()),
        )
        .expect_err("launch should reject while a session workflow run is active");
    assert!(matches!(error, DaemonError::WorkflowLaunchRejected { .. }));
}

#[test]
fn manual_workflow_launch_queue_is_fifo_across_workflows() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let first_workflow = service
        .create_workflow(session.id(), Some("first".to_string()))
        .expect("first workflow should be created");
    let first_node = service
        .add_workflow_node(session.id(), first_workflow.id(), "agent-1")
        .expect("first node should be added");
    let first_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_node.id(),
            Some("entry".to_string()),
        )
        .expect("first endpoint should be created");

    let second_workflow = service
        .create_workflow(session.id(), Some("second".to_string()))
        .expect("second workflow should be created");
    let second_node = service
        .add_workflow_node(session.id(), second_workflow.id(), "agent-2")
        .expect("second node should be added");
    let second_endpoint = service
        .create_workflow_endpoint(
            session.id(),
            second_workflow.id(),
            second_node.id(),
            Some("entry".to_string()),
        )
        .expect("second endpoint should be created");

    service
        .set_workflow_launch_policy(session.id(), WorkflowLaunchPolicy::Queue)
        .expect("queue policy should be set");
    let active = service
        .invoke_workflow_endpoint(
            session.id(),
            first_workflow.id(),
            first_endpoint.id(),
            Some("go".to_string()),
        )
        .expect("active workflow run should be created");
    assert_eq!(active.status(), WorkflowRunStatus::Created);

    let first_queued = service
        .admit_manual_workflow_launch(
            session.id(),
            second_workflow.id(),
            second_endpoint.id(),
            Some("second".to_string()),
        )
        .expect("second workflow should queue");
    let second_queued = service
        .admit_manual_workflow_launch(
            session.id(),
            first_workflow.id(),
            first_endpoint.id(),
            Some("third".to_string()),
        )
        .expect("third launch should queue");

    let queued = service
        .list_queued_workflow_launches(session.id())
        .expect("queued launches should list");
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].source(), QueuedWorkflowLaunchSource::Manual);
    assert_eq!(queued[1].source(), QueuedWorkflowLaunchSource::Manual);

    match first_queued {
        WorkflowLaunchAdmission::Queued(ref queued_launch) => {
            assert_eq!(queued[0].id(), queued_launch.id())
        }
        WorkflowLaunchAdmission::StartNow => panic!("expected queued launch"),
    }
    match second_queued {
        WorkflowLaunchAdmission::Queued(ref queued_launch) => {
            assert_eq!(queued[1].id(), queued_launch.id())
        }
        WorkflowLaunchAdmission::StartNow => panic!("expected queued launch"),
    }

    service
        .cancel_workflow_run(session.id(), active.id())
        .expect("active workflow run should stop");
    let dequeued = service
        .dequeue_next_workflow_launch(session.id())
        .expect("queued workflow launch should dequeue")
        .expect("expected queued workflow launch");
    assert_eq!(dequeued.id(), queued[0].id());
}

#[test]
fn workflow_console_supports_append_read_and_clear() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");

    let initial = service
        .read_workflow_console(session.id(), workflow.id())
        .expect("console should read");
    assert_eq!(initial.workflow_id(), workflow.id());
    assert!(initial.entries().is_empty());

    let first = service
        .append_workflow_console_entry(
            session.id(),
            workflow.id(),
            Some("node-run-1".to_string()),
            Some("agent-1".to_string()),
            "hello\n",
        )
        .expect("console append should succeed");
    assert_eq!(first.text(), "hello\n");

    let second = service
        .append_workflow_console_entry(
            session.id(),
            workflow.id(),
            Some("node-run-2".to_string()),
            Some("agent-2".to_string()),
            "world\n",
        )
        .expect("console append should succeed");
    assert_eq!(second.text(), "world\n");

    let populated = service
        .read_workflow_console(session.id(), workflow.id())
        .expect("console should read");
    assert_eq!(populated.entries().len(), 2);
    assert_eq!(populated.entries()[0].text(), "hello\n");
    assert_eq!(populated.entries()[1].text(), "world\n");

    let cleared = service
        .clear_workflow_console(session.id(), workflow.id())
        .expect("console clear should succeed");
    assert!(cleared.entries().is_empty());
}

#[test]
fn workflow_watchdog_skip_policy_skips_when_endpoint_run_is_active() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            None,
        )
        .expect("watchdog should be created");
    let run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("manual".to_string()),
        )
        .expect("workflow should invoke");
    let plans = service
        .collect_due_workflow_watchdog_invocations(watchdog.next_run_at_ms())
        .expect("watchdog collection should succeed");
    assert!(plans.is_empty());
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert_eq!(watchdog.last_status(), Some("skipped_running"));
    assert!(!watchdog.pending_run());
    assert_eq!(run.status(), WorkflowRunStatus::Created);
}

#[test]
fn workflow_watchdog_queue_policy_queues_one_pending_run() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Queue,
            None,
        )
        .expect("watchdog should be created");
    let run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("manual".to_string()),
        )
        .expect("workflow should invoke");
    let queued = service
        .collect_due_workflow_watchdog_invocations(watchdog.next_run_at_ms())
        .expect("watchdog collection should succeed");
    assert!(queued.is_empty());
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert_eq!(watchdog.last_status(), Some("queued_running"));
    assert!(watchdog.pending_run());

    let session_mut = service
        .store
        .get_mut(session.id())
        .expect("session should exist");
    let active_run = session_mut
        .workflow_run_mut(run.id())
        .expect("workflow run should exist");
    active_run.set_status(WorkflowRunStatus::Completed);

    let plans = service
        .collect_due_workflow_watchdog_invocations(unix_epoch_ms())
        .expect("watchdog collection should succeed");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].endpoint_id, endpoint.id());
    let watchdog = service
        .resolve_workflow_watchdog_ref(session.id(), watchdog.id())
        .expect("watchdog should resolve");
    assert!(!watchdog.pending_run());
    assert_eq!(watchdog.last_status(), Some("invoking_pending"));
}

#[test]
fn completing_a_workflow_node_run_creates_structured_downstream_dispatches() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);
    let workflow = service
        .create_workflow(session.id(), Some("review".to_string()))
        .expect("workflow should be created");
    let first = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("first workflow node should be added");
    let second = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("second workflow node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            first.id(),
            second.id(),
            None,
            None,
        )
        .expect("workflow edge should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            first.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("review this diff".to_string()),
        )
        .expect("workflow run should be created");
    let started = service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("entry node should start");
    assert_eq!(started.status(), WorkflowRunStatus::Running);
    assert_eq!(
        started.active_node_run_id(),
        Some(workflow_run.node_runs()[0].id())
    );

    let completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
            None,
            None,
        )
        .expect("entry node completion should route downstream work");
    assert_eq!(completion.workflow_run.status(), WorkflowRunStatus::Waiting);
    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(completion.dispatches[0].node_run.node_id(), second.id());
    assert_eq!(completion.dispatches[0].messages.len(), 1);
    assert_eq!(
        completion.dispatches[0].messages[0].target_node_id(),
        second.id()
    );
    let payload: WorkflowHandoffPayload =
        serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
            .expect("handoff payload should deserialize");
    assert_eq!(payload.workflow_run_id(), workflow_run.id());
    assert_eq!(payload.workflow_id(), workflow.id());
    assert_eq!(
        payload.source_node_run_id(),
        workflow_run.node_runs()[0].id()
    );
    assert_eq!(payload.source_node_id(), first.id());
    assert_eq!(payload.source_agent_id(), "agent-1");
    assert_eq!(payload.target_node_id(), second.id());
    assert_eq!(payload.invocation_prompt(), Some("review this diff"));
    assert!(payload.completion().is_none());

    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.status(), WorkflowRunStatus::Waiting);
    assert_eq!(resolved.node_runs().len(), 2);
    assert_eq!(resolved.messages().len(), 2);
}

#[test]
fn join_nodes_wait_for_all_inputs_before_dispatching_once() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(
        &mut service,
        session.id(),
        &["agent-1", "agent-2", "agent-3", "agent-4"],
    );
    let workflow = service
        .create_workflow(session.id(), Some("join".to_string()))
        .expect("workflow should be created");
    let entry = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("entry node should be added");
    let branch_one = service
        .add_workflow_node(session.id(), workflow.id(), "agent-2")
        .expect("branch one node should be added");
    let branch_two = service
        .add_workflow_node(session.id(), workflow.id(), "agent-3")
        .expect("branch two node should be added");
    let join = service
        .add_workflow_node(session.id(), workflow.id(), "agent-4")
        .expect("join node should be added");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            entry.id(),
            branch_one.id(),
            None,
            None,
        )
        .expect("entry should connect to branch one");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            entry.id(),
            branch_two.id(),
            None,
            None,
        )
        .expect("entry should connect to branch two");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            branch_one.id(),
            join.id(),
            None,
            None,
        )
        .expect("branch one should connect to join");
    service
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            branch_two.id(),
            join.id(),
            None,
            None,
        )
        .expect("branch two should connect to join");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            entry.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run the join drill".to_string()),
        )
        .expect("workflow run should be created");
    let started = service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("entry node should start");
    let entry_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            started.node_runs()[0].id(),
            None,
            None,
        )
        .expect("entry node should dispatch both branches");
    assert_eq!(entry_completion.dispatches.len(), 2);

    let branch_one_run = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_one.id())
        .expect("branch one dispatch should exist")
        .node_run
        .clone();
    let branch_two_run = entry_completion
        .dispatches
        .iter()
        .find(|dispatch| dispatch.node_run.node_id() == branch_two.id())
        .expect("branch two dispatch should exist")
        .node_run
        .clone();
    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_one_run.id())
        .expect("branch one should start");
    let branch_one_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_one_run.id(),
            None,
            None,
        )
        .expect("branch one completion should succeed");
    assert!(branch_one_completion.dispatches.is_empty());
    let waiting = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve after first branch");
    assert_eq!(waiting.node_runs().len(), 3);
    assert_eq!(
        waiting
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join.id())
            .count(),
        1
    );
    assert!(waiting
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join.id())
        .all(|message| message.consumed_by_node_run_id().is_none()));

    service
        .start_workflow_node_run(session.id(), workflow_run.id(), branch_two_run.id())
        .expect("branch two should start");
    let branch_two_completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            branch_two_run.id(),
            None,
            None,
        )
        .expect("branch two completion should succeed");
    assert_eq!(branch_two_completion.dispatches.len(), 1);
    let join_dispatch = &branch_two_completion.dispatches[0];
    assert_eq!(join_dispatch.node_run.node_id(), join.id());
    assert_eq!(join_dispatch.messages.len(), 2);
    assert_eq!(
        join_dispatch
            .messages
            .iter()
            .map(|message| message.target_node_id())
            .collect::<Vec<_>>(),
        vec![join.id(), join.id()]
    );
    let resolved = service
        .resolve_workflow_run_ref(session.id(), workflow_run.id())
        .expect("workflow run should resolve");
    assert_eq!(resolved.node_runs().len(), 4);
    assert_eq!(
        resolved
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join.id())
            .count(),
        2
    );
    assert!(resolved
        .messages()
        .iter()
        .filter(|message| message.target_node_id() == join.id())
        .all(|message| message.consumed_by_node_run_id() == Some(join_dispatch.node_run.id())));
}

#[test]
fn delete_session_removes_it_from_registry() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let deleted = service
        .delete_session(created.id())
        .expect("session should delete");

    assert_eq!(deleted.id(), created.id());
    assert!(matches!(
        service.get_session(created.id()),
        Err(DaemonError::SessionNotFound { .. })
    ));
    assert!(service.list_sessions().is_empty());
}

fn seed_agents(service: &mut SessionService, session_id: &str, agent_ids: &[&str]) {
    let session = service
        .store
        .get_mut(session_id)
        .expect("session should exist for test seeding");
    let agents = agent_ids
        .iter()
        .enumerate()
        .map(|(index, agent_id)| {
            AgentInstance::new(
                agent_id.to_string(),
                format!("ref-{agent_id}"),
                session_id.to_string(),
                None,
                "dev-stub",
                Some("default".to_string()),
                None,
                None,
                GridPosition::new(0, index as u32, 1, 1),
            )
        })
        .collect::<Vec<_>>();
    session.set_agents(agents);
}

#[test]
fn prompt_queue_starts_then_queues_then_advances() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    service
        .add_attachment_to_session(created.id(), "attachment-1")
        .expect("attachment should be added");
    service
        .add_attachment_to_session(created.id(), "attachment-2")
        .expect("attachment should be added");

    let (_, first) = service
        .submit_prompt(
            created.id(),
            "attachment-1",
            "agent-1",
            "first prompt",
            Vec::new(),
        )
        .expect("first prompt should start");
    let (_, second) = service
        .submit_prompt(
            created.id(),
            "attachment-2",
            "agent-1",
            "second prompt",
            Vec::new(),
        )
        .expect("second prompt should queue");

    match first {
        PromptSubmissionOutcome::Started { prompt } => assert_eq!(prompt.id(), "prompt-1"),
        _ => panic!("expected running prompt"),
    }
    match second {
        PromptSubmissionOutcome::Queued { prompt } => assert_eq!(prompt.id(), "prompt-2"),
        _ => panic!("expected queued prompt"),
    }

    assert_eq!(
        service
            .get_session(created.id())
            .expect("session should exist")
            .scheduler_state(),
        SchedulerState::Waiting
    );
    let serialized = serde_json::to_value(
        service
            .get_session(created.id())
            .expect("session should exist"),
    )
    .expect("session should serialize");
    assert!(serialized.get("prompt_runtime").is_none());
    assert!(serialized.get("prompt_states").is_some());
    assert!(serialized.get("active_prompt").is_some());
    assert!(serialized.get("queued_prompts").is_some());
    assert!(serialized.get("scheduler_state").is_some());

    let (_session, completed) = service
        .complete_active_prompt(created.id(), "agent-1")
        .expect("active prompt should complete");
    assert_eq!(completed.id(), "prompt-1");
    let (session, started_next) = service
        .activate_next_queued_prompt(created.id(), "agent-1")
        .expect("next prompt should activate");
    assert_eq!(
        started_next.expect("next prompt should start").id(),
        "prompt-2"
    );
    assert_eq!(
        session.active_prompt().expect("active prompt exists").id(),
        "prompt-2"
    );
    assert_eq!(session.scheduler_state(), SchedulerState::Running);
}

#[test]
fn activating_expected_queued_prompt_validates_queue_front() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    service
        .add_attachment_to_session(created.id(), "attachment-1")
        .expect("attachment should be added");
    service
        .add_attachment_to_session(created.id(), "attachment-2")
        .expect("attachment should be added");
    service
        .submit_prompt(
            created.id(),
            "attachment-1",
            "agent-1",
            "first prompt",
            Vec::new(),
        )
        .expect("first prompt should start");
    service
        .submit_prompt(
            created.id(),
            "attachment-2",
            "agent-1",
            "second prompt",
            Vec::new(),
        )
        .expect("second prompt should queue");
    service
        .complete_active_prompt(created.id(), "agent-1")
        .expect("active prompt should complete");

    let error = service
        .activate_expected_next_queued_prompt(created.id(), "agent-1", "prompt-mismatch")
        .expect_err("mismatched expected prompt should fail");
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "activate expected queued prompt");
            assert!(message.contains("prompt-mismatch"));
            assert!(message.contains("prompt-2"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let (session, started_next) = service
        .activate_expected_next_queued_prompt(created.id(), "agent-1", "prompt-2")
        .expect("matching expected prompt should activate");
    assert_eq!(
        started_next.expect("next prompt should start").id(),
        "prompt-2"
    );
    assert_eq!(
        session.active_prompt().expect("active prompt exists").id(),
        "prompt-2"
    );
}

#[test]
fn config_updates_are_versioned() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    service
        .add_attachment_to_session(created.id(), "attachment-1")
        .expect("attachment should be added");

    let mut changes = BTreeMap::new();
    changes.insert("theme".to_string(), "compact".to_string());
    let (_, config) = service
        .update_config(created.id(), "attachment-1", changes, false)
        .expect("config should update");

    assert_eq!(config.version(), 1);
    assert_eq!(
        config.values().get("theme").map(String::as_str),
        Some("compact")
    );
    assert_eq!(config.updated_by_attachment_id(), Some("attachment-1"));
}

#[test]
fn rejects_idle_required_config_update_while_prompt_running() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    service
        .add_attachment_to_session(created.id(), "attachment-1")
        .expect("attachment should be added");
    service
        .submit_prompt(
            created.id(),
            "attachment-1",
            "agent-1",
            "first prompt",
            Vec::new(),
        )
        .expect("prompt should start");

    let error = service
        .update_config(created.id(), "attachment-1", BTreeMap::new(), true)
        .expect_err("idle-required config change should be rejected");

    match error {
        DaemonError::ConfigChangeRejectedWhilePromptRunning { session_id } => {
            assert_eq!(session_id, created.id())
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn detaching_an_attachment_keeps_its_active_prompt_running() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    service
        .add_attachment_to_session(created.id(), "attachment-1")
        .expect("attachment should be added");

    let (_, outcome) = service
        .submit_prompt(
            created.id(),
            "attachment-1",
            "agent-1",
            "background prompt",
            Vec::new(),
        )
        .expect("prompt should start");
    let prompt_id = match outcome {
        PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        other => panic!("expected running prompt, got {other:?}"),
    };

    let (session, effect) = service
        .remove_attachment_from_session(created.id(), "attachment-1")
        .expect("detach should succeed");

    assert!(!effect.removed_active_prompt);
    assert_eq!(effect.removed_queued_prompt_count, 0);
    assert!(session.attachment_ids().is_empty());
    assert_eq!(
        session.active_prompt().map(|prompt| prompt.id()),
        Some(prompt_id.as_str())
    );
    assert_eq!(session.scheduler_state(), SchedulerState::Running);
}

#[test]
fn workflow_watchdog_defaults_to_bounded_max_wakeups() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");

    let watchdog = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            None,
        )
        .expect("watchdog should be created");

    assert_eq!(
        watchdog.max_wakeups(),
        Some(crate::session::DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS),
    );
    assert_eq!(watchdog.wakeups_executed(), 0);
}

#[test]
fn workflow_watchdog_budget_can_be_unbounded_or_auto_disable_when_exhausted() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = service
        .create_workflow(session.id(), Some("watchdog".to_string()))
        .expect("workflow should be created");
    let node = service
        .add_workflow_node(session.id(), workflow.id(), "agent-1")
        .expect("node should be added");
    let endpoint = service
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");

    let bounded = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            Some(Some(1)),
        )
        .expect("bounded watchdog should be created");
    let unbounded = service
        .create_workflow_watchdog(
            session.id(),
            workflow.id(),
            endpoint.id(),
            1,
            "run".to_string(),
            WorkflowWatchdogPolicy::Skip,
            Some(None),
        )
        .expect("unbounded watchdog should be created");

    let bounded = service
        .mark_workflow_watchdog_invoked(session.id(), bounded.id(), "workflow-run-1")
        .expect("bounded watchdog should update");
    assert_eq!(bounded.max_wakeups(), Some(1));
    assert_eq!(bounded.wakeups_executed(), 1);
    assert!(!bounded.enabled());
    assert_eq!(bounded.last_status(), Some("completed_budget"));

    let unbounded = service
        .mark_workflow_watchdog_invoked(session.id(), unbounded.id(), "workflow-run-2")
        .expect("unbounded watchdog should update");
    assert_eq!(unbounded.max_wakeups(), None);
    assert_eq!(unbounded.wakeups_executed(), 1);
    assert!(unbounded.enabled());
    assert_eq!(unbounded.last_status(), Some("started"));
}
