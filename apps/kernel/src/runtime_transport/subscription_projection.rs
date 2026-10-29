//! MP-08 / MP-10 / MP-11: publish small session projections before bulk output.
use crate::runtime::projection::SessionSnapshotProjection;
use crate::transport::kernel_protocol::{
    can_skip_session_snapshot, room_workflows_changed_event, session_projection_changed_events,
    workflow_run_only_changed, workflow_run_updated_events, KernelEvent,
};

pub(crate) fn session_projection_events(
    snapshot: &SessionSnapshotProjection,
    previous: Option<&SessionSnapshotProjection>,
    has_terminal_workflow_update: bool,
) -> Vec<KernelEvent> {
    let mut events = session_projection_changed_events(snapshot, previous);
    let workflow_events = workflow_run_updated_events(snapshot, previous)
        .into_iter()
        .chain(room_workflows_changed_event(snapshot, previous))
        .collect::<Vec<_>>();
    let workflow_run_only =
        workflow_run_only_changed(snapshot, previous) && !workflow_events.is_empty();
    // An archived terminal run is replaced by the separately drained update.
    // A full hot-session snapshot would immediately erase that archive in clients.
    // Room inventory alone still needs the full snapshot for canvas consumers.
    let processes_only_changed = previous.is_some_and(|previous| {
        previous.session == snapshot.session
            && previous.provider_run == snapshot.provider_run
            && previous.agent_activity == snapshot.agent_activity
            && previous.room_workflows == snapshot.room_workflows
    });
    let replaces_snapshot = has_terminal_workflow_update
        || events.iter().any(|event| {
            // Resource samples carry no provider or prompt state. A concurrent
            // transition still needs the snapshot when no other delta covers it.
            !matches!(event, KernelEvent::SessionProcessesChanged { .. }) || processes_only_changed
        })
        || workflow_events
            .iter()
            .any(|event| !matches!(event, KernelEvent::RoomWorkflowsChanged { .. }))
        || workflow_run_only;
    events.extend(workflow_events);
    if !can_skip_session_snapshot(snapshot, previous, replaces_snapshot) {
        events.push(KernelEvent::SessionSnapshot {
            room_workflows: snapshot.room_workflows.clone(),
            session: Box::new(snapshot.session.clone()),
            provider_run: Box::new(
                snapshot
                    .provider_run
                    .clone()
                    .map(crate::provider::PublicProviderRun::from),
            ),
            agent_activity: Box::new(snapshot.agent_activity.clone()),
            agent_activity_revision: snapshot.metadata.last_event_id,
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SessionSnapshotProjection {
        let session = crate::session::RuntimeSession::new(
            "session",
            Some("before".into()),
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        SessionSnapshotProjection {
            processes: Vec::new(),
            room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
                &session,
                crate::session::DEFAULT_LOCAL_USER_ID,
            ),
            metadata: crate::runtime::projection::ProjectionMetadata::new(3, 1),
            session,
            provider_run: None,
            agent_activity: Default::default(),
        }
    }

    #[test]
    fn initial_projection_supplies_the_full_session_before_output() {
        assert!(matches!(
            session_projection_events(&snapshot(), None, false).as_slice(),
            [KernelEvent::SessionSnapshot { .. }]
        ));
    }

    #[test]
    fn metadata_delta_does_not_repeat_the_full_session() {
        let before = snapshot();
        let mut after = before.clone();
        after.session.set_alias(Some("after".into()));
        assert!(matches!(
            session_projection_events(&after, Some(&before), false).as_slice(),
            [KernelEvent::SessionMetadataChanged { .. }]
        ));
    }

    fn add_process(snapshot: &mut SessionSnapshotProjection) {
        use crate::runtime::process_resources::{
            ProcessSample, SessionProcess, SessionProcessOwner,
        };
        snapshot.processes.push(SessionProcess {
            owner: SessionProcessOwner::Agent("agent".into()),
            process: ProcessSample {
                process_id: "process".into(),
                pid: 42,
                command: "provider".into(),
                resident_set_bytes: 1024,
                cpu_millis: 0,
                sampled_at_ms: 10,
                age_ms: 10,
            },
        });
    }

    #[test]
    fn process_delta_does_not_repeat_the_full_session() {
        let before = snapshot();
        let mut after = before.clone();
        add_process(&mut after);
        assert!(matches!(
            session_projection_events(&after, Some(&before), false).as_slice(),
            [KernelEvent::SessionProcessesChanged { .. }]
        ));
    }

    #[test]
    fn process_delta_keeps_simultaneous_session_and_provider_changes() {
        let before = snapshot();
        let mut after = before.clone();
        add_process(&mut after);
        after.session.set_alias(Some("after".into()));
        after.provider_run = Some(
            crate::provider::RuntimeProviderRun::from_control_capability_inference(
                "run",
                after.session.id().into(),
                Some("agent".into()),
                "codex".into(),
            ),
        );
        assert!(matches!(
            session_projection_events(&after, Some(&before), false).as_slice(),
            [
                KernelEvent::SessionProcessesChanged { .. },
                KernelEvent::SessionSnapshot { .. }
            ]
        ));
    }

    #[test]
    fn process_delta_keeps_the_room_workflow_inventory_snapshot() {
        let before = snapshot();
        let mut after = before.clone();
        add_process(&mut after);
        after.room_workflows.revision.push_str("-changed");
        assert!(matches!(
            session_projection_events(&after, Some(&before), false).as_slice(),
            [
                KernelEvent::SessionProcessesChanged { .. },
                KernelEvent::RoomWorkflowsChanged { .. },
                KernelEvent::SessionSnapshot { .. }
            ]
        ));
    }

    #[test]
    fn archived_terminal_workflow_update_prevents_a_hot_snapshot_from_erasing_it() {
        let snapshot = snapshot();
        assert!(session_projection_events(&snapshot, Some(&snapshot), true).is_empty());
    }
    // MP-08 / MP-10 / MP-11: pre-backlog publication keeps canonical task/window deltas.
    #[test]
    fn task_updates_reach_the_pre_backlog_projection() {
        let before = snapshot();
        let mut after = before.clone();
        let task = serde_json::from_value(serde_json::json!({
            "task_id":"task", "room_id":after.session.id(), "owner_user_id":"local",
            "agent_id":"agent", "prompt_id":"prompt", "provider_run_id":null,
            "revision":2, "blocked_revision":0, "state":"working", "reason":"work",
            "obligations":[], "wait":null, "last_progress_at_ms":1, "progress_sequence":0,
            "no_progress_wakes":0, "correction_used":false, "pending_prompt_id":null
        }))
        .unwrap();
        after.session.set_agent_tasks(vec![task]);
        let events = session_projection_events(&after, Some(&before), false);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, KernelEvent::AgentTaskUpdated { .. })),
            "pre-backlog delivery must keep the canonical task delta"
        );
    }

    #[test]
    fn sudo_window_change_keeps_a_full_snapshot_beside_an_interaction() {
        let before = snapshot();
        let mut after = before.clone();
        after.session.set_alias(Some("after".into()));
        let window = serde_json::from_value(serde_json::json!({
            "entry_id":"sudo:window", "session_id":after.session.id(), "agent_id":"agent",
            "owner_user_id":"local", "terminal_id":"terminal", "requester":null,
            "prompt_id":"prompt", "provider_run_id":"run", "task_id":"task",
            "duration_minutes":60, "expires_at_ms":3600000, "revision":1, "warning_sent":false
        }))
        .unwrap();
        after.session.set_sudo_windows(vec![window]);
        after
            .session
            .add_active_interaction(crate::session::RuntimeInteraction::new(
                "owner-decision",
                "agent",
                crate::session::RuntimeInteractionKind::Permission,
                crate::session::RuntimeInteractionLevel::Warning,
                None,
                "Owner decision",
                vec![crate::session::RuntimeInteractionChoice::new(
                    "cancel", "Cancel", "cancel", None,
                )],
                None,
                None,
                None,
            ));
        let events = session_projection_events(&after, Some(&before), false);
        assert!(events
            .iter()
            .any(|event| matches!(event, KernelEvent::RuntimeInteractionsChanged { .. })));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, KernelEvent::SessionSnapshot { .. })),
            "an interaction delta cannot hide the newly opened sudo window"
        );
    }
}
