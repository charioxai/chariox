use super::*;

#[test]
fn lifecycle_preserves_identity_and_reset_invalidates_runtime_handles() {
    let viewport = CanonicalViewport::new(1440, 900, 2, 2880, 1800).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();

    assert_eq!(
        environment.snapshot().lifecycle,
        EnvironmentLifecycle::Stopped
    );
    assert_eq!(environment.snapshot().runtime_generation, 1);

    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopping)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopped)
        .unwrap();

    assert_eq!(environment.snapshot().environment_id, "environment-1");
    assert_eq!(environment.snapshot().runtime_generation, 1);

    environment.reset_runtime().unwrap();
    assert_eq!(environment.snapshot().environment_id, "environment-1");
    assert_eq!(environment.snapshot().runtime_generation, 2);
    assert_eq!(
        environment.snapshot().lifecycle,
        EnvironmentLifecycle::Starting
    );
}

#[test]
fn lifecycle_rejects_unsafe_transitions_and_invalid_viewports() {
    assert_eq!(
        CanonicalViewport::new(0, 900, 1, 1440, 900),
        Err(EnvironmentError::InvalidViewport)
    );

    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();
    assert_eq!(
        environment.transition_to(EnvironmentLifecycle::Ready),
        Err(EnvironmentError::InvalidLifecycleTransition {
            from: EnvironmentLifecycle::Stopped,
            to: EnvironmentLifecycle::Ready,
        })
    );
    assert_eq!(
        environment.transition_to(EnvironmentLifecycle::Starting),
        Err(EnvironmentError::InvalidLifecycleTransition {
            from: EnvironmentLifecycle::Stopped,
            to: EnvironmentLifecycle::Starting,
        })
    );
}

#[test]
fn tab_identity_survives_reconciliation_and_navigation_invalidates_old_references() {
    let mut environment = ready_environment();
    let tab_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    assert_eq!(tab_id, "tab-1");

    let reconciled_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    assert_eq!(reconciled_id, tab_id);
    assert_eq!(environment.snapshot().tabs.len(), 1);

    environment
        .validate_tab_reference(1, &tab_id, 1)
        .expect("initial reference should be current");
    environment
        .record_navigation(&tab_id, "https://example.test/inbox", "Inbox")
        .unwrap();
    assert_eq!(environment.snapshot().tabs[0].document_revision, 2);
    assert_eq!(
        environment.validate_tab_reference(1, &tab_id, 1),
        Err(EnvironmentError::StaleDocumentRevision {
            tab_id: "tab-1".to_string(),
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn controller_tab_reconciliation_preserves_identity_and_tracks_documents_and_focus() {
    let mut environment = ready_environment();
    environment.reconcile_controller_tabs(
        vec![
            observed_tab("target-a", "loader-a1", "https://a.test", "A"),
            observed_tab("target-b", "loader-b1", "https://b.test", "B"),
        ],
        Some("target-b"),
    );
    let initial = environment.snapshot();
    assert_eq!(
        initial
            .tabs
            .iter()
            .map(|tab| (tab.tab_id.as_str(), tab.document_revision, tab.focused))
            .collect::<Vec<_>>(),
        vec![("tab-1", 1, false), ("tab-2", 1, true)]
    );

    environment.reconcile_controller_tabs(
        vec![
            observed_tab("target-b", "loader-b2", "https://b.test/inbox", "Inbox"),
            observed_tab("target-a", "loader-a1", "https://a.test", "A renamed"),
            observed_tab("target-c", "loader-c1", "https://c.test", "C"),
        ],
        Some("target-a"),
    );
    let reconciled = environment.snapshot();
    assert_eq!(
        reconciled
            .tabs
            .iter()
            .map(|tab| (
                tab.tab_id.as_str(),
                tab.url.as_str(),
                tab.title.as_str(),
                tab.document_revision,
                tab.focused,
            ))
            .collect::<Vec<_>>(),
        vec![
            ("tab-1", "https://a.test", "A renamed", 1, true),
            ("tab-2", "https://b.test/inbox", "Inbox", 2, false),
            ("tab-3", "https://c.test", "C", 1, false),
        ]
    );
}

#[test]
fn controller_tab_reconciliation_retires_missing_targets_and_detects_same_url_reload() {
    let mut environment = ready_environment();
    environment.reconcile_controller_tabs(
        vec![
            observed_tab("target-a", "loader-a1", "https://a.test", "A"),
            observed_tab("target-b", "loader-b1", "https://b.test", "B"),
        ],
        Some("target-a"),
    );
    environment.reconcile_controller_tabs(
        vec![observed_tab(
            "target-b",
            "loader-b2",
            "https://b.test",
            "B reloaded",
        )],
        Some("missing-target"),
    );

    let snapshot = environment.snapshot();
    assert_eq!(snapshot.tabs.len(), 1);
    assert_eq!(snapshot.tabs[0].tab_id, "tab-2");
    assert_eq!(snapshot.tabs[0].document_revision, 2);
    assert!(snapshot.tabs[0].focused);
    assert_eq!(snapshot.focused_tab_id.as_deref(), Some("tab-2"));
}

#[test]
fn element_references_are_opaque_stable_within_a_document_and_stale_after_navigation() {
    let mut environment = ready_environment();
    environment.reconcile_controller_tabs(
        vec![observed_tab("target-a", "loader-a1", "https://a.test", "A")],
        Some("target-a"),
    );
    let binding = environment
        .controller_tab_binding("tab-1")
        .expect("controller binding exists");
    assert_eq!(binding.runtime_target_id, "target-a");
    assert_eq!(binding.document_id, "loader-a1");
    assert_eq!(binding.document_revision, 1);

    let first = environment
        .register_element_references(
            "tab-1",
            1,
            1,
            ["backend:103".to_string(), "backend:104".to_string()],
        )
        .expect("element references register");
    let repeated = environment
        .register_element_references("tab-1", 1, 1, ["backend:103".to_string()])
        .expect("same document reuses references");
    assert_eq!(first["backend:103"], repeated["backend:103"]);
    assert!(first["backend:103"].starts_with("element-"));

    let resolved = environment
        .resolve_element_reference(&first["backend:103"])
        .expect("current reference resolves");
    assert_eq!(resolved.tab_id, "tab-1");
    assert_eq!(resolved.document_revision, 1);
    assert_eq!(resolved.controller_node_ref, "backend:103");

    environment.reconcile_controller_tabs(
        vec![observed_tab("target-a", "loader-a2", "https://a.test", "A")],
        Some("target-a"),
    );
    assert!(matches!(
        environment.resolve_element_reference(&first["backend:103"]),
        Err(EnvironmentError::StaleElementReference { .. })
    ));

    let current = environment
        .register_element_references("tab-1", 1, 2, ["backend:203".to_string()])
        .expect("new document receives a new reference");
    environment
        .invalidate_runtime_after_process_loss()
        .expect("process loss invalidates the runtime");
    assert!(matches!(
        environment.resolve_element_reference(&current["backend:203"]),
        Err(EnvironmentError::StaleElementReference { .. })
    ));
}

fn observed_tab(
    runtime_target_id: &str,
    document_id: &str,
    url: &str,
    title: &str,
) -> EnvironmentTabObservation {
    EnvironmentTabObservation {
        runtime_target_id: runtime_target_id.to_string(),
        document_id: document_id.to_string(),
        url: url.to_string(),
        title: title.to_string(),
    }
}

#[test]
fn closing_and_resetting_retire_runtime_tab_identity() {
    let mut environment = ready_environment();
    let tab_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    environment.close_tab(&tab_id).unwrap();
    let replacement_id = environment
        .register_or_reconcile_tab("controller-target-1", "https://example.test", "Example")
        .unwrap();
    assert_eq!(replacement_id, "tab-2");

    environment
        .transition_to(EnvironmentLifecycle::Stopping)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopped)
        .unwrap();
    environment.reset_runtime().unwrap();
    assert!(environment.snapshot().tabs.is_empty());
    assert_eq!(
        environment.validate_tab_reference(1, &replacement_id, 1),
        Err(EnvironmentError::StaleRuntimeGeneration {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn viewport_updates_are_actor_attributed_and_revision_guarded() {
    let mut environment = ready_environment();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let replacement = CanonicalViewport::new(1280, 720, 2, 2560, 1440).unwrap();

    environment
        .update_viewport("agent-1", 1, replacement)
        .expect("an actor may resize while no human owns input");
    assert_eq!(environment.snapshot().viewport.revision, 2);
    assert_eq!(
        environment.snapshot().viewport.last_actor_id.as_deref(),
        Some("agent-1")
    );

    let stale_replacement = CanonicalViewport::new(1024, 768, 1, 1024, 768).unwrap();
    assert_eq!(
        environment.update_viewport("agent-1", 1, stale_replacement),
        Err(EnvironmentError::StaleViewportRevision {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn viewport_updates_reject_unknown_actors() {
    let mut environment = ready_environment();
    let replacement = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    assert_eq!(
        environment.update_viewport("missing", 1, replacement),
        Err(EnvironmentError::UnknownActor {
            actor_id: "missing".to_string(),
        })
    );
}

#[test]
fn authenticated_viewport_update_does_not_register_actor_before_admission() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();
    let replacement = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    assert_eq!(
        environment.update_viewport_as_actor(
            EnvironmentActor::new("user-1", EnvironmentActorKind::Human, "User 1"),
            1,
            replacement,
        ),
        Err(EnvironmentError::EnvironmentNotReady {
            lifecycle: EnvironmentLifecycle::Stopped,
        })
    );
    assert!(environment.snapshot().actors.is_empty());
    assert_eq!(environment.snapshot().viewport.revision, 1);
}

#[test]
fn observations_run_concurrently_and_mutations_serialize_per_target() {
    let mut environment = ready_environment_with_agent();
    let tab_a = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let tab_b = environment
        .register_or_reconcile_tab("target-b", "https://b.test", "B")
        .unwrap();

    for _ in 0..2 {
        assert!(matches!(
            environment.submit_action(EnvironmentActionRequest::browser_observation(
                "agent-1", 1, "snapshot", &tab_a, 1,
            )),
            Ok(ActionAdmission::Accepted { .. })
        ));
    }

    let action_a = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_a, 1,
            ))
            .unwrap(),
    );
    let action_b = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "fill", &tab_b, 1,
            ))
            .unwrap(),
    );
    assert_ne!(action_a, action_b);
    let queued_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "second-click",
                &tab_a,
                1,
            ))
            .unwrap(),
    );
    assert_eq!(
        environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == queued_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Queued
    );
    assert_eq!(
        environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == queued_action_id)
            .unwrap()
            .started_at_ms,
        None
    );
    assert_eq!(
        environment.finish_action(&queued_action_id, EnvironmentActionTerminal::Completed),
        Err(EnvironmentError::ActionNotRunning {
            action_id: queued_action_id.clone(),
            state: EnvironmentActionState::Queued,
        })
    );

    environment
        .finish_action(&action_a, EnvironmentActionTerminal::Completed)
        .unwrap();
    let promoted = environment
        .snapshot()
        .actions
        .into_iter()
        .find(|action| action.action_id == queued_action_id)
        .unwrap();
    assert_eq!(promoted.state, EnvironmentActionState::Running);
    assert!(promoted.started_at_ms >= Some(promoted.submitted_at_ms));
}

#[test]
fn starting_environment_admits_browser_actions_only_when_browser_components_are_ready() {
    let mut environment = starting_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();

    environment.update_component_health(
        EnvironmentComponent::BrowserController,
        EnvironmentComponentHealthState::Ready,
        None,
    );
    assert_eq!(
        environment.submit_action(EnvironmentActionRequest::browser_mutation(
            "agent-1", 1, "click", &tab_id, 1,
        )),
        Err(EnvironmentError::EnvironmentNotReady {
            lifecycle: EnvironmentLifecycle::Starting,
        })
    );

    environment.update_component_health(
        EnvironmentComponent::Browser,
        EnvironmentComponentHealthState::Ready,
        None,
    );
    assert!(matches!(
        environment.submit_action(EnvironmentActionRequest::browser_mutation(
            "agent-1", 1, "click", &tab_id, 1,
        )),
        Ok(ActionAdmission::Accepted { .. })
    ));
    assert_eq!(
        environment.submit_action(EnvironmentActionRequest::computer_mutation(
            "agent-1",
            1,
            "pointer-click",
            Some(&tab_id),
        )),
        Err(EnvironmentError::EnvironmentNotReady {
            lifecycle: EnvironmentLifecycle::Starting,
        })
    );
}

#[test]
fn action_lifecycle_records_submission_start_finish_and_redacted_outcome() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
    );

    let running = environment
        .snapshot()
        .actions
        .into_iter()
        .find(|action| action.action_id == action_id)
        .expect("accepted Action should be projected");
    assert!(running.submitted_at_ms > 0);
    assert_eq!(running.started_at_ms, Some(running.submitted_at_ms));
    assert_eq!(running.finished_at_ms, None);
    assert_eq!(running.outcome, None);

    let event_cursor = environment.snapshot().event_cursor;
    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    let completed = environment
        .snapshot()
        .actions
        .into_iter()
        .find(|action| action.action_id == action_id)
        .expect("completed Action should remain in recent history");
    assert!(completed.finished_at_ms >= completed.started_at_ms);
    assert_eq!(completed.outcome, Some(EnvironmentActionOutcome::Completed));
    assert!(matches!(
        environment.events_after(event_cursor),
        EnvironmentReplay::Events { events, .. }
            if matches!(
                events.as_slice(),
                [EnvironmentEvent {
                    kind: EnvironmentEventKind::ActionChanged {
                        action_id: changed_action_id,
                        state: EnvironmentActionState::Completed,
                        started_at_ms,
                        finished_at_ms,
                        outcome: Some(EnvironmentActionOutcome::Completed),
                        ..
                    },
                    ..
                }] if changed_action_id == &action_id
                    && *started_at_ms == completed.started_at_ms
                    && *finished_at_ms == completed.finished_at_ms
            )
    ));
}

#[test]
fn computer_mutation_reserves_desktop_before_the_focused_tab() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "pointer-click",
                Some(&tab_id),
            ))
            .unwrap(),
    );
    let action = environment
        .snapshot()
        .actions
        .into_iter()
        .find(|action| action.action_id == action_id)
        .unwrap();
    assert_eq!(
        action.targets,
        vec![
            InputTarget::Desktop,
            InputTarget::BrowserTab(tab_id.clone())
        ]
    );

    assert!(matches!(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
        ActionAdmission::Queued { .. }
    ));
}

#[test]
fn mutation_queue_preserves_order_across_overlapping_multi_target_actions() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let running_tab_action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "tab-click",
                &tab_id,
                1,
            ))
            .unwrap(),
    );
    let queued_computer_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "pointer-click",
                Some(&tab_id),
            ))
            .unwrap(),
    );
    let queued_desktop_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "key-press",
                None,
            ))
            .unwrap(),
    );

    environment
        .finish_action(&running_tab_action_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    let snapshot = environment.snapshot();
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == queued_computer_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == queued_desktop_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Queued
    );

    environment
        .finish_action(
            &queued_computer_action_id,
            EnvironmentActionTerminal::Completed,
        )
        .unwrap();
    assert_eq!(
        environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == queued_desktop_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
}

#[test]
fn mutation_queue_rejects_new_work_at_its_bound() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_capacities("room-1", "environment-1", viewport, 128, 1).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();

    accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "first", &tab_id, 1,
            ))
            .unwrap(),
    );
    queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "second", &tab_id, 1,
            ))
            .unwrap(),
    );
    assert_eq!(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "third", &tab_id, 1,
            ))
            .unwrap(),
        ActionAdmission::RejectedSaturated { capacity: 1 }
    );
    assert_eq!(environment.snapshot().actions.len(), 2);
}

#[test]
fn cancelling_queued_work_promotes_the_next_eligible_action() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "tab-click",
                &tab_id,
                1,
            ))
            .unwrap(),
    );
    let queued_computer_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "pointer-click",
                Some(&tab_id),
            ))
            .unwrap(),
    );
    let queued_desktop_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "key-press",
                None,
            ))
            .unwrap(),
    );

    assert_eq!(
        environment
            .cancel_action("agent-1", &queued_computer_action_id)
            .unwrap(),
        ActionCancellationOutcome::Cancelled
    );
    let snapshot = environment.snapshot();
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == queued_computer_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Cancelled
    );
    let cancelled = snapshot
        .actions
        .iter()
        .find(|action| action.action_id == queued_computer_action_id)
        .unwrap();
    assert_eq!(cancelled.started_at_ms, None);
    assert!(cancelled.finished_at_ms >= Some(cancelled.submitted_at_ms));
    assert_eq!(
        cancelled.outcome,
        Some(EnvironmentActionOutcome::Cancelled {
            reason: EnvironmentActionCancellationReason::Requested,
        })
    );
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == queued_desktop_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
    assert_eq!(
        environment
            .cancel_action("agent-1", &queued_computer_action_id)
            .unwrap(),
        ActionCancellationOutcome::AlreadyTerminal {
            action_state: EnvironmentActionState::Cancelled,
        }
    );
}

#[test]
fn human_cancellation_requires_control_of_an_action_target() {
    let mut environment = ready_environment_with_agent();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "pointer-click",
                None,
            ))
            .unwrap(),
    );

    assert_eq!(
        environment.cancel_action("user-1", &action_id),
        Err(EnvironmentError::ActionCancellationForbidden {
            actor_id: "user-1".to_string(),
            action_id: action_id.clone(),
        })
    );
    assert!(
        !environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .unwrap()
            .cancellation_requested
    );
}

#[test]
fn human_takeover_waits_for_the_agent_action_to_be_terminal() {
    let mut environment = ready_environment_with_agent();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "fill", &tab_id, 1,
            ))
            .unwrap(),
    );
    let queued_browser_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "pointer-click",
                Some(&tab_id),
            ))
            .unwrap(),
    );
    let unblocked_desktop_action_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "key-press",
                None,
            ))
            .unwrap(),
    );
    let takeover_cursor = environment.snapshot().event_cursor;

    assert_eq!(
        environment
            .request_takeover("user-1", InputTarget::BrowserTab(tab_id.clone()))
            .unwrap(),
        TakeoverOutcome::CancellationRequired {
            action_ids: vec![action_id.clone()],
        }
    );
    assert!(environment.snapshot().input_ownership.is_empty());
    assert!(
        environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .unwrap()
            .cancellation_requested
    );
    assert!(matches!(
        environment.events_after(takeover_cursor),
        EnvironmentReplay::Events { events, .. }
            if events.iter().any(|event| matches!(
                &event.kind,
                EnvironmentEventKind::ActionChanged {
                    action_id: changed_action_id,
                    state: EnvironmentActionState::Running,
                    cancellation_requested: true,
                    ..
                } if changed_action_id == &action_id
            ))
    ));
    let cancellation_cursor = environment.snapshot().event_cursor;
    assert_eq!(
        environment.cancel_action("user-1", &action_id).unwrap(),
        ActionCancellationOutcome::CancellationRequested
    );
    assert_eq!(environment.snapshot().event_cursor, cancellation_cursor);
    assert_eq!(
        environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == queued_browser_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Cancelled
    );
    assert_eq!(
        environment
            .snapshot()
            .actions
            .iter()
            .find(|action| action.action_id == unblocked_desktop_action_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
    assert_eq!(
        environment.snapshot().pending_input_takeovers,
        vec![PendingInputTakeover {
            target: InputTarget::BrowserTab(tab_id.clone()),
            human_actor_id: "user-1".to_string(),
            blocking_action_ids: vec![action_id.clone()],
        }]
    );
    assert!(matches!(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
        ActionAdmission::RejectedTakeover {
            human_actor_id,
            ..
        } if human_actor_id == "user-1"
    ));

    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Cancelled)
        .unwrap();
    assert_eq!(
        environment.snapshot().input_ownership,
        vec![InputOwnership {
            target: InputTarget::BrowserTab(tab_id.clone()),
            actor_id: "user-1".to_string(),
        }]
    );
    assert!(environment.snapshot().pending_input_takeovers.is_empty());
    let snapshot = environment.snapshot();
    let cancelled_running = snapshot
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .unwrap();
    assert!(!cancelled_running.cancellation_requested);
    assert_eq!(
        cancelled_running.outcome,
        Some(EnvironmentActionOutcome::Cancelled {
            reason: EnvironmentActionCancellationReason::Requested,
        })
    );
    let cancelled_queued = snapshot
        .actions
        .iter()
        .find(|action| action.action_id == queued_browser_action_id)
        .unwrap();
    assert_eq!(cancelled_queued.state, EnvironmentActionState::Cancelled);
    assert_eq!(
        cancelled_queued.outcome,
        Some(EnvironmentActionOutcome::Cancelled {
            reason: EnvironmentActionCancellationReason::HumanTakeover,
        })
    );
    assert_eq!(
        environment
            .request_takeover("user-1", InputTarget::BrowserTab(tab_id.clone()))
            .unwrap(),
        TakeoverOutcome::Granted
    );
    environment
        .release_input("user-1", &InputTarget::BrowserTab(tab_id))
        .unwrap();
    assert!(environment.snapshot().input_ownership.is_empty());
}

#[test]
fn failed_takeover_does_not_register_the_authenticated_actor() {
    let mut environment = ready_environment();

    assert_eq!(
        environment.request_takeover_as_actor(
            EnvironmentActor::new("user-1", EnvironmentActorKind::Human, "Miguel"),
            InputTarget::BrowserTab("missing-tab".to_string()),
        ),
        Err(EnvironmentError::UnknownTab {
            tab_id: "missing-tab".to_string(),
        })
    );
    assert!(environment.snapshot().actors.is_empty());
    assert!(environment.snapshot().input_ownership.is_empty());
    assert!(environment.snapshot().pending_input_takeovers.is_empty());
}

#[test]
fn only_the_authenticated_owner_can_release_input() {
    let mut environment = ready_environment();
    for actor_id in ["user-1", "user-2"] {
        environment
            .register_actor(EnvironmentActor::new(
                actor_id,
                EnvironmentActorKind::Human,
                actor_id,
            ))
            .unwrap();
    }
    environment
        .request_takeover("user-1", InputTarget::Desktop)
        .unwrap();

    assert_eq!(
        environment.release_input("user-2", &InputTarget::Desktop),
        Err(EnvironmentError::InputOwnedByAnotherActor {
            target: InputTarget::Desktop,
            actor_id: "user-1".to_string(),
        })
    );
    assert_eq!(
        environment.snapshot().input_ownership,
        vec![InputOwnership {
            target: InputTarget::Desktop,
            actor_id: "user-1".to_string(),
        }]
    );
}

#[test]
fn desktop_owner_exclusively_controls_viewport_updates() {
    let mut environment = ready_environment_with_agent();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    assert_eq!(
        environment.request_takeover("user-1", InputTarget::Desktop),
        Ok(TakeoverOutcome::Granted)
    );

    let agent_viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    assert_eq!(
        environment.update_viewport("agent-1", 1, agent_viewport),
        Err(EnvironmentError::InputOwnedByAnotherActor {
            target: InputTarget::Desktop,
            actor_id: "user-1".to_string(),
        })
    );
    let human_viewport = CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
    environment
        .update_viewport("user-1", 1, human_viewport)
        .unwrap();
}

#[test]
fn reconnect_replays_ordered_events_or_requires_a_snapshot_after_a_gap() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_event_capacity("room-1", "environment-1", viewport, 3).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let cursor = environment.snapshot().event_cursor;

    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_id, 1,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Completed)
        .unwrap();

    let expected = environment.events_after(cursor);
    assert!(matches!(
        &expected,
        EnvironmentReplay::Events { events, next_cursor }
            if events.len() == 3
                && events.windows(2).all(|pair| pair[0].event_id + 1 == pair[1].event_id)
                && *next_cursor == environment.snapshot().event_cursor
    ));
    assert_eq!(environment.events_after(cursor), expected);
    assert!(matches!(
        environment.events_after(0),
        EnvironmentReplay::SnapshotRequired { snapshot }
            if snapshot.event_cursor == environment.snapshot().event_cursor
    ));
}

#[test]
fn process_loss_fails_only_running_actions_and_invalidates_runtime_handles() {
    let mut environment = ready_environment_with_agent();
    let tab_a = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let tab_b = environment
        .register_or_reconcile_tab("target-b", "https://b.test", "B")
        .unwrap();
    let completed_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "click", &tab_a, 1,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&completed_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    let running_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1", 1, "fill", &tab_b, 1,
            ))
            .unwrap(),
    );

    let cursor = environment.snapshot().event_cursor;
    environment.invalidate_runtime_after_process_loss().unwrap();
    let snapshot = environment.snapshot();
    assert_eq!(snapshot.runtime_generation, 2);
    assert_eq!(snapshot.lifecycle, EnvironmentLifecycle::Starting);
    assert!(snapshot.tabs.is_empty());
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == completed_id)
            .unwrap()
            .state,
        EnvironmentActionState::Completed
    );
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == running_id)
            .unwrap()
            .state,
        EnvironmentActionState::Failed
    );
    let failed = snapshot
        .actions
        .iter()
        .find(|action| action.action_id == running_id)
        .unwrap();
    assert!(failed.finished_at_ms >= failed.started_at_ms);
    assert_eq!(
        failed.outcome,
        Some(EnvironmentActionOutcome::Failed {
            code: EnvironmentActionFailureCode::ProcessLost,
        })
    );
    assert!(matches!(
        environment.events_after(cursor),
        EnvironmentReplay::Events { events, .. }
            if matches!(events[events.len() - 2].kind, EnvironmentEventKind::RuntimeInvalidated)
                && matches!(
                    events[events.len() - 1].kind,
                    EnvironmentEventKind::LifecycleChanged {
                        lifecycle: EnvironmentLifecycle::Starting,
                    }
                )
    ));
}

#[test]
fn controller_recovery_preserves_room_identity_tabs_actors_and_human_ownership() {
    let mut environment = ready_environment_with_agent();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    environment.reconcile_controller_tabs(
        vec![
            observed_tab("target-a", "loader-a", "https://a.test", "A"),
            observed_tab("target-b", "loader-b", "https://b.test", "B"),
        ],
        Some("target-a"),
    );
    let element_refs = environment
        .register_element_references("tab-1", 1, 1, ["backend:103".to_string()])
        .unwrap();
    let completed_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "completed-click",
                "tab-1",
                1,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&completed_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    let running_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "running-click",
                "tab-1",
                1,
            ))
            .unwrap(),
    );
    let queued_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "queued-click",
                "tab-1",
                1,
            ))
            .unwrap(),
    );
    let computer_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "desktop-key",
                None,
            ))
            .unwrap(),
    );
    assert_eq!(
        environment
            .request_takeover("user-1", InputTarget::BrowserTab("tab-2".to_string()))
            .unwrap(),
        TakeoverOutcome::Granted
    );

    environment.begin_browser_controller_recovery();
    let recovering = environment.snapshot();
    assert_eq!(recovering.environment_id, "environment-1");
    assert_eq!(recovering.runtime_generation, 1);
    assert_eq!(
        recovering
            .tabs
            .iter()
            .map(|tab| tab.tab_id.as_str())
            .collect::<Vec<_>>(),
        vec!["tab-1", "tab-2"]
    );
    assert_eq!(recovering.actors.len(), 2);
    assert_eq!(
        recovering.input_ownership,
        vec![InputOwnership {
            target: InputTarget::BrowserTab("tab-2".to_string()),
            actor_id: "user-1".to_string(),
        }]
    );
    assert_eq!(
        recovering
            .actions
            .iter()
            .find(|action| action.action_id == completed_id)
            .unwrap()
            .state,
        EnvironmentActionState::Completed
    );
    assert_eq!(
        recovering
            .actions
            .iter()
            .find(|action| action.action_id == running_id)
            .unwrap()
            .outcome,
        Some(EnvironmentActionOutcome::Failed {
            code: EnvironmentActionFailureCode::ProcessLost,
        })
    );
    assert_eq!(
        recovering
            .actions
            .iter()
            .find(|action| action.action_id == queued_id)
            .unwrap()
            .state,
        EnvironmentActionState::Queued
    );
    assert_eq!(
        recovering
            .actions
            .iter()
            .find(|action| action.action_id == computer_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
    assert_eq!(
        environment.resolve_element_reference(&element_refs["backend:103"]),
        Err(EnvironmentError::StaleElementReference {
            reference_id: element_refs["backend:103"].clone(),
        })
    );
    assert_eq!(
        environment.submit_action(EnvironmentActionRequest::browser_mutation(
            "agent-1",
            1,
            "blocked-during-recovery",
            "tab-1",
            1,
        )),
        Err(EnvironmentError::EnvironmentNotReady {
            lifecycle: EnvironmentLifecycle::Starting,
        })
    );

    environment.reconcile_controller_tabs(
        vec![
            observed_tab("target-a", "loader-a", "https://a.test", "A"),
            observed_tab("target-b", "loader-b", "https://b.test", "B"),
        ],
        Some("target-a"),
    );
    environment.complete_browser_controller_recovery();
    let recovered = environment.snapshot();
    assert_eq!(recovered.runtime_generation, 1);
    assert_eq!(recovered.tabs.len(), 2);
    assert_eq!(recovered.input_ownership, recovering.input_ownership);
    assert_eq!(
        recovered
            .actions
            .iter()
            .find(|action| action.action_id == queued_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
    assert_eq!(
        recovered
            .actions
            .iter()
            .find(|action| action.action_id == computer_id)
            .unwrap()
            .state,
        EnvironmentActionState::Running
    );
}

#[test]
fn controller_recovery_fails_queued_work_if_its_document_changed() {
    let mut environment = ready_environment_with_agent();
    environment.reconcile_controller_tabs(
        vec![observed_tab("target-a", "loader-a1", "https://a.test", "A")],
        Some("target-a"),
    );
    accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "running-click",
                "tab-1",
                1,
            ))
            .unwrap(),
    );
    let queued_id = queued_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_mutation(
                "agent-1",
                1,
                "queued-click",
                "tab-1",
                1,
            ))
            .unwrap(),
    );

    environment.begin_browser_controller_recovery();
    environment.reconcile_controller_tabs(
        vec![observed_tab(
            "target-a",
            "loader-a2",
            "https://a.test/reloaded",
            "Reloaded",
        )],
        Some("target-a"),
    );
    environment.complete_browser_controller_recovery();

    let recovered = environment.snapshot();
    assert_eq!(recovered.tabs.len(), 1);
    assert_eq!(recovered.tabs[0].tab_id, "tab-1");
    assert_eq!(recovered.tabs[0].document_revision, 2);
    let queued = recovered
        .actions
        .iter()
        .find(|action| action.action_id == queued_id)
        .unwrap();
    assert_eq!(queued.state, EnvironmentActionState::Failed);
    assert_eq!(
        queued.outcome,
        Some(EnvironmentActionOutcome::Failed {
            code: EnvironmentActionFailureCode::ProcessLost,
        })
    );
}

#[test]
fn process_loss_emits_action_changes_before_compacting_terminal_records() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_capacities("room-1", "environment-1", viewport, 1, 128).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let tab_a = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let tab_b = environment
        .register_or_reconcile_tab("target-b", "https://b.test", "B")
        .unwrap();
    for (tab_id, operation) in [(tab_a, "click"), (tab_b, "fill")] {
        accepted_action_id(
            environment
                .submit_action(EnvironmentActionRequest::browser_mutation(
                    "agent-1", 1, operation, &tab_id, 1,
                ))
                .unwrap(),
        );
    }

    environment.invalidate_runtime_after_process_loss().unwrap();

    let snapshot = environment.snapshot();
    assert_eq!(snapshot.actions.len(), 1);
    assert_eq!(snapshot.actions[0].state, EnvironmentActionState::Failed);
    assert_eq!(
        snapshot.actions[0].outcome,
        Some(EnvironmentActionOutcome::Failed {
            code: EnvironmentActionFailureCode::ProcessLost,
        })
    );
}

#[test]
fn idempotency_reuses_the_original_action_and_rejects_conflicting_replays() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let request = EnvironmentActionRequest::browser_mutation("agent-1", 1, "send", &tab_id, 1)
        .with_idempotency_key("send-message-1");
    let action_id = accepted_action_id(environment.submit_action(request.clone()).unwrap());

    assert_eq!(
        environment.submit_action(request).unwrap(),
        ActionAdmission::Existing {
            action_id: action_id.clone(),
            state: EnvironmentActionState::Running,
        }
    );
    let conflicting =
        EnvironmentActionRequest::browser_mutation("agent-1", 1, "delete", &tab_id, 1)
            .with_idempotency_key("send-message-1");
    assert_eq!(
        environment.submit_action(conflicting),
        Err(EnvironmentError::IdempotencyConflict {
            idempotency_key: "send-message-1".to_string(),
        })
    );
}

#[test]
fn terminal_action_state_is_immutable() {
    let mut environment = ready_environment_with_agent();
    let action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "key-chord",
                None,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&action_id, EnvironmentActionTerminal::Completed)
        .unwrap();
    assert_eq!(
        environment.finish_action(&action_id, EnvironmentActionTerminal::Failed),
        Err(EnvironmentError::ActionAlreadyTerminal {
            action_id,
            state: EnvironmentActionState::Completed,
        })
    );
}

#[test]
fn controller_terminal_outcomes_use_closed_redacted_codes() {
    let mut environment = ready_environment_with_agent();
    let failed_action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "key-chord",
                None,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&failed_action_id, EnvironmentActionTerminal::Failed)
        .unwrap();
    let cancelled_action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1",
                1,
                "mouse-click",
                None,
            ))
            .unwrap(),
    );
    environment
        .finish_action(&cancelled_action_id, EnvironmentActionTerminal::Cancelled)
        .unwrap();

    let snapshot = environment.snapshot();
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == failed_action_id)
            .unwrap()
            .outcome,
        Some(EnvironmentActionOutcome::Failed {
            code: EnvironmentActionFailureCode::ControllerFailure,
        })
    );
    assert_eq!(
        snapshot
            .actions
            .iter()
            .find(|action| action.action_id == cancelled_action_id)
            .unwrap()
            .outcome,
        Some(EnvironmentActionOutcome::Cancelled {
            reason: EnvironmentActionCancellationReason::ControllerCancellation,
        })
    );
}

#[test]
fn restarting_a_stopped_runtime_invalidates_old_handles() {
    let mut environment = ready_environment();
    environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopping)
        .unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Stopped)
        .unwrap();

    environment.start_runtime().unwrap();
    assert_eq!(environment.snapshot().runtime_generation, 2);
    assert_eq!(
        environment.snapshot().lifecycle,
        EnvironmentLifecycle::Starting
    );
    assert!(environment.snapshot().tabs.is_empty());
}

#[test]
fn terminal_action_snapshot_is_bounded_but_history_and_idempotency_are_retained() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_event_capacity("room-1", "environment-1", viewport, 2).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();

    let mut completed_ids = Vec::new();
    for sequence in 1..=3 {
        let action_id = accepted_action_id(
            environment
                .submit_action(
                    EnvironmentActionRequest::browser_mutation(
                        "agent-1",
                        1,
                        format!("click-{sequence}"),
                        &tab_id,
                        1,
                    )
                    .with_idempotency_key(format!("click-{sequence}")),
                )
                .unwrap(),
        );
        environment
            .finish_action(&action_id, EnvironmentActionTerminal::Completed)
            .unwrap();
        completed_ids.push(action_id);
    }
    let active_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::browser_observation(
                "agent-1", 1, "snapshot", &tab_id, 1,
            ))
            .unwrap(),
    );

    let retained_ids: Vec<_> = environment
        .snapshot()
        .actions
        .into_iter()
        .map(|action| action.action_id)
        .collect();
    assert_eq!(
        retained_ids,
        vec![
            completed_ids[1].clone(),
            completed_ids[2].clone(),
            active_id,
        ]
    );
    assert_eq!(
        environment
            .submit_action(
                EnvironmentActionRequest::browser_mutation("agent-1", 1, "click-1", &tab_id, 1,)
                    .with_idempotency_key("click-1"),
            )
            .unwrap(),
        ActionAdmission::Existing {
            action_id: completed_ids[0].clone(),
            state: EnvironmentActionState::Completed,
        }
    );
}

#[test]
fn compacted_non_idempotent_actions_release_their_request_payloads() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_event_capacity("room-1", "environment-1", viewport, 2).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();

    for operation in ["snapshot-1", "snapshot-2", "snapshot-3"] {
        let action_id = accepted_action_id(
            environment
                .submit_action(EnvironmentActionRequest::computer_mutation(
                    "agent-1", 1, operation, None,
                ))
                .unwrap(),
        );
        environment
            .finish_action(&action_id, EnvironmentActionTerminal::Completed)
            .unwrap();
    }

    assert_eq!(environment.snapshot().actions.len(), 2);
    assert_eq!(environment.retained_action_request_count(), 2);
}

#[test]
fn action_history_pages_newest_first_across_hot_record_compaction() {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment =
        RoomEnvironment::new_with_capacities("room-1", "environment-1", viewport, 1, 128).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();

    let mut action_ids = Vec::new();
    for operation in ["click", "fill", "key-chord"] {
        let action_id = accepted_action_id(
            environment
                .submit_action(EnvironmentActionRequest::computer_mutation(
                    "agent-1", 1, operation, None,
                ))
                .unwrap(),
        );
        environment
            .finish_action(&action_id, EnvironmentActionTerminal::Completed)
            .unwrap();
        action_ids.push(action_id);
    }

    assert_eq!(environment.snapshot().actions.len(), 1);
    let first_page = environment.action_history(None, 2);
    assert_eq!(
        first_page
            .actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec![action_ids[2].as_str(), action_ids[1].as_str()]
    );
    assert!(first_page
        .actions
        .iter()
        .all(|action| action.outcome == Some(EnvironmentActionOutcome::Completed)));
    assert_eq!(first_page.next_before_sequence, Some(2));

    let second_page = environment.action_history(first_page.next_before_sequence, 2);
    assert_eq!(
        second_page
            .actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec![action_ids[0].as_str()]
    );
    assert_eq!(second_page.next_before_sequence, None);

    let running_action_id = accepted_action_id(
        environment
            .submit_action(EnvironmentActionRequest::computer_mutation(
                "agent-1", 1, "scroll", None,
            ))
            .unwrap(),
    );
    environment
        .cancel_action("agent-1", &running_action_id)
        .unwrap();
    let running_page = environment.action_history(None, 1);
    assert_eq!(running_page.actions[0].action_id, running_action_id);
    assert_eq!(
        running_page.actions[0].state,
        EnvironmentActionState::Running
    );
    assert!(running_page.actions[0].cancellation_requested);
    environment
        .finish_action(&running_action_id, EnvironmentActionTerminal::Cancelled)
        .unwrap();
    assert_eq!(
        environment.action_history(None, 1).actions[0].outcome,
        Some(EnvironmentActionOutcome::Cancelled {
            reason: EnvironmentActionCancellationReason::Requested,
        })
    );
}

#[test]
fn one_room_cannot_acquire_a_second_environment() {
    let mut environments = RoomEnvironmentRegistry::new();
    let first = environments
        .create(
            "room-1",
            "environment-1",
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .expect("the Room should acquire its first Environment");
    assert_eq!(first.session_id, "room-1");
    assert_eq!(first.environment_id, "environment-1");

    let error = environments
        .create(
            "room-1",
            "environment-2",
            CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap(),
        )
        .expect_err("the Room must not acquire a second Environment implicitly");
    assert_eq!(
        error,
        EnvironmentError::EnvironmentAlreadyExists {
            session_id: "room-1".to_string(),
            environment_id: "environment-1".to_string(),
        }
    );
    assert_eq!(
        environments
            .snapshot("room-1")
            .expect("the original Environment should remain")
            .environment_id,
        "environment-1"
    );
}

#[test]
fn removing_a_room_retires_its_environment_identity() {
    let mut environments = RoomEnvironmentRegistry::new();
    environments
        .create(
            "room-1",
            "environment-1",
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .expect("the Environment should be created");

    let retired = environments
        .remove("room-1")
        .expect("the Environment should be retired with its Room");
    assert_eq!(retired.environment_id, "environment-1");
    assert_eq!(
        environments
            .snapshot("room-1")
            .expect_err("a retired Environment must not remain addressable"),
        EnvironmentError::EnvironmentNotFound {
            session_id: "room-1".to_string(),
        }
    );
}

#[test]
fn idempotency_survives_generation_change_without_repeating_work() {
    let mut environment = ready_environment_with_agent();
    let tab_id = environment
        .register_or_reconcile_tab("target-a", "https://a.test", "A")
        .unwrap();
    let request = EnvironmentActionRequest::browser_mutation("agent-1", 1, "send", &tab_id, 1)
        .with_idempotency_key("send-message-1");
    let action_id = accepted_action_id(environment.submit_action(request).unwrap());
    environment.invalidate_runtime_after_process_loss().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();

    let retry = EnvironmentActionRequest::browser_mutation("agent-1", 2, "send", &tab_id, 99)
        .with_idempotency_key("send-message-1");
    assert_eq!(
        environment.submit_action(retry).unwrap(),
        ActionAdmission::Existing {
            action_id,
            state: EnvironmentActionState::Failed,
        }
    );
}

#[test]
fn actor_reconnect_preserves_identity_and_cannot_change_actor_kind() {
    let mut environment = ready_environment();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel",
        ))
        .unwrap();
    environment
        .set_actor_presence("user-1", EnvironmentActorPresence::Disconnected)
        .unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Human,
            "Miguel G.",
        ))
        .unwrap();
    let actor = &environment.snapshot().actors[0];
    assert_eq!(actor.actor_id, "user-1");
    assert_eq!(actor.display_label, "Miguel G.");
    assert_eq!(actor.presence, EnvironmentActorPresence::Present);

    assert_eq!(
        environment.register_actor(EnvironmentActor::new(
            "user-1",
            EnvironmentActorKind::Agent,
            "Not Miguel",
        )),
        Err(EnvironmentError::ActorKindConflict {
            actor_id: "user-1".to_string(),
        })
    );
}

#[test]
fn actor_reconciliation_preserves_history_and_emits_only_for_changes() {
    let mut environment = ready_environment();
    let human = EnvironmentActor::new("user-1", EnvironmentActorKind::Human, "Miguel");
    let agent = EnvironmentActor::new("agent-1", EnvironmentActorKind::Agent, "Mara");

    environment
        .reconcile_actors(vec![human.clone(), agent.clone()])
        .unwrap();
    let reconciled_cursor = environment.snapshot().event_cursor;
    environment
        .reconcile_actors(vec![human.clone(), agent])
        .unwrap();
    assert_eq!(environment.snapshot().event_cursor, reconciled_cursor);

    environment.reconcile_actors(vec![human]).unwrap();
    let actors = environment.snapshot().actors;
    assert_eq!(actors.len(), 2);
    assert_eq!(
        actors
            .iter()
            .find(|actor| actor.actor_id == "user-1")
            .unwrap()
            .presence,
        EnvironmentActorPresence::Present
    );
    assert_eq!(
        actors
            .iter()
            .find(|actor| actor.actor_id == "agent-1")
            .unwrap()
            .presence,
        EnvironmentActorPresence::Disconnected
    );
}

#[test]
fn component_health_projects_safe_diagnostic_codes() {
    let mut environment = ready_environment();
    environment.update_component_health(
        EnvironmentComponent::BrowserController,
        EnvironmentComponentHealthState::Ready,
        None,
    );
    environment.update_component_health(
        EnvironmentComponent::Streamer,
        EnvironmentComponentHealthState::Degraded,
        Some("encoder_restart_required"),
    );

    assert_eq!(
        environment.snapshot().health,
        vec![
            EnvironmentComponentHealth {
                component: EnvironmentComponent::BrowserController,
                state: EnvironmentComponentHealthState::Ready,
                diagnostic_code: None,
            },
            EnvironmentComponentHealth {
                component: EnvironmentComponent::Browser,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            },
            EnvironmentComponentHealth {
                component: EnvironmentComponent::Desktop,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            },
            EnvironmentComponentHealth {
                component: EnvironmentComponent::Streamer,
                state: EnvironmentComponentHealthState::Degraded,
                diagnostic_code: Some("encoder_restart_required".to_string()),
            },
        ]
    );
}

fn accepted_action_id(admission: ActionAdmission) -> String {
    match admission {
        ActionAdmission::Accepted { action_id } => action_id,
        other => panic!("expected accepted action, got {other:?}"),
    }
}

fn queued_action_id(admission: ActionAdmission) -> String {
    match admission {
        ActionAdmission::Queued { action_id, .. } => action_id,
        other => panic!("expected queued action, got {other:?}"),
    }
}

fn ready_environment_with_agent() -> RoomEnvironment {
    let mut environment = ready_environment();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    environment
}

fn starting_environment_with_agent() -> RoomEnvironment {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();
    environment.start_runtime().unwrap();
    environment
        .register_actor(EnvironmentActor::new(
            "agent-1",
            EnvironmentActorKind::Agent,
            "Mara",
        ))
        .unwrap();
    environment
}

fn ready_environment() -> RoomEnvironment {
    let viewport = CanonicalViewport::new(1440, 900, 1, 1440, 900).unwrap();
    let mut environment = RoomEnvironment::new("room-1", "environment-1", viewport).unwrap();
    environment.start_runtime().unwrap();
    environment
        .transition_to(EnvironmentLifecycle::Ready)
        .unwrap();
    environment
}
