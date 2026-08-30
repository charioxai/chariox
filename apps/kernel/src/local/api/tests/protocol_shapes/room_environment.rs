use super::*;
use crate::session::{
    CanonicalViewport, EnvironmentAction, EnvironmentActionState, EnvironmentActor,
    EnvironmentActorKind, EnvironmentActorPresence, EnvironmentComponent,
    EnvironmentComponentHealth, EnvironmentComponentHealthState, EnvironmentEvent,
    EnvironmentEventKind, EnvironmentLifecycle, EnvironmentMode, EnvironmentReplay, EnvironmentTab,
    InputOwnership, InputTarget, RoomEnvironmentSnapshot,
};

#[test]
fn room_environment_state_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request = LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
        session_id: "session-1".to_string(),
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment state request should encode"),
        serde_json::json!({
            "GetRoomEnvironmentState": {
                "session_id": "session-1"
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetRoomEnvironmentState": {
                "session_id": "session-1"
            }
        }))
        .expect("Room Environment state request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentState {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Stopped,
            health: vec![EnvironmentComponentHealth {
                component: EnvironmentComponent::BrowserController,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            }],
            viewport: CanonicalViewport::new(1280, 800, 2, 2560, 1600)
                .expect("viewport should be valid"),
            actors: vec![EnvironmentActor {
                actor_id: "agent-1".to_string(),
                kind: EnvironmentActorKind::Agent,
                display_label: "Browser agent".to_string(),
                presence: EnvironmentActorPresence::Present,
            }],
            tabs: vec![EnvironmentTab {
                tab_id: "tab-1".to_string(),
                url: "https://example.test/".to_string(),
                title: "Example".to_string(),
                document_revision: 3,
                focused: true,
            }],
            focused_tab_id: Some("tab-1".to_string()),
            actions: vec![
                EnvironmentAction {
                    action_id: "action-1".to_string(),
                    sequence: 1,
                    idempotency_key: Some("idempotency-1".to_string()),
                    actor_id: "agent-1".to_string(),
                    runtime_generation: 1,
                    mode: EnvironmentMode::Browser,
                    kind: "click".to_string(),
                    targets: vec![
                        InputTarget::Desktop,
                        InputTarget::BrowserTab("tab-1".to_string()),
                    ],
                    state: EnvironmentActionState::Running,
                    cancellation_requested: true,
                },
                EnvironmentAction {
                    action_id: "action-2".to_string(),
                    sequence: 2,
                    idempotency_key: None,
                    actor_id: "agent-1".to_string(),
                    runtime_generation: 1,
                    mode: EnvironmentMode::Browser,
                    kind: "second-click".to_string(),
                    targets: vec![InputTarget::BrowserTab("tab-1".to_string())],
                    state: EnvironmentActionState::Queued,
                    cancellation_requested: false,
                },
            ],
            input_ownership: vec![InputOwnership {
                target: InputTarget::Desktop,
                actor_id: "agent-1".to_string(),
            }],
            pending_input_takeovers: Vec::new(),
            event_cursor: 0,
        },
    };
    assert_eq!(
        serde_json::to_value(&response).expect("Room Environment state response should encode"),
        serde_json::json!({
            "RoomEnvironmentState": {
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-1",
                    "runtime_generation": 1,
                    "lifecycle": "stopped",
                    "health": [{
                        "component": "browser_controller",
                        "state": "unavailable",
                        "diagnostic_code": null
                    }],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 2,
                        "desktop_pixel_width": 2560,
                        "desktop_pixel_height": 1600,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [{
                        "actor_id": "agent-1",
                        "kind": "agent",
                        "display_label": "Browser agent",
                        "presence": "present"
                    }],
                    "tabs": [{
                        "tab_id": "tab-1",
                        "url": "https://example.test/",
                        "title": "Example",
                        "document_revision": 3,
                        "focused": true
                    }],
                    "focused_tab_id": "tab-1",
                    "actions": [
                        {
                            "action_id": "action-1",
                            "sequence": 1,
                            "idempotency_key": "idempotency-1",
                            "actor_id": "agent-1",
                            "runtime_generation": 1,
                            "mode": "browser",
                            "kind": "click",
                            "targets": [
                                {
                                    "kind": "desktop"
                                },
                                {
                                    "kind": "browser_tab",
                                    "id": "tab-1"
                                }
                            ],
                            "state": "running",
                            "cancellation_requested": true
                        },
                        {
                            "action_id": "action-2",
                            "sequence": 2,
                            "idempotency_key": null,
                            "actor_id": "agent-1",
                            "runtime_generation": 1,
                            "mode": "browser",
                            "kind": "second-click",
                            "targets": [{
                                "kind": "browser_tab",
                                "id": "tab-1"
                            }],
                            "state": "queued",
                            "cancellation_requested": false
                        }
                    ],
                    "input_ownership": [{
                        "target": {
                            "kind": "desktop"
                        },
                        "actor_id": "agent-1"
                    }],
                    "pending_input_takeovers": [],
                    "event_cursor": 0
                }
            }
        })
    );

    let mut previous_protocol_value =
        serde_json::to_value(&response).expect("Room Environment response should encode");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment snapshot should be an object")
        .remove("pending_input_takeovers");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment/actions/0")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment Action should be an object")
        .remove("sequence");
    previous_protocol_value
        .pointer_mut("/RoomEnvironmentState/environment/actions/0")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Room Environment Action should be an object")
        .remove("cancellation_requested");
    let LocalDaemonResponse::RoomEnvironmentState { environment } =
        serde_json::from_value(previous_protocol_value)
            .expect("pre-v272 snapshots should decode with no pending takeovers")
    else {
        panic!("expected Room Environment state response");
    };
    assert!(environment.pending_input_takeovers.is_empty());
    assert_eq!(environment.actions[0].sequence, 0);
    assert!(!environment.actions[0].cancellation_requested);
}

#[test]
fn room_environment_event_replay_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request = LocalDaemonRequest::GetRoomEnvironmentEvents(GetRoomEnvironmentEventsRequest {
        session_id: "session-1".to_string(),
        cursor: 41,
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment event request should encode"),
        serde_json::json!({
            "GetRoomEnvironmentEvents": {
                "session_id": "session-1",
                "cursor": 41,
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetRoomEnvironmentEvents": {
                "session_id": "session-1",
                "cursor": 41,
            }
        }))
        .expect("Room Environment event request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentEvents {
        replay: EnvironmentReplay::Events {
            events: vec![EnvironmentEvent {
                event_id: 42,
                environment_id: "environment-1".to_string(),
                runtime_generation: 3,
                kind: EnvironmentEventKind::ViewportChanged { revision: 7 },
            }],
            next_cursor: 42,
        },
    };
    let value = serde_json::json!({
        "RoomEnvironmentEvents": {
            "replay": {
                "Events": {
                    "events": [{
                        "event_id": 42,
                        "environment_id": "environment-1",
                        "runtime_generation": 3,
                        "kind": {
                            "ViewportChanged": {
                                "revision": 7,
                            }
                        }
                    }],
                    "next_cursor": 42,
                }
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&response).expect("Room Environment event replay should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(value)
            .expect("Room Environment event replay should decode"),
        response
    );
    assert_eq!(
        serde_json::from_value::<EnvironmentEventKind>(serde_json::json!({
            "ActionChanged": {
                "action_id": "action-1",
                "state": "running",
            }
        }))
        .expect("pre-v276 Action events should decode"),
        EnvironmentEventKind::ActionChanged {
            action_id: "action-1".to_string(),
            state: EnvironmentActionState::Running,
            cancellation_requested: false,
        }
    );
}

#[test]
fn room_environment_start_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request = LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
        session_id: "session-1".to_string(),
        viewport: RoomEnvironmentViewportRequest {
            css_width: 1280,
            css_height: 800,
            device_scale_factor: 2,
            desktop_pixel_width: 2560,
            desktop_pixel_height: 1600,
        },
    });
    let value = serde_json::json!({
        "StartRoomEnvironment": {
            "session_id": "session-1",
            "viewport": {
                "css_width": 1280,
                "css_height": 800,
                "device_scale_factor": 2,
                "desktop_pixel_width": 2560,
                "desktop_pixel_height": 1600
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment start request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("Room Environment start request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Starting,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 2, 2560, 1600)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 1,
        },
    };
    assert_eq!(
        serde_json::to_value(response).expect("Room Environment start response should encode"),
        serde_json::json!({
            "RoomEnvironmentUpdated": {
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-session-1",
                    "runtime_generation": 1,
                    "lifecycle": "starting",
                    "health": [],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 2,
                        "desktop_pixel_width": 2560,
                        "desktop_pixel_height": 1600,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [],
                    "tabs": [],
                    "focused_tab_id": null,
                    "actions": [],
                    "input_ownership": [],
                    "pending_input_takeovers": [],
                    "event_cursor": 1
                }
            }
        })
    );
}

#[test]
fn room_environment_stop_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request = LocalDaemonRequest::StopRoomEnvironment(StopRoomEnvironmentRequest {
        session_id: "session-1".to_string(),
    });
    let value = serde_json::json!({
        "StopRoomEnvironment": {
            "session_id": "session-1"
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment stop request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("Room Environment stop request should decode"),
        request
    );
}

#[test]
fn room_environment_retry_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request = LocalDaemonRequest::RetryRoomEnvironment(RetryRoomEnvironmentRequest {
        session_id: "session-1".to_string(),
    });
    let value = serde_json::json!({
        "RetryRoomEnvironment": {
            "session_id": "session-1"
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("Room Environment retry request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("Room Environment retry request should decode"),
        request
    );
}

#[test]
fn room_environment_viewport_update_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request =
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(UpdateRoomEnvironmentViewportRequest {
            session_id: "session-1".to_string(),
            expected_revision: 4,
            viewport: RoomEnvironmentViewportRequest {
                css_width: 1440,
                css_height: 900,
                device_scale_factor: 2,
                desktop_pixel_width: 2880,
                desktop_pixel_height: 1800,
            },
        });
    let value = serde_json::json!({
        "UpdateRoomEnvironmentViewport": {
            "session_id": "session-1",
            "expected_revision": 4,
            "viewport": {
                "css_width": 1440,
                "css_height": 900,
                "device_scale_factor": 2,
                "desktop_pixel_width": 2880,
                "desktop_pixel_height": 1800
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("viewport request should encode"),
        value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(value)
            .expect("viewport request should decode"),
        request
    );
}

#[test]
fn room_environment_takeover_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request = LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
        RequestRoomEnvironmentInputTakeoverRequest {
            session_id: "session-1".to_string(),
            target: InputTarget::Desktop,
        },
    );
    let request_value = serde_json::json!({
        "RequestRoomEnvironmentInputTakeover": {
            "session_id": "session-1",
            "target": {
                "kind": "desktop"
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("takeover request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("takeover request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
        outcome: crate::session::TakeoverOutcome::Granted,
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 2,
        },
    };
    assert_eq!(
        serde_json::to_value(response).expect("takeover response should encode"),
        serde_json::json!({
            "RoomEnvironmentTakeoverUpdated": {
                "outcome": {
                    "state": "granted"
                },
                "environment": {
                    "session_id": "session-1",
                    "environment_id": "environment-session-1",
                    "runtime_generation": 1,
                    "lifecycle": "ready",
                    "health": [],
                    "viewport": {
                        "css_width": 1280,
                        "css_height": 800,
                        "device_scale_factor": 1,
                        "desktop_pixel_width": 1280,
                        "desktop_pixel_height": 800,
                        "revision": 1,
                        "last_actor_id": null
                    },
                    "actors": [],
                    "tabs": [],
                    "focused_tab_id": null,
                    "actions": [],
                    "input_ownership": [],
                    "pending_input_takeovers": [],
                    "event_cursor": 2
                }
            }
        })
    );
}

#[test]
fn room_environment_input_release_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 276);

    let request =
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
            session_id: "session-1".to_string(),
            target: InputTarget::Desktop,
        });
    let request_value = serde_json::json!({
        "ReleaseRoomEnvironmentInput": {
            "session_id": "session-1",
            "target": {
                "kind": "desktop"
            }
        }
    });
    assert_eq!(
        serde_json::to_value(&request).expect("input release request should encode"),
        request_value
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_value)
            .expect("input release request should decode"),
        request
    );

    let response = LocalDaemonResponse::RoomEnvironmentInputReleased {
        environment: RoomEnvironmentSnapshot {
            session_id: "session-1".to_string(),
            environment_id: "environment-session-1".to_string(),
            runtime_generation: 1,
            lifecycle: EnvironmentLifecycle::Ready,
            health: Vec::new(),
            viewport: CanonicalViewport::new(1280, 800, 1, 1280, 800)
                .expect("viewport should be valid"),
            actors: Vec::new(),
            tabs: Vec::new(),
            focused_tab_id: None,
            actions: Vec::new(),
            input_ownership: Vec::new(),
            pending_input_takeovers: Vec::new(),
            event_cursor: 3,
        },
    };
    let response_value =
        serde_json::to_value(&response).expect("input release response should encode");
    assert_eq!(
        response_value.pointer("/RoomEnvironmentInputReleased/environment/event_cursor"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_value)
            .expect("input release response should decode"),
        response
    );
}
