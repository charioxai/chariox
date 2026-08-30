use super::*;
use crate::session::CanonicalViewport;

#[test]
fn room_environment_takeover_and_release_use_authenticated_actor_and_room_lane() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-takeover",
                "worktree-environment-takeover",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("managed runtime should become ready");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
            RequestRoomEnvironmentInputTakeoverRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        ))
        .expect("authenticated Room member should take desktop input");
    let LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
        outcome,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(outcome, crate::session::TakeoverOutcome::Granted);
    assert_eq!(environment.input_ownership.len(), 1);
    assert_eq!(
        environment.input_ownership[0].actor_id,
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        environment.input_ownership[0].target,
        crate::session::InputTarget::Desktop
    );

    let response = harness
        .dispatch(LocalDaemonRequest::ReleaseRoomEnvironmentInput(
            ReleaseRoomEnvironmentInputRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        ))
        .expect("authenticated Room member should release desktop input");
    let LocalDaemonResponse::RoomEnvironmentInputReleased { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert!(environment.input_ownership.is_empty());
}

#[test]
fn room_environment_reconciles_human_and_agent_presence() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-actors",
                "worktree-environment-actors",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = started else {
        panic!("unexpected local response: {started:?}");
    };
    let human_actor_id =
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID);
    let default_agent_actor_id = crate::session::agent_environment_actor_id(default_agent.id());
    assert!(environment.actors.iter().any(|actor| {
        actor.actor_id == human_actor_id
            && actor.display_label == "Local user"
            && actor.presence == crate::session::EnvironmentActorPresence::Present
    }));
    assert!(environment.actors.iter().any(|actor| {
        actor.actor_id == default_agent_actor_id
            && actor.kind == crate::session::EnvironmentActorKind::Agent
            && actor.presence == crate::session::EnvironmentActorPresence::Present
    }));

    let attachment_one = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "environment-client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attachment should join")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment_two = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "environment-client-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attachment should join")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment_one.id().to_string(),
            },
        ))
        .expect("first attachment should leave");
    let after_first_detach = room_environment_state(&harness, session.id());
    assert_eq!(
        actor_presence(&after_first_detach, &human_actor_id),
        crate::session::EnvironmentActorPresence::Present
    );

    harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment_two.id().to_string(),
            },
        ))
        .expect("second attachment should leave");
    let after_last_detach = room_environment_state(&harness, session.id());
    assert_eq!(
        actor_presence(&after_last_detach, &human_actor_id),
        crate::session::EnvironmentActorPresence::Disconnected
    );

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("Navigator".to_string()),
            provider: Some("dev-stub".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected local response: {other:?}"),
    };
    let spawned_actor_id = crate::session::agent_environment_actor_id(spawned.id());
    let after_spawn = room_environment_state(&harness, session.id());
    let spawned_actor = after_spawn
        .actors
        .iter()
        .find(|actor| actor.actor_id == spawned_actor_id)
        .expect("spawned agent should have an Environment Actor");
    assert_eq!(spawned_actor.display_label, "Navigator");
    assert_eq!(
        spawned_actor.presence,
        crate::session::EnvironmentActorPresence::Present
    );

    harness
        .dispatch(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            agent_id: spawned.id().to_string(),
        }))
        .expect("agent should be destroyed");
    let after_destroy = room_environment_state(&harness, session.id());
    assert_eq!(
        actor_presence(&after_destroy, &spawned_actor_id),
        crate::session::EnvironmentActorPresence::Disconnected
    );
}

fn room_environment_state(
    harness: &LocalRouterTestHarness,
    session_id: &str,
) -> crate::session::RoomEnvironmentSnapshot {
    match harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentState(
            GetRoomEnvironmentStateRequest {
                session_id: session_id.to_string(),
            },
        ))
        .expect("Room Environment state should be readable")
    {
        LocalDaemonResponse::RoomEnvironmentState { environment } => environment,
        other => panic!("unexpected local response: {other:?}"),
    }
}

fn actor_presence(
    environment: &crate::session::RoomEnvironmentSnapshot,
    actor_id: &str,
) -> crate::session::EnvironmentActorPresence {
    environment
        .actors
        .iter()
        .find(|actor| actor.actor_id == actor_id)
        .unwrap_or_else(|| panic!("missing Environment Actor `{actor_id}`"))
        .presence
}

#[test]
fn room_environment_viewport_update_uses_authenticated_actor_and_revision() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-viewport",
                "worktree-environment-viewport",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("managed runtime should become ready");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentViewport(
            UpdateRoomEnvironmentViewportRequest {
                session_id: session.id().to_string(),
                expected_revision: 1,
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1440,
                    css_height: 900,
                    device_scale_factor: 2,
                    desktop_pixel_width: 2880,
                    desktop_pixel_height: 1800,
                },
            },
        ))
        .expect("authenticated Room member should update the canonical viewport");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    let expected_actor_id =
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID);
    assert_eq!(environment.viewport.css_width, 1440);
    assert_eq!(environment.viewport.revision, 2);
    assert_eq!(
        environment.viewport.last_actor_id.as_deref(),
        Some(expected_actor_id.as_str())
    );
    assert!(environment.actors.iter().any(|actor| {
        actor.actor_id == expected_actor_id
            && actor.kind == crate::session::EnvironmentActorKind::Human
            && actor.display_label == "Local user"
    }));

    let error = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentViewport(
            UpdateRoomEnvironmentViewportRequest {
                session_id: session.id().to_string(),
                expected_revision: 1,
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1600,
                    css_height: 1000,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1600,
                    desktop_pixel_height: 1000,
                },
            },
        ))
        .expect_err("a stale viewport revision must fail");
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: "environment.viewport.update",
            ..
        }
    ));
}

#[test]
fn room_environment_start_rejects_invalid_initial_viewport_with_stable_code() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-invalid-viewport",
                "worktree-environment-invalid-viewport",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 0,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect_err("an initial zero-width viewport must be rejected");
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "environment.start");
            assert!(message.starts_with("environment_invalid_viewport:"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn room_environment_start_crosses_the_router_boundary_without_duplication() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-start", "worktree-environment-start"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let request = StartRoomEnvironmentRequest {
        session_id: session.id().to_string(),
        viewport: RoomEnvironmentViewportRequest {
            css_width: 1280,
            css_height: 800,
            device_scale_factor: 2,
            desktop_pixel_width: 2560,
            desktop_pixel_height: 1600,
        },
    };

    let first = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(request.clone()))
        .expect("Room Environment should start through the router");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = first else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(environment.session_id, session.id());
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
    assert_eq!(environment.runtime_generation, 1);
    assert_eq!(environment.viewport.css_width, 1280);

    let second = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(request))
        .expect("repeating start should be idempotent");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated,
    } = second
    else {
        panic!("unexpected local response: {second:?}");
    };
    assert_eq!(repeated.environment_id, environment.environment_id);
    assert_eq!(repeated.runtime_generation, environment.runtime_generation);
    assert_eq!(repeated.event_cursor, environment.event_cursor);

    let repeated_without_viewport = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 0,
                    css_height: 0,
                    device_scale_factor: 0,
                    desktop_pixel_width: 0,
                    desktop_pixel_height: 0,
                },
            },
        ))
        .expect("an existing Environment should keep its canonical viewport");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated_without_viewport,
    } = repeated_without_viewport
    else {
        panic!("unexpected local response: {repeated_without_viewport:?}");
    };
    assert_eq!(repeated_without_viewport, repeated);
}

#[test]
fn room_environment_stop_preserves_identity_and_is_idempotent() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-stop", "worktree-environment-stop"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: started,
    } = started
    else {
        panic!("unexpected local response: {started:?}");
    };

    let first = harness
        .dispatch(LocalDaemonRequest::StopRoomEnvironment(
            StopRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("Room Environment should stop");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = first else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(environment.environment_id, started.environment_id);
    assert_eq!(environment.runtime_generation, started.runtime_generation);
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Stopped
    );

    let second = harness
        .dispatch(LocalDaemonRequest::StopRoomEnvironment(
            StopRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("repeating stop should be idempotent");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated,
    } = second
    else {
        panic!("unexpected local response: {second:?}");
    };
    assert_eq!(repeated, environment);

    let restarted = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("a stopped Room Environment should restart");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: restarted,
    } = restarted
    else {
        panic!("unexpected local response: {restarted:?}");
    };
    assert_eq!(restarted.environment_id, environment.environment_id);
    assert_eq!(
        restarted.runtime_generation,
        environment.runtime_generation + 1
    );
    assert_eq!(
        restarted.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
}

#[test]
fn room_environment_retry_invalidates_failed_runtime_without_replacing_environment() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-retry", "worktree-environment-retry"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: started,
    } = started
    else {
        panic!("unexpected local response: {started:?}");
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Failed)
            .expect("managed runtime failure should be recorded");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::RetryRoomEnvironment(
            RetryRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("failed Room Environment should retry");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(environment.environment_id, started.environment_id);
    assert_eq!(
        environment.runtime_generation,
        started.runtime_generation + 1
    );
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
}

#[test]
fn room_environment_state_crosses_the_router_boundary() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment", "worktree-environment"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .create_room_environment(
                session.id(),
                "environment-1",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room should acquire an Environment");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentState(
            GetRoomEnvironmentStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("Room Environment should be projected through the router");
    let LocalDaemonResponse::RoomEnvironmentState { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(environment.session_id, session.id());
    assert_eq!(environment.environment_id, "environment-1");
}

#[test]
fn room_environment_event_replay_crosses_the_router_boundary() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-events",
                "worktree-environment-events",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        let state = app.session_state_store();
        state
            .create_room_environment(
                session.id(),
                "environment-1",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room should acquire an Environment");
        state
            .start_room_environment(
                session.id(),
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room Environment should start");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentEvents(
            GetRoomEnvironmentEventsRequest {
                session_id: session.id().to_string(),
                cursor: 0,
            },
        ))
        .expect("Room Environment events should be projected through the router");
    assert!(matches!(
        response,
        LocalDaemonResponse::RoomEnvironmentEvents {
            replay: crate::session::EnvironmentReplay::Events {
                events,
                next_cursor,
            }
        } if !events.is_empty()
            && events.windows(2).all(|pair| pair[0].event_id + 1 == pair[1].event_id)
            && next_cursor == events.last().unwrap().event_id
    ));

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentEvents(
            GetRoomEnvironmentEventsRequest {
                session_id: session.id().to_string(),
                cursor: u64::MAX,
            },
        ))
        .expect("a replay gap should return the authoritative Room Environment snapshot");
    assert!(matches!(
        response,
        LocalDaemonResponse::RoomEnvironmentEvents {
            replay: crate::session::EnvironmentReplay::SnapshotRequired { snapshot }
        } if snapshot.session_id == session.id()
            && snapshot.environment_id == "environment-1"
    ));
}

#[test]
fn room_environment_state_requires_room_membership() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-auth",
                "worktree-environment-auth",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
                session_id: session.id().to_string(),
            }),
        )
        .expect_err("an outsider must not read the Room Environment");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::GetRoomEnvironmentEvents(GetRoomEnvironmentEventsRequest {
                session_id: session.id().to_string(),
                cursor: 0,
            }),
        )
        .expect_err("an outsider must not replay Room Environment events");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
}

#[test]
fn room_environment_lifecycle_requires_room_membership() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-start-auth",
                "worktree-environment-start-auth",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect_err("an outsider must not start the Room Environment");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));

    for request in [
        LocalDaemonRequest::StopRoomEnvironment(StopRoomEnvironmentRequest {
            session_id: session.id().to_string(),
        }),
        LocalDaemonRequest::RetryRoomEnvironment(RetryRoomEnvironmentRequest {
            session_id: session.id().to_string(),
        }),
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
            RequestRoomEnvironmentInputTakeoverRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        ),
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
            session_id: session.id().to_string(),
            target: crate::session::InputTarget::Desktop,
        }),
    ] {
        let error = harness
            .dispatch_as_user("outsider-1", request)
            .expect_err("an outsider must not control the Room Environment lifecycle");
        assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
    }
}
