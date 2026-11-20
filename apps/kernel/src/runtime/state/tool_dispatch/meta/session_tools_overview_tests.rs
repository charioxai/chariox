//! MP-08 / MP-10 / MP-11: room overview is a public MCP projection.

#[tokio::test]
async fn room_overview_excludes_transport_and_provider_private_state_in_both_mcp_representations() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "overview-workspace",
            "overview-worktree",
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            actor.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "overview-worker".into(),
                worker_machine_id: "overview-machine".into(),
                execution_lease_id: "overview-lease".into(),
                leased_agent_id: "overview-leased-agent".into(),
                active_worker_provider_run_id: Some("overview-worker-run".into()),
                relay_url: Some("wss://relay.example.test".into()),
                relay_token: Some("synthetic-overview-relay-canary".into()),
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("private-peer")
                .with_owner_user_id("other-owner"),
        )
        .unwrap();
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let state =
        crate::runtime::router::CommandRouter::with_interactive_capacity(app, 1).runtime_state();
    let result = state
        .meta_session_overview(&session, &actor, Default::default())
        .unwrap();
    let (content, structured) =
        crate::transport::mcp_server::runtime_tool_content_for_test(result.payload);
    for representation in [
        serde_json::to_string(&content).unwrap(),
        structured.to_string(),
    ] {
        assert!(
            !representation.contains("synthetic-overview-relay-canary"),
            "MP-11 transport credential exposed"
        );
        assert!(
            !representation.contains("relay_token"),
            "MP-11 relay credential field exposed"
        );
        assert!(
            !representation.contains("provider_resume_state"),
            "MP-11 provider-private state exposed"
        );
        assert!(
            !representation.contains(peer.id()),
            "MP-11 private collaborator agent exposed"
        );
    }
    let public = &structured["agents"][0];
    assert_eq!(public["id"], actor.id());
    assert_eq!(
        public["remote_execution"]["worker_kernel_id"],
        "overview-worker"
    );
    assert_eq!(
        public["remote_execution"]["relay_url"],
        "wss://relay.example.test"
    );
    assert_eq!(
        public["remote_execution"]["active_worker_provider_run_id"],
        "overview-worker-run"
    );
}

// MP-08 / MP-10 / MP-11: retained runs must not inherit authority from a smaller roster.
#[tokio::test]
async fn room_overview_historical_workflow_visibility_survives_foreign_agent_deletion() {
    use crate::local::{
        GetSessionStateRequest, GetWorkflowRunRequest, KernelConnectionClass,
        ListWorkflowRunsRequest, LocalDaemonRequest, LocalDaemonResponse,
    };
    use crate::runtime::command::{KernelCommand, KernelCommandSource};
    use crate::session::{
        CollaborationLevel, WorkflowMessage, WorkflowNodeRun, WorkflowNodeRunStatus,
        WorkflowOutputPayload, WorkflowRun, WorkflowRunStatus,
    };
    for level in [
        CollaborationLevel::Private,
        CollaborationLevel::Transparent,
        CollaborationLevel::Full,
    ] {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, actor) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "overview-history-workspace",
                "overview-history-worktree",
            ))
            .unwrap();
        let foreign = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id("foreign-history-owner"),
            )
            .unwrap();
        let foreign_worker = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id("foreign-history-owner"),
            )
            .unwrap();
        let retired_owned = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id(actor.owner_user_id()),
            )
            .unwrap();
        // MP-11: workflow cleanup keys deletion events by session, not agent.
        // Several retired copies ensure provenance is not the latest deletion alone.
        let runtime_worktree = app
            .config()
            .workflow_runtime_artifact_root()
            .join("instances")
            .join(session.id())
            .join("retained-history");
        let runtime_owned = (0..3)
            .map(|_| {
                app.agents.materialize_workflow_runtime_agent(
                    actor.clone(),
                    session.id(),
                    &runtime_worktree.to_string_lossy(),
                )
            })
            .collect::<Vec<_>>();
        let runtime_foreign = app.agents.materialize_workflow_runtime_agent(
            foreign.clone(),
            session.id(),
            &runtime_worktree.to_string_lossy(),
        );
        let runtime_owned_run = WorkflowRun::new(
            "runtime-owned-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-authorized-runtime-history".into()),
            None,
            runtime_owned
                .iter()
                .enumerate()
                .map(|(index, worker)| {
                    WorkflowNodeRun::new(
                        format!("runtime-owned-node-{index}"),
                        format!("node-{index}"),
                        worker.id(),
                        0,
                        WorkflowNodeRunStatus::Completed,
                    )
                })
                .collect(),
            vec![],
        )
        .with_creator(Some(actor.id().into()));
        let runtime_mixed_run = WorkflowRun::new(
            "runtime-mixed-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-foreign-history-runtime-worker".into()),
            None,
            vec![WorkflowNodeRun::new(
                "runtime-foreign-node",
                "foreign-node",
                runtime_foreign.id(),
                0,
                WorkflowNodeRunStatus::Completed,
            )],
            vec![],
        )
        .with_creator(Some(actor.id().into()));
        let mut node = WorkflowNodeRun::new(
            "foreign-node-run",
            "foreign-node",
            foreign_worker.id(),
            0,
            WorkflowNodeRunStatus::Completed,
        );
        node.add_thinking_trace("synthetic-foreign-history-trace");
        let mut foreign_run = WorkflowRun::new(
            "foreign-history-run",
            "foreign-workflow",
            "foreign-endpoint",
            "foreign-node",
            Some("synthetic-foreign-history-invocation".into()),
            None,
            vec![node],
            vec![WorkflowMessage::new(
                "foreign-message",
                Some("foreign-node-run".into()),
                "foreign-node",
                "handoff",
                "synthetic-foreign-history-summary",
                "synthetic-foreign-history-handoff",
            )],
        )
        .with_creator(Some(foreign.id().into()));
        foreign_run.set_status(WorkflowRunStatus::Completed);
        foreign_run.set_final_output(
            Some(WorkflowOutputPayload::new(
                "synthetic-foreign-history-output",
                vec![],
            )),
            Some(true),
            None,
            None,
        );
        let mut owned_run = WorkflowRun::new(
            "owned-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-authorized-history-invocation".into()),
            None,
            vec![],
            vec![],
        )
        .with_creator(Some(actor.id().into()));
        owned_run.set_status(WorkflowRunStatus::Completed);
        let retired_run = WorkflowRun::new(
            "retired-owned-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-authorized-retired-history".into()),
            None,
            vec![WorkflowNodeRun::new(
                "owned-node-run",
                "owned-node",
                retired_owned.id(),
                0,
                WorkflowNodeRunStatus::Completed,
            )],
            vec![],
        )
        .with_creator(Some(retired_owned.id().into()));
        let mixed_run = WorkflowRun::new(
            "mixed-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-foreign-history-mixed-worker".into()),
            None,
            vec![WorkflowNodeRun::new(
                "mixed-node-run",
                "mixed-node",
                foreign_worker.id(),
                0,
                WorkflowNodeRunStatus::Completed,
            )],
            vec![],
        )
        .with_creator(Some(actor.id().into()));
        let unknown_run = WorkflowRun::new(
            "unknown-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-unknown-history".into()),
            None,
            vec![],
            vec![],
        )
        .with_creator(Some("missing-history-agent".into()));
        let no_creator_run = WorkflowRun::new(
            "no-creator-history-run",
            "owned-workflow",
            "owned-endpoint",
            "owned-node",
            Some("synthetic-no-creator-history".into()),
            None,
            vec![],
            vec![],
        );
        crate::app::KernelSessionService::new(&mut app)
            .destroy_agent(foreign.id())
            .unwrap();
        crate::app::KernelSessionService::new(&mut app)
            .destroy_agent(foreign_worker.id())
            .unwrap();
        crate::app::KernelSessionService::new(&mut app)
            .destroy_agent(retired_owned.id())
            .unwrap();
        assert!(app.agents.get_agent(foreign.id()).is_err());
        assert!(app.agents.get_agent(foreign_worker.id()).is_err());
        assert!(app.agents.get_agent(retired_owned.id()).is_err());
        let mut serialized =
            serde_json::to_value(app.sessions().get_session(session.id()).unwrap()).unwrap();
        serialized["workflow_runs"] = serde_json::json!([
            foreign_run,
            owned_run,
            retired_run,
            mixed_run,
            unknown_run,
            no_creator_run,
            runtime_owned_run,
            runtime_mixed_run
        ]);
        for member in serialized["members"].as_array_mut().unwrap() {
            if member["user_id"] == actor.owner_user_id() {
                member["collaboration_level"] = serde_json::to_value(level).unwrap();
            }
        }
        app.sessions_mut()
            .restore_session(serde_json::from_value(serialized.clone()).unwrap());
        let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app, 1);
        let state = router.runtime_state();
        state
            .owned
            .workflow_cleanup_runtime_instances_exclusive(session.id())
            .unwrap();
        for worker in runtime_owned
            .iter()
            .chain(std::iter::once(&runtime_foreign))
        {
            assert!(state.owned.agent_store.get_agent(worker.id()).is_err());
        }
        // Cleanup archives terminal runs; reconstruct the retained-history view
        // after exercising its real deletion producer.
        let mut retained =
            serde_json::to_value(state.owned.session_store.get_session(session.id()).unwrap())
                .unwrap();
        retained["workflow_runs"] = serialized["workflow_runs"].clone();
        state
            .owned
            .session_store
            .write()
            .restore_session(serde_json::from_value(retained).unwrap());
        // MP-08 / MP-10 / MP-11: public protocol and subscription projection share Room's policy.
        let retained_session = state.owned.session_snapshot(session.id()).unwrap();
        // Cover full projections from cold and warmed lists and exact/prefix resolution.
        let read_projection = crate::runtime::projection::SessionStateProjectionStore::default();
        let pending =
            crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker::default();
        let mut pre_lane_responses = Vec::new();
        for request in [
            crate::local::LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest),
            crate::local::LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest),
            crate::local::LocalDaemonRequest::ResolveSession(crate::local::ResolveSessionRequest {
                session_ref: session.id().into(),
                workspace_id: None,
            }),
            crate::local::LocalDaemonRequest::ResolveSession(crate::local::ResolveSessionRequest {
                session_ref: session.id()[..8].into(),
                workspace_id: Some(session.workspace_id().into()),
            }),
        ] {
            pre_lane_responses.push(
                crate::runtime::session_read_control::projected_session_read_response(
                    &state,
                    &read_projection,
                    &state.owned.provider_run_projection,
                    &pending,
                    &request,
                    actor.owner_user_id(),
                )
                .await
                .expect("production pre-lane read must handle the request")
                .unwrap(),
            );
        }
        let room_state = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session.id().into(),
        });
        let lean = router
            .dispatch(
                KernelCommand::from_local_request("room-state", None, None, &room_state),
                room_state,
            )
            .await
            .expect("Room member can read lean state");
        let LocalDaemonResponse::SessionState {
            session: lean_room, ..
        } = lean
        else {
            panic!("expected lean Room state");
        };
        assert_eq!(lean_room.id(), session.id());
        assert!(
            lean_room.workflow_runs().is_empty(),
            "lean state omits embedded runs"
        );

        let list = LocalDaemonRequest::ListWorkflowRuns(ListWorkflowRunsRequest {
            session_id: session.id().into(),
            workflow_ref: None,
            cursor: None,
            limit: None,
        });
        let get = LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
            session_id: session.id().into(),
            workflow_run_ref: "retired-owned-history-run".into(),
        });
        let listed = router
            .dispatch(
                KernelCommand::from_local_request("list-history", None, None, &list),
                list.clone(),
            )
            .await
            .expect("owner can list historical workflow runs");
        let LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } = listed else {
            panic!("expected workflow run list");
        };
        assert!(workflow_runs
            .iter()
            .any(|run| run.id() == "retired-owned-history-run"));
        let got = router
            .dispatch(
                KernelCommand::from_local_request("get-history", None, None, &get),
                get.clone(),
            )
            .await
            .expect("owner can read the retired agent's run");
        let LocalDaemonResponse::WorkflowRun { workflow_run } = got else {
            panic!("expected historical workflow run");
        };
        assert_eq!(workflow_run.id(), "retired-owned-history-run");
        assert_eq!(
            workflow_run.invocation_prompt(),
            Some("synthetic-authorized-retired-history")
        );
        for request in [list, get] {
            let mut command = KernelCommand::from_local_request_with_source(
                "outsider-history",
                KernelCommandSource::RelayClient,
                None,
                None,
                &request,
            );
            command.caller.user_id = Some("outside-history-reader".into());
            command.caller.client_id = Some("outside-history-client".into());
            command.caller.connection_class = Some(KernelConnectionClass::Terminal);
            let result = router.dispatch(command, request).await;
            assert!(
                matches!(
                    &result,
                    Err(crate::error::DaemonError::SessionAccessDenied { .. })
                ),
                "a non-member must be refused the Room's workflow history: {result:?}"
            );
        }

        let projected = retained_session
            .clone()
            .redacted_for_user_with_workflow_history(
                actor.owner_user_id(),
                &state.owned.durable_state_store,
            )
            .unwrap();
        let attachment_projection = crate::runtime::projection::SessionSnapshotProjection {
            jobs: Vec::new(),
            processes: Vec::new(),
            metadata: crate::runtime::projection::ProjectionMetadata::new(
                crate::runtime::projection::SESSION_SNAPSHOT_PROJECTION_VERSION,
                0,
            ),
            session: retained_session.clone(),
            provider_run: None,
            agent_activity: Default::default(),
            agent_tree_revs: Default::default(),
            room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
                &retained_session,
                actor.owner_user_id(),
            ),
        }
        .redacted_for_user(actor.owner_user_id(), &state.owned.durable_state_store)
        .unwrap();
        for value in [
            serde_json::to_value(projected).unwrap(),
            serde_json::to_value(attachment_projection).unwrap(),
        ]
        .into_iter()
        .chain(
            pre_lane_responses
                .into_iter()
                .map(|response| serde_json::to_value(response).unwrap()),
        ) {
            let text = value.to_string();
            assert!(text.contains("synthetic-authorized-retired-history"));
            assert!(text.contains("synthetic-authorized-runtime-history"));
            assert_eq!(
                text.contains("synthetic-foreign-history-"),
                level.can_view_agent_trace()
            );
        }
        let result = state
            .meta_session_overview(&session, &actor, Default::default())
            .unwrap();
        let (content, structured) =
            crate::transport::mcp_server::runtime_tool_content_for_test(result.payload);
        for representation in [
            serde_json::to_string(&content).unwrap(),
            structured.to_string(),
        ] {
            assert!(
                representation.contains("synthetic-authorized-history-invocation"),
                "MP-11 authorized history lost at {level:?}"
            );
            assert!(
                representation.contains("synthetic-authorized-retired-history"),
                "MP-11 authorized deleted-agent history lost at {level:?}"
            );
            assert!(
                representation.contains("synthetic-authorized-runtime-history"),
                "MP-11 authorized workflow-cleanup history lost at {level:?}"
            );
            assert_eq!(
                representation.contains("synthetic-foreign-history-"),
                level.can_view_agent_trace(),
                "MP-11 deleted foreign workflow history visibility at {level:?}"
            );
            assert!(
                representation.contains("synthetic-no-creator-history"),
                "MP-11 kernel owner can read unproven history at {level:?}"
            );
            {
                let canary = "synthetic-unknown-history";
                assert_eq!(
                    representation.contains(canary),
                    level.can_view_agent_trace(),
                    "MP-11 unproven history visibility at {level:?}"
                );
            }
        }
        assert_eq!(
            structured["workflow_runs"].as_array().unwrap().len(),
            if level.can_view_agent_trace() { 8 } else { 4 }
        );
    }
}
