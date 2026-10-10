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
    let future = vec![
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
// MP-08 / MP-10 / MP-11: P02b fail-first shared revision/CAS seam.
fn envp02b_read(
    harness: &LocalRouterTestHarness,
) -> crate::project_environment::ProjectEnvironment {
    let response = harness
        .dispatch(LocalDaemonRequest::GetProjectEnvironment(
            crate::local::GetProjectEnvironmentRequest {
                project_id: "edit-project".into(),
            },
        ))
        .unwrap();
    let LocalDaemonResponse::ProjectEnvironment { environment } = response else {
        panic!("snapshot expected")
    };
    environment
}
fn envp02b_harness() -> LocalRouterTestHarness {
    let harness = LocalRouterTestHarness::new();
    harness.with_app_mut(|app| {
        app.sessions_mut()
            .restore_projects(vec![crate::session::RuntimeProject::new(
                "edit-project",
                "local",
                "/plain/folder",
                "Edit",
                crate::session::RuntimeProjectKind::Named,
            )])
    });
    harness
}
fn envp02b_save(
    environment: &crate::project_environment::ProjectEnvironment,
    title: &str,
) -> LocalDaemonRequest {
    serde_json::from_value(serde_json::json!({"SaveProjectEnvironmentRevision":{
        "projectId":"edit-project", "expectedRevision":environment.revision, "expectedContentDigest":environment.content_digest,
        "draft":{"project_requirements":[{"requirement_id":"user-node", "title":title,"scope":{"kind":"project"},"origins":environment.project_requirements.iter().find(|r|r.requirement_id=="user-node").map(|r|r.origins.clone()).unwrap_or_default(),"spec":{"kind":"software","identity":"node","version_constraint":">=22","platform":null,"install_scope":"project","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null}],
        "folders":environment.folders.iter().map(|f|serde_json::json!({"folder_id":f.folder_id,"portable_folder_key":f.portable_folder_key,"label":f.label,"optional_git":f.optional_git,"requirements":f.requirements})).collect::<Vec<_>>()},
        "acceptedProposalIds":[],"excludedProposalIds":[]}})).unwrap()
}
#[test]
fn envp02b_save_and_reopen_revision_preserves_first_edit_and_rejects_stale_writer() {
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    let LocalDaemonResponse::ProjectEnvironmentSaved {
        environment: saved,
        diff,
    } = harness.dispatch(envp02b_save(&before, "Editor A")).unwrap()
    else {
        panic!("P02b Save must be delivered")
    };
    assert_eq!(saved.revision, 1);
    assert_eq!(
        saved.parent_revision_digest.as_ref(),
        Some(&before.content_digest)
    );
    assert_eq!(diff.expected_revision, 0);
    assert_eq!(saved.reviewed_by.as_deref(), Some("local"));
    assert!(harness
        .dispatch(envp02b_save(&before, "Editor B"))
        .unwrap_err()
        .to_string()
        .contains("revision conflict"));
    assert_eq!(
        envp02b_read(&harness).project_requirements[0].title,
        "Editor A"
    );
    let LocalDaemonResponse::ProjectEnvironmentSaved {
        environment: second,
        ..
    } = harness
        .dispatch(envp02b_save(&saved, "Editor A + B"))
        .unwrap()
    else {
        panic!("save expected")
    };
    assert_eq!(second.revision, 2);
    let root = harness.with_app(|app| app.config().private_runtime_state_root());
    let project =
        harness.with_app(|app| app.sessions().get_project("edit-project").unwrap().clone());
    assert_eq!(
        crate::project_environment::ProjectEnvironmentStore::new(&root)
            .snapshot(&project)
            .unwrap(),
        second
    );
    harness
        .dispatch(LocalDaemonRequest::DeleteProject(DeleteProjectRequest {
            project_id: "edit-project".into(),
        }))
        .unwrap();
    assert!(!root
        .join("project-environments")
        .read_dir()
        .unwrap()
        .any(|e| e.unwrap().path().extension().is_some_and(|s| s == "json")));
}
// MP-08 / MP-10 / MP-11: concurrent editors, the TUI and Detect share one Project lock.
// A short read in progress must delay, not reject, another reader or a Save.
fn envp02b_hold_lock(
    harness: &LocalRouterTestHarness,
    held: std::time::Duration,
) -> std::thread::JoinHandle<()> {
    let root = harness.with_app(|app| app.config().private_runtime_state_root());
    let lock = crate::project_environment::ProjectEnvironmentStore::new(&root)
        .try_lock("edit-project")
        .unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(held);
        drop(lock);
    })
}
#[test]
fn envp02b_short_concurrent_read_delays_instead_of_rejecting_save_and_read() {
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    let reader = envp02b_hold_lock(&harness, std::time::Duration::from_millis(300));
    let saved = harness.dispatch(envp02b_save(&before, "Editor A"));
    reader.join().unwrap();
    let LocalDaemonResponse::ProjectEnvironmentSaved { environment, .. } =
        saved.expect("Save waits for a short concurrent read")
    else {
        panic!("save expected")
    };
    assert_eq!(environment.revision, 1);
    let reader = envp02b_hold_lock(&harness, std::time::Duration::from_millis(300));
    let read = envp02b_read(&harness);
    reader.join().unwrap();
    assert_eq!(read.revision, 1);
}
#[test]
fn envp02b_long_lock_holder_fails_save_as_busy_without_mutation() {
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    let detect = envp02b_hold_lock(&harness, std::time::Duration::from_secs(5));
    let error = harness
        .dispatch(envp02b_save(&before, "Editor A"))
        .unwrap_err()
        .to_string();
    detect.join().unwrap();
    assert!(error.contains("busy"), "{error}");
    assert_eq!(envp02b_read(&harness).revision, 0);
}
#[test]
fn envp02b_edits_authorize_owner_before_any_revision_mutation() {
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    assert!(harness
        .dispatch_as_user("foreign", envp02b_save(&before, "Foreign"))
        .unwrap_err()
        .to_string()
        .contains("does not own"));
    assert_eq!(before, envp02b_read(&harness));
}
#[test]
fn envp02b_invalid_edits_fail_atomically_and_unknown_proposals_cannot_be_accepted() {
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    for patch in [
        "scope",
        "duplicate",
        "folder",
        "proposal",
        "origin",
        "bounds",
    ] {
        let mut value = serde_json::to_value(envp02b_save(&before, "Valid")).unwrap();
        let request = &mut value["SaveProjectEnvironmentRevision"];
        match patch {
            "scope" => {
                request["draft"]["project_requirements"][0]["scope"] =
                    serde_json::json!({"kind":"folder","folder_id":"foreign"})
            }
            "duplicate" => {
                let r = request["draft"]["project_requirements"][0].clone();
                request["draft"]["project_requirements"]
                    .as_array_mut()
                    .unwrap()
                    .push(r);
            }
            "folder" => request["draft"]["folders"][0]["folder_id"] = serde_json::json!("foreign"),
            "proposal" => request["acceptedProposalIds"] = serde_json::json!(["unknown"]),
            "origin" => {
                request["draft"]["project_requirements"][0]["origins"] = serde_json::json!([{"kind":"detected","folder_id":"foreign","relative_path":"invented","line":1,"evidence_digest":"fake"}])
            }
            "bounds" => {
                request["draft"]["project_requirements"][0]["title"] =
                    serde_json::json!("x".repeat(1025))
            }
            _ => unreachable!(),
        }
        assert!(
            harness
                .dispatch(serde_json::from_value(value).unwrap())
                .is_err(),
            "must reject {patch}"
        );
        assert_eq!(before, envp02b_read(&harness));
    }
}
#[test]
fn envp02b_preview_is_read_only_and_stale_preview_shows_current_kernel_revision() {
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    let mut value = serde_json::to_value(envp02b_save(&before, "Editor B")).unwrap();
    harness.dispatch(envp02b_save(&before, "Editor A")).unwrap();
    let saved = envp02b_read(&harness);
    value["SaveProjectEnvironmentRevision"]["draft"]["project_requirements"][0]["origins"] =
        serde_json::to_value(&saved.project_requirements[0].origins).unwrap();
    let request=serde_json::from_value(serde_json::json!({"PreviewEnvironmentDiff":{"projectId":"edit-project","expectedRevision":0,"draft":value["SaveProjectEnvironmentRevision"]["draft"]}})).unwrap();
    let LocalDaemonResponse::ProjectEnvironmentDiff { diff } = harness.dispatch(request).unwrap()
    else {
        panic!("P02b Preview must be delivered")
    };
    assert_eq!(diff.expected_revision, saved.revision);
    assert_eq!(diff.source_digest, saved.content_digest);
    assert!(diff
        .requirements
        .iter()
        .any(|r| r.kind == crate::project_environment::EnvironmentDiffKind::Changed));
    assert_eq!(saved, envp02b_read(&harness));
}

#[test]
fn envp02b_accept_exclude_choices_survive_changed_detect_and_immutable_history() {
    use crate::project_environment::*;
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    let root = harness.with_app(|app| app.config().private_runtime_state_root());
    let store = ProjectEnvironmentStore::new(&root);
    let mut cache:EnvironmentDetectionCache=serde_json::from_value(serde_json::json!({
        "project_id":"edit-project","evidence_digest":"a".repeat(64),"proposals":(["accept","exclude"].iter().map(|id|serde_json::json!({"proposal_id":format!("proposal-{id}"),"requirement":{"requirement_id":id,"title":id,"scope":{"kind":"folder","folder_id":before.folders[0].folder_id},"origins":[{"kind":"detected_metadata","source":"test-metadata","reference":"safe"}],"spec":{"kind":"software","identity":id,"version_constraint":null,"platform":null,"install_scope":"project","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":false,"legacy_entry":null}})).collect::<Vec<_>>()),
        "operation":{"operation_id":"detect-test","attempt":1,"local_project_id":"edit-project","revision_digest":before.content_digest,"target":{"machine_id":"machine","target_instance_generation":"generation","slice_ref":null},"kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],"created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}}})).unwrap();
    store.save_detection(&cache).unwrap();
    let detected = envp02b_read(&harness);
    let request=serde_json::from_value(serde_json::json!({"SaveProjectEnvironmentRevision":{"projectId":"edit-project","expectedRevision":0,"expectedContentDigest":detected.content_digest,"draft":environment_draft_for_test(&detected),"acceptedProposalIds":[detected.proposals.iter().find(|p|p.requirement.requirement_id=="accept").unwrap().proposal_id],"excludedProposalIds":[detected.proposals.iter().find(|p|p.requirement.requirement_id=="exclude").unwrap().proposal_id]}})).unwrap();
    let LocalDaemonResponse::ProjectEnvironmentSaved {
        environment: saved,
        diff,
    } = harness.dispatch(request).unwrap()
    else {
        panic!("proposal review Save expected")
    };
    assert!(saved.proposals.is_empty());
    assert!(
        diff.requirements
            .iter()
            .any(|row| row.requirement_id == "accept" && row.kind == EnvironmentDiffKind::Added),
        "accepted proposal must be in review diff even when proposal ID differs"
    );
    assert_eq!(saved.folders[0].requirements[0].requirement_id, "accept");
    assert!(saved.folders[0].requirements[0].required);
    let history_path = root
        .join("project-environments")
        .read_dir()
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("revisions.json")
        })
        .expect("immutable history");
    let history: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&history_path).unwrap()).unwrap();
    cache.evidence_digest = "b".repeat(64);
    cache.proposals[0].requirement.title = "Changed evidence".into();
    store.save_detection(&cache).unwrap();
    let refreshed = envp02b_read(&harness);
    assert!(refreshed.proposals.is_empty());
    assert_eq!(refreshed.content_digest, saved.content_digest);
    assert_eq!(refreshed.folders[0].requirements[0].title, "accept");
    let mut draft = environment_draft_for_test(&refreshed);
    draft["folders"][0]["requirements"][0]["title"] = serde_json::json!("Reviewed title");
    let request=serde_json::from_value(serde_json::json!({"SaveProjectEnvironmentRevision":{"projectId":"edit-project","expectedRevision":saved.revision,"expectedContentDigest":saved.content_digest,"draft":draft,"acceptedProposalIds":[],"excludedProposalIds":[]}})).unwrap();
    harness.dispatch(request).unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&history_path).unwrap()).unwrap();
    assert_eq!(after["revisions"][0], history["revisions"][0]);
    assert_eq!(after["revisions"][1], history["revisions"][1]);
}
fn environment_draft_for_test(
    e: &crate::project_environment::ProjectEnvironment,
) -> serde_json::Value {
    serde_json::json!({"project_requirements":e.project_requirements,"folders":e.folders.iter().map(|f|serde_json::json!({"folder_id":f.folder_id,"portable_folder_key":f.portable_folder_key,"label":f.label,"optional_git":f.optional_git,"requirements":f.requirements})).collect::<Vec<_>>()})
}

// MP-08 / MP-10 / MP-11: first Save retains old recipe bytes and records only its digest.
#[test]
fn envp02b_first_save_records_legacy_digests_without_rewriting_recipe_or_values() {
    let harness = envp02b_harness();
    let definition:crate::session::ProjectEnvironmentDefinition=serde_json::from_value(serde_json::json!({"schema_version":1,"origin":"user_authored","source":"commands","target_platform":"linux-x86_64","source_path":null,"setup_steps":[{"kind":"command","command":"echo private-source-only"}],"validation_commands":["true"]})).unwrap();
    harness.with_app_mut(|app| {
        let mut project = app.sessions().get_project("edit-project").unwrap().clone();
        project.set_environment_definition(definition.clone());
        app.sessions_mut().restore_projects(vec![project]);
    });
    let before = envp02b_read(&harness);
    let root = harness.with_app(|app| app.config().private_runtime_state_root());
    let request=serde_json::from_value(serde_json::json!({"SaveProjectEnvironmentRevision":{"projectId":"edit-project","expectedRevision":0,"expectedContentDigest":before.content_digest,"draft":environment_draft_for_test(&before),"acceptedProposalIds":[],"excludedProposalIds":[]}})).unwrap();
    let LocalDaemonResponse::ProjectEnvironmentSaved { environment, .. } =
        harness.dispatch(request).unwrap()
    else {
        panic!("first Save expected")
    };
    assert_eq!(environment.revision, 1);
    assert_eq!(
        harness.with_app(|app| app
            .sessions()
            .get_project("edit-project")
            .unwrap()
            .environment_definition()
            .cloned()),
        Some(definition.clone())
    );
    let history = root
        .join("project-environments")
        .read_dir()
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("revisions.json")
        })
        .unwrap();
    let bytes = std::fs::read(history).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        stored["legacy_digests"],
        serde_json::json!([definition.digest()])
    );
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("private-source-only"));
}

// MP-08 / MP-10 / MP-11: a newer Detect must never authorize an unseen proposal.
#[test]
fn envp02b_stale_proposal_review_is_rejected_without_a_new_protocol_field() {
    use crate::project_environment::*;
    let harness = envp02b_harness();
    let before = envp02b_read(&harness);
    let root = harness.with_app(|app| app.config().private_runtime_state_root());
    let store = ProjectEnvironmentStore::new(&root);
    let mut cache:EnvironmentDetectionCache=serde_json::from_value(serde_json::json!({
        "project_id":"edit-project","evidence_digest":"a".repeat(64),"proposals":(["accept","exclude"].iter().map(|id|serde_json::json!({"proposal_id":format!("proposal-{id}"),"requirement":{"requirement_id":id,"title":id,"scope":{"kind":"folder","folder_id":before.folders[0].folder_id},"origins":[{"kind":"detected_metadata","source":"test-metadata","reference":"safe"}],"spec":{"kind":"software","identity":id,"version_constraint":null,"platform":null,"install_scope":"project","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":false,"legacy_entry":null}})).collect::<Vec<_>>()),
        "operation":{"operation_id":"detect-test","attempt":1,"local_project_id":"edit-project","revision_digest":before.content_digest,"target":{"machine_id":"machine","target_instance_generation":"generation","slice_ref":null},"kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],"created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}}})).unwrap();
    store.save_detection(&cache).unwrap();
    let viewed = envp02b_read(&harness);
    let viewed_id = viewed.proposals[0].proposal_id.clone();
    cache.proposals[0].requirement.title = "New unseen detection".into();
    cache.evidence_digest = "c".repeat(64);
    store.save_detection(&cache).unwrap();
    let current = envp02b_read(&harness);
    let request=serde_json::from_value(serde_json::json!({"SaveProjectEnvironmentRevision":{"projectId":"edit-project","expectedRevision":viewed.revision,"expectedContentDigest":viewed.content_digest,"draft":environment_draft_for_test(&viewed),"acceptedProposalIds":[viewed_id],"excludedProposalIds":[]}})).unwrap();
    assert!(
        harness.dispatch(request).is_err(),
        "must reject unseen changed proposal"
    );
    assert_eq!(current, envp02b_read(&harness));
}

// MP-08 / MP-10 / MP-11: normalized response order must match immutable content identity.
#[test]
fn envp02b_saved_digest_matches_returned_folder_order() {
    let harness = envp02b_harness();
    harness.with_app_mut(|app| {
        let mut project = app.sessions().get_project("edit-project").unwrap().clone();
        project.replace_workspace_ids(vec!["/plain/one".into(), "/plain/two".into()]);
        app.sessions_mut().restore_projects(vec![project]);
    });
    let before = envp02b_read(&harness);
    let mut request = serde_json::to_value(envp02b_save(&before, "Reviewed")).unwrap();
    request["SaveProjectEnvironmentRevision"]["draft"]["folders"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let LocalDaemonResponse::ProjectEnvironmentSaved {
        environment: saved, ..
    } = harness
        .dispatch(serde_json::from_value(request).unwrap())
        .unwrap()
    else {
        panic!("Save expected")
    };
    let request=serde_json::from_value(serde_json::json!({"PreviewEnvironmentDiff":{"projectId":"edit-project","expectedRevision":saved.revision,"draft":environment_draft_for_test(&saved)}})).unwrap();
    let LocalDaemonResponse::ProjectEnvironmentDiff { diff } = harness.dispatch(request).unwrap()
    else {
        panic!("Preview expected")
    };
    assert_eq!(
        saved.content_digest, diff.target_digest,
        "saved digest must represent returned canonical folder order"
    );
}

// MP-08 / MP-10 / MP-11: topology affects local bindings, never saved specification bytes.
#[test]
fn envp02b_saved_revision_survives_workspace_add_remove_and_reorder() {
    for (initial, workspaces) in [
        (vec!["/plain/folder"], vec!["/plain/folder", "/plain/other"]),
        (vec!["/plain/folder", "/plain/other"], vec!["/plain/other"]),
        (
            vec!["/plain/folder", "/plain/other"],
            vec!["/plain/other", "/plain/folder"],
        ),
    ] {
        let harness = envp02b_harness();
        harness.dispatch(serde_json::from_value(serde_json::json!({"UpdateProjectWorkspaces":{"project_id":"edit-project","workspace_ids":initial}})).unwrap()).unwrap();
        let before = envp02b_read(&harness);
        let LocalDaemonResponse::ProjectEnvironmentSaved {
            environment: saved, ..
        } = harness.dispatch(envp02b_save(&before, "Saved")).unwrap()
        else {
            panic!("Save expected")
        };
        harness.dispatch(serde_json::from_value(serde_json::json!({"UpdateProjectWorkspaces":{"project_id":"edit-project","workspace_ids":workspaces}})).unwrap()).unwrap();
        let current = envp02b_read(&harness);
        assert_eq!(
            environment_draft_for_test(&current),
            environment_draft_for_test(&saved),
            "workspace mutation must not alter saved specification"
        );
        assert_eq!(current.revision, saved.revision);
        assert_eq!(current.content_digest, saved.content_digest);
        for (folder, original) in current.folders.iter().zip(&saved.folders) {
            assert_eq!(
                folder.local_workspace_binding.as_str(),
                if workspaces.contains(&original.local_workspace_binding.as_str()) {
                    original.local_workspace_binding.as_str()
                } else {
                    ""
                }
            );
        }
        let LocalDaemonResponse::ProjectEnvironmentDiff { diff } = harness.dispatch(serde_json::from_value(serde_json::json!({"PreviewEnvironmentDiff":{"projectId":"edit-project","expectedRevision":current.revision,"draft":environment_draft_for_test(&current)}})).unwrap()).unwrap() else { panic!("Preview expected") };
        assert_eq!(diff.target_digest, current.content_digest);
    }
}

// MP-08 / MP-10 / MP-11: a queued save validates the Project it actually locks.
#[test]
fn envp02b_detect_uses_attached_folders_after_saved_folder_detach() {
    for select_attached in [false, true] {
        let a = crate::test_support::TestWorktree::new("envp02b-detect-attached");
        let b = crate::test_support::TestWorktree::new("envp02b-detect-detached");
        let harness = envp02b_harness();
        let paths = [
            a.path().to_string_lossy().into_owned(),
            b.path().to_string_lossy().into_owned(),
        ];
        let update = |paths: &[String]| {
            serde_json::from_value(serde_json::json!({"UpdateProjectWorkspaces":{"project_id":"edit-project","workspace_ids":paths}})).unwrap()
        };
        harness.dispatch(update(&paths)).unwrap();
        let before = envp02b_read(&harness);
        harness.dispatch(envp02b_save(&before, "Saved")).unwrap();
        harness.dispatch(update(&paths[..1])).unwrap();
        let saved = envp02b_read(&harness);
        let attached = &before.folders[0].folder_id;
        let detached = &before.folders[1].folder_id;
        let target = harness.with_app(|app| serde_json::json!({"machine_id":app.config().host_machine_id,"target_instance_generation":app.config().daemon_id,"slice_ref":null}));
        let detect = |folders: Vec<&str>| {
            serde_json::from_value(serde_json::json!({"DetectProjectEnvironment":{"projectId":"edit-project","operationId":"attached-detect","folderIds":folders,"target":target,"provider":"codex","allowModelFolders":[]}})).unwrap()
        };
        let response = harness
            .dispatch(detect(if select_attached {
                vec![attached]
            } else {
                vec![]
            }))
            .expect("Detect must not preflight a detached historical folder");
        let LocalDaemonResponse::ProjectEnvironment { environment } = response else {
            panic!("Environment expected")
        };
        assert_eq!(
            environment.operations[0].selected_items,
            vec![attached.clone()]
        );
        assert_eq!(
            environment.operations[0].phase,
            crate::project_environment::EnvironmentOperationPhase::Ready
        );
        assert_eq!(
            environment_draft_for_test(&environment),
            environment_draft_for_test(&saved)
        );
        assert_eq!(environment.content_digest, saved.content_digest);
        assert!(harness
            .dispatch(detect(vec![detached]))
            .unwrap_err()
            .to_string()
            .contains("folder does not belong"));
    }
}

// MP-08 / MP-10 / MP-11: a queued save validates the Project it actually locks.
#[test]
fn envp02b_waiting_save_reloads_topology_at_zero_and_saved_revision() {
    for save_first in [false, true] {
        let harness = envp02b_harness();
        let initial = envp02b_read(&harness);
        if save_first {
            harness.dispatch(envp02b_save(&initial, "Saved")).unwrap();
        }
        let before = envp02b_read(&harness);
        let mut value = serde_json::to_value(envp02b_save(&before, "Queued")).unwrap();
        value["SaveProjectEnvironmentRevision"]["draft"]["folders"][0]["label"] =
            serde_json::json!("Detached edit");
        let LocalDaemonRequest::SaveProjectEnvironmentRevision(request) =
            serde_json::from_value(value).unwrap()
        else {
            panic!("Save request")
        };
        let runtime = harness.runtime_state();
        let holder = envp02b_hold_lock(&harness, std::time::Duration::from_millis(400));
        let task = harness.spawn_test_task(async move {
            runtime
                .save_project_environment_revision(request, "local")
                .await
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        // Fixture mutation represents a topology update completed before Save owns the lock.
        harness.with_app_mut(|app| {
            let mut project = app.sessions().get_project("edit-project").unwrap().clone();
            project.replace_workspace_ids(vec!["/plain/other".into()]);
            app.sessions_mut().restore_projects(vec![project]);
        });
        holder.join().unwrap();
        assert!(
            harness.block_on_test_task(task).unwrap().is_err(),
            "queued save must reject detached folder edits"
        );
        assert_eq!(envp02b_read(&harness).revision, before.revision);
    }
}

// MP-08 / MP-10 / MP-11: user topology mutation shares save admission.
#[test]
fn envp02b_workspace_update_waits_for_environment_lock() {
    let harness = envp02b_harness();
    let holder = envp02b_hold_lock(&harness, std::time::Duration::from_millis(400));
    let runtime = harness.runtime_state();
    let task = harness.spawn_test_task(async move {
        runtime
            .update_project_workspaces("edit-project", vec!["/plain/other".into()], "local")
            .await
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !task.is_finished(),
        "workspace mutation must wait for save admission"
    );
    holder.join().unwrap();
    harness.block_on_test_task(task).unwrap().unwrap();
}

// MP-08 / MP-10 / MP-11: accepting a detected item explicitly reviews a new live folder.
#[test]
fn envp02b_accept_detected_requirement_from_new_attachment_preserves_history() {
    let a = crate::test_support::TestWorktree::new("envp02b-attach-review-a");
    let b = crate::test_support::TestWorktree::new("envp02b-attach-review-b");
    std::fs::write(b.path().join(".env.example"), "PUBLIC_SAMPLE=example\n").unwrap();
    let harness = envp02b_harness();
    let paths = [
        a.path().to_string_lossy().into_owned(),
        b.path().to_string_lossy().into_owned(),
    ];
    let update = |paths: &[String]| {
        serde_json::from_value(serde_json::json!({"UpdateProjectWorkspaces":{"project_id":"edit-project","workspace_ids":paths}})).unwrap()
    };
    harness.dispatch(update(&paths[..1])).unwrap();
    let initial = envp02b_read(&harness);
    harness.dispatch(envp02b_save(&initial, "Saved A")).unwrap();
    let saved_a = envp02b_read(&harness);
    harness.dispatch(update(&paths)).unwrap();
    let target = harness.with_app(|app| serde_json::json!({"machine_id":app.config().host_machine_id,"target_instance_generation":app.config().daemon_id,"slice_ref":null}));
    let LocalDaemonResponse::ProjectEnvironment {environment:detected} = harness.dispatch(serde_json::from_value(serde_json::json!({"DetectProjectEnvironment":{"projectId":"edit-project","operationId":"attach-b-detect","folderIds":[],"target":target,"provider":"codex","allowModelFolders":[]}})).unwrap()).unwrap() else { panic!("Detect expected") };
    assert_eq!(
        environment_draft_for_test(&detected),
        environment_draft_for_test(&saved_a)
    );
    let proposal = detected.proposals.iter().find(|p| matches!(&p.requirement.scope,crate::project_environment::RequirementScope::Folder {folder_id} if folder_id != &saved_a.folders[0].folder_id)).expect("new B must have a static proposal");
    let mut request = serde_json::to_value(envp02b_save(&detected, "Saved A")).unwrap();
    request["SaveProjectEnvironmentRevision"]["acceptedProposalIds"] =
        serde_json::json!([proposal.proposal_id]);
    let LocalDaemonResponse::ProjectEnvironmentSaved {
        environment: reviewed,
        ..
    } = harness
        .dispatch(serde_json::from_value(request).unwrap())
        .expect("accepting a current B proposal must create a reviewed B folder")
    else {
        panic!("Save expected")
    };
    assert_eq!(reviewed.revision, saved_a.revision + 1);
    assert_eq!(reviewed.folders.len(), 2);
    assert!(reviewed
        .folders
        .iter()
        .flat_map(|f| &f.requirements)
        .any(|r| r.requirement_id == proposal.requirement.requirement_id));
    assert_eq!(
        reviewed.parent_revision_digest.as_deref(),
        Some(saved_a.content_digest.as_str())
    );
    assert_eq!(envp02b_read(&harness).folders, reviewed.folders);
}

