use super::*;

#[test]
fn project_lifecycle_archives_idle_sessions_and_restore_parks_them() {
    let worktree = crate::test_support::TestWorktree::new("session-control-project-lifecycle");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let archived = match harness
        .dispatch(LocalDaemonRequest::ArchiveProject(ArchiveProjectRequest {
            project_id: session.project_id().to_string(),
        }))
        .expect("idle project should archive")
    {
        LocalDaemonResponse::ProjectArchived { project, sessions } => (project, sessions),
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        archived.0.status(),
        crate::session::RuntimeProjectStatus::Archived
    );
    assert_eq!(archived.1.len(), 1);
    assert_eq!(archived.1[0].status(), crate::session::SessionStatus::Ended);

    let restored = match harness
        .dispatch(LocalDaemonRequest::RestoreProject(RestoreProjectRequest {
            project_id: session.project_id().to_string(),
        }))
        .expect("archived project should restore")
    {
        LocalDaemonResponse::ProjectRestored { project, sessions } => (project, sessions),
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        restored.0.status(),
        crate::session::RuntimeProjectStatus::Active
    );
    assert_eq!(restored.1.len(), 1);
    assert_eq!(
        restored.1[0].status(),
        crate::session::SessionStatus::Parked
    );
    assert!(restored.1[0].active_provider_run_id().is_none());
}

#[test]
fn project_delete_cascades_when_last_session_removes_project_record() {
    let workspace = crate::test_support::TestWorktree::new("session-control-project-delete");
    let first_worktree = crate::test_support::TestWorktree::new("session-control-delete-first");
    let second_worktree = crate::test_support::TestWorktree::new("session-control-delete-second");
    let harness = LocalRouterTestHarness::new();
    let first = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace.path().display().to_string(),
                first_worktree.path().display().to_string(),
            )
            .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let second = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                workspace.path().display().to_string(),
                second_worktree.path().display().to_string(),
            )
            .with_project_selection(SessionProjectSelection::Existing {
                project_id: first.project_id().to_string(),
            }),
        ))
        .expect("second project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let (project, sessions) = match harness
        .dispatch(LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
            project_id: first.project_id().to_string(),
        }))
        .expect("project should delete")
    {
        LocalDaemonResponse::ProjectDeleted { project, sessions } => (project, sessions),
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(project.id(), first.project_id());
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|session| session.id() == first.id()));
    assert!(sessions.iter().any(|session| session.id() == second.id()));

    let projects = match harness
        .dispatch(LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: true,
        }))
        .expect("projects should list")
    {
        LocalDaemonResponse::ProjectsListed { projects } => projects,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert!(projects.is_empty());
}

#[test]
fn project_requests_enforce_owner_and_named_numbering() {
    let worktree = crate::test_support::TestWorktree::new("session-control-project-numbering");
    let harness = LocalRouterTestHarness::new();
    let create = |harness: &LocalRouterTestHarness| match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let first = create(&harness);
    let second = create(&harness);
    let projects = match harness
        .dispatch(LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: true,
        }))
        .expect("projects should list")
    {
        LocalDaemonResponse::ProjectsListed { projects } => projects,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        projects
            .iter()
            .find(|project| project.id() == first.project_id())
            .map(|project| project.name()),
        Some("Project-1")
    );
    assert_eq!(
        projects
            .iter()
            .find(|project| project.id() == second.project_id())
            .map(|project| project.name()),
        Some("Project-2")
    );

    let error = harness
        .dispatch_as_user(
            "another-user",
            LocalDaemonRequest::RenameProject(RenameProjectRequest {
                project_id: first.project_id().to_string(),
                name: "not-owned".to_string(),
            }),
        )
        .expect_err("non-owner project mutation should fail");
    assert!(error.to_string().contains("does not own project"));
}

#[test]
fn project_workspace_membership_updates_through_the_local_api() {
    let primary_worktree =
        crate::test_support::TestWorktree::new("session-control-project-primary");
    let supporting_worktree =
        crate::test_support::TestWorktree::new("session-control-project-supporting");
    let harness = LocalRouterTestHarness::new();
    let first = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                primary_worktree.path().display().to_string(),
                primary_worktree.path().display().to_string(),
            )
            .with_project_selection(SessionProjectSelection::New),
        ))
        .expect("named project session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let project = match harness
        .dispatch(LocalDaemonRequest::UpdateProjectWorkspaces(
            UpdateProjectWorkspacesRequest {
                project_id: first.project_id().to_string(),
                workspace_ids: vec![
                    primary_worktree.path().display().to_string(),
                    supporting_worktree.path().display().to_string(),
                ],
            },
        ))
        .expect("project Workspaces should update")
    {
        LocalDaemonResponse::ProjectWorkspacesUpdated { project } => project,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        project.workspace_ids(),
        &[
            primary_worktree.path().display().to_string(),
            supporting_worktree.path().display().to_string()
        ]
    );

    let supporting = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                supporting_worktree.path().display().to_string(),
                supporting_worktree.path().display().to_string(),
            )
            .with_project_selection(SessionProjectSelection::Existing {
                project_id: first.project_id().to_string(),
            }),
        ))
        .expect("supporting Workspace should create a session in the Project")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    assert_eq!(
        supporting.workspace_id(),
        supporting_worktree.path().to_str().unwrap()
    );
    assert_eq!(supporting.project_id(), first.project_id());
}

#[test]
fn archived_default_project_rejects_default_session_creation_until_restored() {
    let first_worktree = crate::test_support::TestWorktree::new("session-control-archive-first");
    let second_worktree = crate::test_support::TestWorktree::new("session-control-archive-second");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                first_worktree.path().display().to_string(),
                first_worktree.path().display().to_string(),
            ),
        ))
        .expect("default project session should create")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::ArchiveProject(ArchiveProjectRequest {
            project_id: session.project_id().to_string(),
        }))
        .expect("default project should archive while idle");

    let error = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                first_worktree.path().display().to_string(),
                second_worktree.path().display().to_string(),
            ),
        ))
        .expect_err("archived default project should reject session creation");
    assert!(error
        .to_string()
        .contains("restore it before creating a session"));
}

#[test]
fn local_request_api_supports_session_attach_and_end() {
    let worktree = crate::test_support::TestWorktree::new("session-control-attach-end");
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let ended = match harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().to_string(),
        }))
        .expect("end session should succeed")
    {
        LocalDaemonResponse::SessionEnded { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), attachment.id());
    assert_eq!(ended.id(), session.id());
    harness.with_app(|app| {
        assert!(app.attachments().get_attachment(detached.id()).is_err());
    });
}

#[test]
fn session_attach_clears_a_missing_active_provider_run() {
    let worktree = crate::test_support::TestWorktree::new("session-control-stale-run");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-worker-settlement".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("initial attachment should succeed");
    harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("remote-worker".to_string()),
            provider: Some("opencode".to_string()),
            model: Some("kimi-k2.6".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("second agent should make stale-run recovery exercise the multi-agent path");
    harness.with_app_mut(|app| {
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some("provider-run-missing".to_string()))
            .expect("stale pointer should be installed");
        let stale = app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available");
        assert_eq!(stale.active_provider_run_id(), Some("provider-run-missing"));
    });

    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-worker-settlement".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should recover from a stale provider-run pointer");

    let attached = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.active_provider_run_id(), None);
}

#[test]
fn session_attach_clears_a_projected_leased_run_for_an_unfocused_agent() {
    let worktree = crate::test_support::TestWorktree::new("session-control-projected-run");
    let harness = LocalRouterTestHarness::new();
    let (session, focused_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-projected-worker-run".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("initial attachment should succeed");
    let remote_agent = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("remote-worker".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5.6-sol".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("second agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.agents()
            .bind_remote_execution(
                remote_agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("provider-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        app.sessions_mut()
            .set_focused_agent(session.id(), Some(focused_agent.id().to_string()))
            .expect("first agent should be focused");
        let request =
            LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "gpt-5.6-sol")
                .with_agent_id(remote_agent.id());
        let mut worker_run = RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        worker_run.mark_running();
        let (_, projected_run) = worker_run.project_leased_for_home_agent(
            "leased-agent-1",
            session.id(),
            remote_agent.id(),
        );
        let projected_run_id = projected_run.id().to_string();
        app.update_provider_run_projection(projected_run);
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(projected_run_id))
            .expect("projected run pointer should be installed");
    });

    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-after-projected-worker-run".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should not try to park a projected worker run locally");

    let attached = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(attached.active_provider_run_id(), None);
}

#[test]
fn local_request_api_resolves_and_deletes_sessions_by_ref() {
    let worktree = crate::test_support::TestWorktree::new("session-control-resolve-delete");
    let harness = LocalRouterTestHarness::new();
    let (session, _agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request().with_alias("main"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let workspace_id = session.workspace_id().to_string();

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "mai".to_string(),
            workspace_id: Some(workspace_id.clone()),
        }))
        .expect("resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    let deleted = match harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: session.id()[..8].to_string(),
            workspace_id: Some(workspace_id.clone()),
        }))
        .expect("delete should succeed")
    {
        LocalDaemonResponse::SessionDeleted { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
    assert_eq!(deleted.id(), session.id());
    assert_eq!(deleted.alias(), Some("main"));
    assert_eq!(deleted.status(), crate::session::SessionStatus::Ended);
    assert!(matches!(
        harness.dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "main".to_string(),
            workspace_id: Some(workspace_id),
        })),
        Err(DaemonError::SessionNotFound { .. })
    ));
    let listed = match harness
        .dispatch(LocalDaemonRequest::ListSessions(ListSessionsRequest))
        .expect("list should succeed")
    {
        LocalDaemonResponse::SessionsListed { sessions } => sessions,
        _ => panic!("unexpected local response"),
    };
    assert!(listed.is_empty());
}

#[test]
fn deleting_session_removes_agents_but_keeps_saved_provider_conversations() {
    for provider in ["codex", "claude", "opencode"] {
        let worktree = crate::test_support::TestWorktree::new("delete-saved-conversation");
        let harness = LocalRouterTestHarness::new();
        let (session, agent) = match harness
            .dispatch(LocalDaemonRequest::CreateSession(
                worktree.session_request(),
            ))
            .unwrap()
        {
            LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
            other => panic!("unexpected response {other:?}"),
        };
        let store = harness.with_app(|app| app.external_provider_session_index_store());
        let provider_session_id = format!("saved-{provider}");
        let external_session_id = format!("{provider}:default:{provider_session_id}");
        store.upsert(
            serde_json::from_value(serde_json::json!({
                "owner_user_id": crate::session::DEFAULT_LOCAL_USER_ID,
                "external_session_id": external_session_id,
                "provider": provider,
                "provider_session_id": provider_session_id,
                "title": "Saved conversation from deleted room",
                "last_modified_at_ms": 1,
                "account_profile": "default",
                "capabilities": {"can_read_history": true}
            }))
            .unwrap(),
        );
        store.mark_attached(&external_session_id, session.id(), agent.id());
        let list = || {
            store.list(&crate::local::ListExternalProviderSessionsRequest {
                provider: Some(provider.to_string()),
                cursor: None,
                limit: None,
            })
        };
        assert!(list().sessions.is_empty());
        harness
            .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: session.id().to_string(),
                workspace_id: None,
            }))
            .unwrap();
        harness.with_app(|app| {
            assert!(
                app.agents().get_agent(agent.id()).is_err(),
                "deleted room must not leave a live agent"
            );
            assert!(app.sessions().get_session(session.id()).is_err());
        });
        let saved = list();
        assert_eq!(saved.sessions.len(), 1);
        assert_eq!(saved.sessions[0].provider_session_id, provider_session_id);
        assert!(saved.sessions[0].attached_session_ids.is_empty());
        assert!(saved.sessions[0].attached_agent_ids.is_empty());
        assert!(!saved.sessions[0].attached_to_chariox);
    }
}

#[test]
fn local_request_api_manages_session_invites_and_members() {
    let worktree = crate::test_support::TestWorktree::new("session-control-invites");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let session_id = session.id().to_string();
    let invite_record = match harness
        .dispatch(LocalDaemonRequest::CreateSessionInvite(
            CreateSessionInviteRequest {
                session_id: session_id.clone(),
                expires_in_ms: None,
                max_uses: Some(1),
                collaboration_level: crate::session::CollaborationLevel::Private,
            },
        ))
        .expect("session invite create should succeed")
    {
        LocalDaemonResponse::SessionInviteCreated { invite, session } => {
            assert_eq!(session.id(), session_id);
            invite
        }
        _ => panic!("unexpected local response"),
    };

    let joined = match harness
        .dispatch(LocalDaemonRequest::JoinSessionInvite(
            JoinSessionInviteRequest {
                invite_token: invite_record.invite_token.clone(),
                user_id: "user-2".to_string(),
            },
        ))
        .expect("session invite join should succeed")
    {
        LocalDaemonResponse::SessionInviteJoined { member, session } => {
            assert!(session.has_member("user-2"));
            member
        }
        _ => panic!("unexpected local response"),
    };
    assert_eq!(joined.user_id(), "user-2");

    let (members, invites) = match harness
        .dispatch(LocalDaemonRequest::ListSessionMembers(
            ListSessionMembersRequest {
                session_id: session_id.clone(),
            },
        ))
        .expect("session members should list")
    {
        LocalDaemonResponse::SessionMembersListed { members, invites } => (members, invites),
        _ => panic!("unexpected local response"),
    };
    assert_eq!(members.len(), 2);
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].used_count(), 1);

    let revoked = match harness
        .dispatch(LocalDaemonRequest::RevokeSessionInvite(
            RevokeSessionInviteRequest {
                session_id,
                invite_ref: invite_record.invite.invite_id().to_string(),
            },
        ))
        .expect("session invite revoke should succeed")
    {
        LocalDaemonResponse::SessionInviteRevoked { invite, .. } => invite,
        _ => panic!("unexpected local response"),
    };
    assert!(revoked.is_revoked());
}

#[test]
fn local_request_api_aliases_sessions() {
    let worktree = crate::test_support::TestWorktree::new("session-control-aliases");
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let aliased = match harness
        .dispatch(LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session.id().to_string(),
            alias: "alpha".to_string(),
        }))
        .expect("alias should succeed")
    {
        LocalDaemonResponse::SessionAliased { session } => session,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(aliased.alias(), Some("alpha"));

    let resolved = match harness
        .dispatch(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "alpha".to_string(),
            workspace_id: Some(aliased.workspace_id().to_string()),
        }))
        .expect("alias resolve should succeed")
    {
        LocalDaemonResponse::SessionResolved { session } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(resolved.id(), session.id());
}

#[test]
fn local_request_api_spawns_and_focuses_agents() {
    let worktree = crate::test_support::TestWorktree::new("session-control-agent-focus");
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("  reviewer  ".to_string()),
            provider: Some("opencode".to_string()),
            model: Some("openai/gpt-5.4".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    assert_eq!(spawned.alias(), Some("reviewer"));

    let (session_state, agent_activity) = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            ..
        } => (session, agent_activity),
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.agents().len(), 2);
    assert_eq!(
        agent_activity
            .get(default_agent.id())
            .expect("default agent activity should be projected")
            .status,
        crate::runtime::projection::AgentRuntimeStatus::Idle
    );
    assert_eq!(
        agent_activity
            .get(spawned.id())
            .expect("spawned agent activity should be projected")
            .status,
        crate::runtime::projection::AgentRuntimeStatus::Idle
    );
    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state
            .agents()
            .iter()
            .map(|agent| agent.id())
            .collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == default_agent.id())
            .expect("default agent should still exist")
            .state(),
        crate::agent::AgentState::Idle
    );
    assert_eq!(
        session_state
            .agents()
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should exist")
            .state(),
        crate::agent::AgentState::Focused
    );

    let renamed = match harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
            alias: "web-reviewer".to_string(),
        }))
        .expect("agent alias update should succeed")
    {
        LocalDaemonResponse::AgentAliased { agent, session } => {
            assert_eq!(
                session
                    .agents()
                    .iter()
                    .find(|entry| entry.id() == spawned.id())
                    .and_then(|entry| entry.alias()),
                Some("web-reviewer")
            );
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(renamed.alias(), Some("web-reviewer"));

    let alias_conflict = harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
            alias: "  WEB-REVIEWER  ".to_string(),
        }))
        .expect_err("agent aliases must remain unique within a session");
    assert!(matches!(
        alias_conflict,
        DaemonError::AgentAliasConflict {
            session_id: ref conflict_session_id,
            alias: ref conflict_alias,
        } if conflict_session_id == session.id() && conflict_alias == "WEB-REVIEWER"
    ));
    let reference_conflict = harness
        .dispatch(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
            alias: spawned.agent_ref().to_string(),
        }))
        .expect_err("agent aliases must not shadow another agent reference");
    assert!(matches!(
        reference_conflict,
        DaemonError::AgentAliasConflict { .. }
    ));
    harness.with_app(|app| {
        assert_eq!(
            app.agents()
                .get_agent(default_agent.id())
                .expect("default agent should remain available")
                .alias(),
            default_agent.alias(),
        );
    });

    let profiled = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                provider: Some("codex".to_string()),
                account_profile: None,
                model: Some("gpt-5.4".to_string()),
                effort: Some("low".to_string()),
                clear_effort: false,
            },
        ))
        .expect("agent profile update should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.provider(), "codex");
            assert_eq!(entry.model(), Some("gpt-5.4"));
            assert_eq!(entry.effort(), Some("low"));
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(profiled.provider(), "codex");
    assert_eq!(profiled.model(), Some("gpt-5.4"));
    assert_eq!(profiled.effort(), Some("low"));

    let cleared = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                provider: None,
                account_profile: None,
                model: None,
                effort: None,
                clear_effort: true,
            },
        ))
        .expect("agent profile clear should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.effort(), None);
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cleared.provider(), "codex");
    assert_eq!(cleared.model(), Some("gpt-5.4"));
    assert_eq!(cleared.effort(), None);

    let relocated = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentConfig(
            UpdateAgentConfigRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
                execution_mode: None,
                clear_execution_mode: false,
                permission_level: None,
                clear_permission_level: false,
                workspace_id: Some("/repo/feature".to_string()),
                clear_workspace_id: false,
                worktree_id: Some("/repo/feature-wt".to_string()),
                clear_worktree_id: false,
            },
        ))
        .expect("agent workspace update should succeed")
    {
        LocalDaemonResponse::AgentConfigUpdated { agent, session } => {
            let entry = session
                .agents()
                .iter()
                .find(|entry| entry.id() == spawned.id())
                .expect("updated agent should remain in session snapshot");
            assert_eq!(entry.workspace_id(), Some("/repo/feature"));
            assert_eq!(entry.worktree_id(), Some("/repo/feature-wt"));
            agent
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(relocated.workspace_id(), Some("/repo/feature"));
    assert_eq!(relocated.worktree_id(), Some("/repo/feature-wt"));

    let focused_default = match harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should succeed")
    {
        LocalDaemonResponse::AgentFocused { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(focused_default.id(), default_agent.id());

    let cycled = match harness
        .dispatch(LocalDaemonRequest::CycleAgentFocus(
            CycleAgentFocusRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("cycle should succeed")
    {
        LocalDaemonResponse::AgentFocusCycled { agent } => {
            agent.expect("cycle should return a focused agent")
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(cycled.id(), spawned.id());

    let listed = match harness
        .dispatch(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().to_string(),
        }))
        .expect("list should succeed")
    {
        LocalDaemonResponse::AgentsListed { agents } => agents,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed.iter().map(|agent| agent.id()).collect::<Vec<_>>(),
        vec![default_agent.id(), spawned.id()]
    );
    assert_eq!(
        listed
            .iter()
            .find(|agent| agent.id() == spawned.id())
            .expect("spawned agent should be listed")
            .state(),
        crate::agent::AgentState::Focused
    );
}

#[test]
fn same_agent_profile_update_keeps_active_provider_run() {
    let worktree = crate::test_support::TestWorktree::new("session-control-active-provider");
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let active_run_id = harness.with_app_mut(|app| {
        crate::test_support::authenticate_provider_account(
            &app.provider_account_profile_registry(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            "default",
        )
        .expect("synthetic provider account should be authenticated");
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "codex", "default", "gpt-5.4")
                .with_agent_id(agent.id()),
        )
        .expect("provider launch should succeed")
        .id()
        .to_string()
    });

    let updated_session = match harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                provider: Some("codex".to_string()),
                account_profile: None,
                model: Some("gpt-5.4".to_string()),
                effort: None,
                clear_effort: false,
            },
        ))
        .expect("same profile update should succeed")
    {
        LocalDaemonResponse::AgentProfileUpdated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(
        updated_session.active_provider_run_id(),
        Some(active_run_id.as_str())
    );
    let run = harness.with_app(|app| {
        app.providers()
            .get_run(&active_run_id)
            .expect("active provider run should remain")
    });
    assert_eq!(run.state(), crate::provider::ProviderRunState::Running);
}

#[test]
fn detaching_one_attachment_keeps_the_session_open_for_others() {
    let worktree = crate::test_support::TestWorktree::new("session-control-detach");
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let detached = match harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: first.id().to_string(),
            },
        ))
        .expect("detach should succeed")
    {
        LocalDaemonResponse::SessionDetached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(detached.id(), first.id());
    assert_eq!(state.status().to_string(), "created");
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(second.id()).is_ok()));
}

#[test]
fn attaching_the_same_client_replaces_its_stale_attachment() {
    let worktree = crate::test_support::TestWorktree::new("session-control-reattach");
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    let first = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let second = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_ne!(first.id(), second.id());
    assert_eq!(state.attachment_ids().len(), 1);
    assert!(state.has_attachment(second.id()));
    assert!(harness.with_app(|app| app.attachments().get_attachment(first.id()).is_err()));
}

// MP-08 / MP-10: Project authorization without a session, agent or provider run.
#[test]
fn envp01_project_read_without_agents_denies_foreign_owner() {
    let harness = LocalRouterTestHarness::new();
    let project = crate::session::RuntimeProject::new(
        "envp01-no-agent",
        "local",
        "/plain/folder",
        "Plain",
        crate::session::RuntimeProjectKind::Named,
    );
    harness.with_app_mut(|app| app.sessions_mut().restore_projects(vec![project]));
    let request =
        LocalDaemonRequest::GetProjectEnvironment(crate::local::GetProjectEnvironmentRequest {
            project_id: "envp01-no-agent".into(),
        });
    let response = harness.dispatch_as_user("local", request.clone()).unwrap();
    match response {
        LocalDaemonResponse::ProjectEnvironment { environment } => {
            assert_eq!(environment.local_project_id, "envp01-no-agent");
            assert!(environment.observations.is_empty());
            assert!(environment.operations.is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(harness
        .dispatch_as_user("foreign", request)
        .unwrap_err()
        .to_string()
        .contains("does not own"));
    harness.with_app(|app| assert!(app.sessions().list_sessions().is_empty()));
    let root = harness.with_app(|app| app.config().private_runtime_state_root());
    harness
        .dispatch_as_user(
            "local",
            LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
                project_id: "envp01-no-agent".into(),
            }),
        )
        .unwrap();
    assert!(!root
        .join("project-environments")
        .read_dir()
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")));
}

#[test]
fn envp01_future_operations_are_known_but_unsupported_without_side_effects() {
    let harness = LocalRouterTestHarness::new();
    let target = serde_json::json!({"machine_id":"machine","target_instance_generation":"generation","slice_ref":null});
    let draft = serde_json::json!({"project_requirements":[],"folders":[]});
    let future = vec![
        serde_json::json!({"PreviewEnvironmentDiff":{"projectId":"project","expectedRevision":0,"draft":draft}}),
        serde_json::json!({"SaveProjectEnvironmentRevision":{"projectId":"project","expectedRevision":0,"expectedContentDigest":"digest","draft":draft,"acceptedProposalIds":[],"excludedProposalIds":[]}}),
        serde_json::json!({"PlanProjectEnvironment":{"projectId":"project","expectedRevision":0,"revisionDigest":"digest","target":target,"selectedItems":[]}}),
        serde_json::json!({"ApplyProjectEnvironment":{"projectId":"project","operationId":"op","planId":"plan","expectedRevision":0,"revisionDigest":"digest","target":target,"selectedItems":[],"perItemOptIns":[]}}),
        serde_json::json!({"CheckProjectEnvironment":{"projectId":"project","operationId":"op","revisionDigest":"digest","target":target,"selectedItems":[]}}),
        serde_json::json!({"GetEnvironmentOperation":{"projectId":"project","operationId":"op"}}),
        serde_json::json!({"CancelEnvironmentOperation":{"projectId":"project","operationId":"op"}}),
        serde_json::json!({"RetryEnvironmentOperation":{"projectId":"project","operationId":"op","expectedAttempt":1}}),
        serde_json::json!({"ExportProjectEnvironment":{"projectId":"project","operationId":"op","expectedRevision":0,"revisionDigest":"digest","selectedItems":[],"selectedFiles":[],"destination":{"kind":"file","path":"/should-never-be-written"}}}),
        serde_json::json!({"PreviewEnvironmentImport":{"artifactId":"absent","artifactDigest":"digest","choice":{"kind":"new"},"folderMap":[]}}),
        serde_json::json!({"CommitEnvironmentImport":{"operationId":"op","previewId":"absent","previewDigest":"digest","choice":{"kind":"new"}}}),
    ];
    for value in future {
        let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
        assert!(matches!(
            harness.dispatch(request).unwrap(),
            LocalDaemonResponse::EnvironmentUnsupportedFeature {
                supported_schema: 1,
                ..
            }
        ));
    }
    harness.with_app(|app| {
        assert!(app.sessions().list_sessions().is_empty());
        assert!(app.sessions().list_projects("local", true).is_empty());
        assert!(!app
            .config()
            .private_runtime_state_root()
            .join("project-environments")
            .exists());
    });
}

// MP-08 / MP-10 / MP-11: Detect is delivered; authorization precedes evidence or provider I/O.
#[test]
fn envp02a_hidden_utility_cleanup_preserves_standalone_project_environment() {
    let worktree = crate::test_support::TestWorktree::new("envp02a-utility-project");
    let harness = LocalRouterTestHarness::new();
    let project = crate::session::RuntimeProject::new(
        "utility-project",
        "local",
        worktree.path().to_string_lossy(),
        "Detect",
        crate::session::RuntimeProjectKind::Named,
    );
    harness.with_app_mut(|app| app.sessions_mut().restore_projects(vec![project]));
    let get =
        LocalDaemonRequest::GetProjectEnvironment(crate::local::GetProjectEnvironmentRequest {
            project_id: "utility-project".into(),
        });
    let before = harness.dispatch(get.clone()).unwrap();
    let created = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_hidden(true)
                .with_project_selection(SessionProjectSelection::Existing {
                    project_id: "utility-project".into(),
                }),
        ))
        .unwrap();
    let LocalDaemonResponse::SessionCreated { session, .. } = created else {
        panic!("utility session expected")
    };
    let runtime = harness.runtime_state();
    let mut foreign = session.clone();
    foreign.set_owner_user_id("foreign");
    assert!(harness
        .block_on_test_task(runtime.delete_environment_utility_session(&foreign))
        .unwrap_err()
        .to_string()
        .contains("binding mismatch"));
    harness
        .block_on_test_task(runtime.delete_environment_utility_session(&session))
        .unwrap();
    let after = harness
        .dispatch(get)
        .expect("read-only utility cleanup must retain the standalone Project");
    let (
        LocalDaemonResponse::ProjectEnvironment {
            environment: before,
        },
        LocalDaemonResponse::ProjectEnvironment { environment: after },
    ) = (before, after)
    else {
        panic!("Environment snapshots expected")
    };
    assert_eq!(before.lineage, after.lineage);
    assert_eq!(before.content_digest, after.content_digest);
    harness.with_app(|app| assert!(app.sessions().list_all_sessions().is_empty()));
    let LocalDaemonResponse::SessionCreated { session, .. } = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request().with_hidden(true),
        ))
        .unwrap()
    else {
        panic!("temporary hidden session expected")
    };
    harness
        .dispatch(LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
            project_id: "utility-project".into(),
        }))
        .unwrap();
    harness
        .block_on_test_task(runtime.delete_environment_utility_session(&session))
        .unwrap();
    harness.with_app(|app| assert!(app.sessions().list_all_sessions().is_empty()));
}

#[test]
fn envp02a_detect_rejects_foreign_owner_and_wrong_source_without_side_effects() {
    let worktree = crate::test_support::TestWorktree::new("envp02a-authorization");
    let harness = LocalRouterTestHarness::new();
    let project = crate::session::RuntimeProject::new(
        "detect-project",
        "local",
        worktree.path().to_string_lossy(),
        "Detect",
        crate::session::RuntimeProjectKind::Named,
    );
    harness.with_app_mut(|app| app.sessions_mut().restore_projects(vec![project]));
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "DetectProjectEnvironment": {"projectId":"detect-project","operationId":"op","folderIds":[],
        "target":{"machine_id":"wrong","target_instance_generation":"wrong","slice_ref":null},
        "provider":"codex","allowModelFolders":[]}
    }))
    .unwrap();
    assert!(harness
        .dispatch_as_user("foreign", request.clone())
        .unwrap_err()
        .to_string()
        .contains("does not own"));
    assert!(harness
        .dispatch_as_user("local", request)
        .unwrap_err()
        .to_string()
        .contains("source kernel"));
    harness.with_app(|app| {
        assert!(app.sessions().list_sessions().is_empty());
        assert!(!app
            .config()
            .private_runtime_state_root()
            .join("project-environments")
            .exists());
    });
}
