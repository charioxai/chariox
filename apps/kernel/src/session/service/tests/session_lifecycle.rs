use super::*;
use crate::session::{RuntimeProjectKind, RuntimeSession};

#[test]
fn creates_gets_and_lists_sessions() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    assert_eq!(created.id().len(), 16);
    assert!(created.id().chars().all(|char| char.is_ascii_hexdigit()));
    assert_eq!(created.alias(), Some("workspace-1-1"));
    assert_eq!(created.workspace_id(), "workspace-1");
    assert_eq!(created.worktree_id(), "worktree-1");
    assert_eq!(created.host_machine_id(), "machine-test");
    assert_eq!(created.host_daemon_id(), "daemon-test");
    assert_eq!(created.status(), SessionStatus::Created);
    assert!(created.active_provider_run_id().is_none());
    assert_eq!(
        created.max_agents(),
        crate::session::DEFAULT_SESSION_MAX_AGENTS
    );
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
fn create_session_uses_configured_default_agent_cap() {
    let mut config = test_config();
    config.user_config.workflow.session_default_max_agents = Some(2048);
    let mut service = SessionService::new(&config);

    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    assert_eq!(created.max_agents(), 2048);
}

#[test]
fn create_session_rejects_deprecated_metaagent_request() {
    let mut service = SessionService::new(&test_config());

    let error = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1").with_metaagent(true))
        .expect_err("deprecated metaagent session creation should fail");

    assert!(
        error
            .to_string()
            .contains("send `/meta <task>` to enter meta mode"),
        "unexpected error: {error}"
    );
    assert_eq!(service.active_session_count(), 0);
}

#[test]
fn prompt_id_allocator_advances_past_observed_prompt_ids() {
    let service = SessionService::new(&test_config());

    service.observe_prompt_id("prompt-41");
    assert_eq!(service.reserve_prompt_id(), "prompt-42");

    service.observe_prompt_id("prompt-7");
    assert_eq!(service.reserve_prompt_id(), "prompt-43");
}

#[test]
fn create_session_generates_default_aliases_from_workspace_name() {
    let mut service = SessionService::new(&test_config());
    let first = service
        .create_session(CreateSessionRequest::new(
            "/Users/miguel/arroba-cloud",
            "worktree-1",
        ))
        .expect("first session should be created");
    let second = service
        .create_session(CreateSessionRequest::new(
            "/Users/miguel/arroba-cloud",
            "worktree-2",
        ))
        .expect("second session should be created");

    assert_eq!(first.alias(), Some("arroba-cloud-1"));
    assert_eq!(second.alias(), Some("arroba-cloud-2"));
}

#[test]
fn create_session_sanitizes_default_alias_base() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new(
            "/tmp/Arroba Cloud!!",
            "worktree-1",
        ))
        .expect("session should be created");

    assert_eq!(created.alias(), Some("arroba-cloud-1"));
}

#[test]
fn create_session_generates_default_alias_for_blank_alias() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("/repo", "worktree-1").with_alias("  "))
        .expect("session should be created");

    assert_eq!(created.alias(), Some("repo-1"));
}

#[test]
fn hidden_sessions_do_not_get_default_aliases() {
    let mut service = SessionService::new(&test_config());
    let hidden = service
        .create_session(CreateSessionRequest::new("/repo", "worktree-1").with_hidden(true))
        .expect("hidden session should be created");
    assert_eq!(hidden.project_id(), "");
    assert!(service
        .list_projects(DEFAULT_LOCAL_USER_ID, true)
        .is_empty());

    let visible = service
        .create_session(CreateSessionRequest::new("/repo", "worktree-2"))
        .expect("visible session should be created");

    assert_eq!(hidden.alias(), None);
    assert_eq!(visible.alias(), Some("repo-1"));
    assert_eq!(service.list_projects(DEFAULT_LOCAL_USER_ID, true).len(), 1);
}

#[test]
fn restoring_hidden_session_detaches_it_from_legacy_project() {
    let mut source = SessionService::new(&test_config());
    let visible = source
        .create_session(CreateSessionRequest::new("/repo", "worktree-visible"))
        .expect("visible session should be created");
    let project = source
        .get_project(visible.project_id())
        .expect("visible project should exist");

    let mut hidden = RuntimeSession::new(
        "hidden-runtime",
        None,
        "/repo",
        "worktree-hidden",
        "machine-test",
        "daemon-test",
    );
    hidden.set_hidden(true);
    assert!(hidden.assign_project_id(project.id()));

    let mut restored = SessionService::new(&test_config());
    restored.restore_projects(vec![project.clone()]);
    let hidden = restored.restore_session(hidden);
    assert_eq!(hidden.project_id(), "");
    assert!(restored
        .list_visible_projects(DEFAULT_LOCAL_USER_ID, true)
        .is_empty());

    assert_eq!(
        restored.remove_projects_without_visible_sessions(),
        vec![project]
    );
    assert!(restored
        .list_projects(DEFAULT_LOCAL_USER_ID, true)
        .is_empty());
}

#[test]
fn default_alias_counts_existing_unaliased_sessions_and_skips_conflicts() {
    let mut service = SessionService::new(&test_config());
    service
        .create_session(CreateSessionRequest::new("/repo", "worktree-1").with_hidden(true))
        .expect("hidden session should not count");
    service.restore_session(RuntimeSession::new(
        "legacy-session",
        None,
        "/repo".to_string(),
        "worktree-legacy".to_string(),
        "machine-test".to_string(),
        "daemon-test".to_string(),
    ));
    service
        .create_session(CreateSessionRequest::new("/repo", "worktree-2").with_alias("repo-3"))
        .expect("explicit alias should be created");

    let created = service
        .create_session(CreateSessionRequest::new("/repo", "worktree-3"))
        .expect("session should be created");

    assert_eq!(created.alias(), Some("repo-4"));
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
fn create_session_stores_workspace_live_sync_mode_override() {
    let mut service = SessionService::new(&test_config());

    let created = service
        .create_session(
            CreateSessionRequest::new("workspace-1", "worktree-1")
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        )
        .expect("session should be created");

    assert_eq!(
        created.workspace_live_sync_mode(),
        Some(crate::config::WorkspaceLiveSyncMode::Tracked)
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
    assert_eq!(
        session.members()[0].collaboration_level(),
        crate::session::CollaborationLevel::Private
    );

    let (session, invite) = service
        .create_session_invite(
            session.id(),
            "invite-1".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
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
    assert_eq!(
        member.collaboration_level(),
        crate::session::CollaborationLevel::Private
    );
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
fn delete_session_removes_it_from_registry() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let project_id = created.project_id().to_string();

    let deleted = service
        .delete_session(created.id())
        .expect("session should delete");

    assert_eq!(deleted.id(), created.id());
    assert!(matches!(
        service.get_session(created.id()),
        Err(DaemonError::SessionNotFound { .. })
    ));
    assert!(service.list_sessions().is_empty());
    assert!(service.get_project(&project_id).is_err());
    assert!(service
        .list_projects(DEFAULT_LOCAL_USER_ID, true)
        .is_empty());
}

#[test]
fn delete_session_keeps_project_until_last_visible_session_is_deleted() {
    let mut service = SessionService::new(&test_config());
    let first = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let second = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-2"))
        .expect("second session should be created");
    assert_eq!(first.project_id(), second.project_id());
    let project_id = first.project_id().to_string();

    service
        .delete_session(first.id())
        .expect("first session should delete");
    assert!(service.get_project(&project_id).is_ok());

    service
        .delete_session(second.id())
        .expect("second session should delete");
    assert!(service.get_project(&project_id).is_err());
}

#[test]
fn kernel_restart_reconciliation_clears_restored_attachments() {
    let mut service = SessionService::new(&test_config());
    let mut session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    session.add_attachment("attachment-1");
    session.add_attachment("attachment-2");
    session.activate_prompt(crate::session::PromptQueueItem::new(
        "prompt-restart",
        "attachment-1",
        "agent-1",
        "recover me",
        crate::session::PromptStatus::Running,
    ));

    let reconciliation = session.reconcile_after_kernel_restart();
    service.restore_session(session.clone());

    assert_eq!(reconciliation.cleared_attachment_count, 2);
    assert_eq!(reconciliation.recoverable_prompt_count, 1);
    assert_eq!(reconciliation.interrupted_prompt_count, 0);
    assert!(reconciliation.changed());
    let restored = service
        .get_session(session.id())
        .expect("session should still exist");
    assert!(restored.attachment_ids().is_empty());
    assert_eq!(
        restored
            .active_prompt_for_agent("agent-1")
            .map(|prompt| prompt.id()),
        Some("prompt-restart")
    );

    let mut shutdown = restored;
    let shutdown_reconciliation = shutdown.interrupt_runtime_for_shutdown();
    assert_eq!(shutdown_reconciliation.interrupted_prompt_count, 1);
    assert!(shutdown.active_prompt_for_agent("agent-1").is_none());
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
fn config_update_idle_admission_is_not_owned_by_session_service() {
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

    let (_session, config) = service
        .update_config(created.id(), "attachment-1", BTreeMap::new(), true)
        .expect("low-level session service should not own prompt-state admission");

    assert_eq!(config.version(), 1);
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
fn detaching_an_attachment_keeps_its_queued_prompts() {
    let mut service = SessionService::new(&test_config());
    let created = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    service
        .add_attachment_to_session(created.id(), "attachment-1")
        .expect("attachment should be added");

    let (_, active) = service
        .submit_prompt(
            created.id(),
            "attachment-1",
            "agent-1",
            "active prompt",
            Vec::new(),
        )
        .expect("active prompt should start");
    assert!(matches!(active, PromptSubmissionOutcome::Started { .. }));
    let (_, queued) = service
        .submit_prompt(
            created.id(),
            "attachment-1",
            "agent-1",
            "queued prompt",
            Vec::new(),
        )
        .expect("second prompt should queue");
    let queued_prompt_id = match queued {
        PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        other => panic!("expected queued prompt, got {other:?}"),
    };

    let (session, effect) = service
        .remove_attachment_from_session(created.id(), "attachment-1")
        .expect("detach should succeed");

    assert_eq!(effect.removed_queued_prompt_count, 0);
    assert_eq!(session.queued_prompts().len(), 1);
    assert_eq!(session.queued_prompts()[0].id(), queued_prompt_id);
    assert_eq!(session.queued_prompts()[0].prompt(), "queued prompt");
}

#[test]
fn legacy_session_project_migration_uses_repo_label_hint_and_is_restart_stable() {
    let legacy = RuntimeSession::new(
        "legacy-session",
        Some("legacy".to_string()),
        "/workspace/arroba",
        "/workspace/arroba",
        "machine-test",
        "daemon-test",
    );
    let mut legacy_json = serde_json::to_value(&legacy).expect("legacy session should encode");
    legacy_json
        .as_object_mut()
        .expect("session should encode as an object")
        .remove("project_id");
    let legacy: RuntimeSession =
        serde_json::from_value(legacy_json).expect("legacy session should decode");

    let mut first_service = SessionService::new(&test_config());
    let migrated = first_service
        .restore_session_with_default_project_name_hint(legacy, Some("mgutierrez09/arroba"));
    let project = first_service
        .get_project(migrated.project_id())
        .expect("migrated default project should exist");
    assert_eq!(project.name(), "mgutierrez09/arroba");
    assert_eq!(project.kind(), RuntimeProjectKind::Default);

    let mut restarted_service = SessionService::new(&test_config());
    restarted_service.restore_projects(first_service.durable_projects());
    let restored = restarted_service.restore_session(migrated);
    let restored_project = restarted_service
        .get_project(restored.project_id())
        .expect("default project should survive restart");
    assert_eq!(restored_project.id(), project.id());
    assert_eq!(restored_project.name(), "mgutierrez09/arroba");
}
