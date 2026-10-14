//! MP-08 / MP-10 / MP-11: publish small session projections before bulk output.
use crate::runtime::projection::SessionSnapshotProjection;
use crate::transport::kernel_protocol::{
    agent_activity_changed_event, provider_run_changed_event, room_workflows_changed_event,
    runtime_interactions_changed_event, session_metadata_changed_event, workflow_run_only_changed,
    workflow_run_updated_events, KernelEvent,
};

pub(crate) fn session_projection_events(
    snapshot: &SessionSnapshotProjection,
    previous: Option<&SessionSnapshotProjection>,
    has_terminal_workflow_update: bool,
) -> Vec<KernelEvent> {
    let mut events = [
        agent_activity_changed_event(snapshot, previous),
        provider_run_changed_event(snapshot, previous),
        session_metadata_changed_event(snapshot, previous),
        runtime_interactions_changed_event(snapshot, previous),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let workflow_events = workflow_run_updated_events(snapshot, previous)
        .into_iter()
        .chain(room_workflows_changed_event(snapshot, previous))
        .collect::<Vec<_>>();
    let workflow_run_only =
        workflow_run_only_changed(snapshot, previous) && !workflow_events.is_empty();
    // An archived terminal run is replaced by the separately drained update.
    // A full hot-session snapshot would immediately erase that archive in clients.
    // Room inventory alone still needs the full snapshot for canvas consumers.
    let replaces_snapshot = has_terminal_workflow_update
        || !events.is_empty()
        || workflow_events
            .iter()
            .any(|event| !matches!(event, KernelEvent::RoomWorkflowsChanged { .. }))
        || workflow_run_only;
    events.extend(workflow_events);
    if !replaces_snapshot {
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

    #[test]
    fn archived_terminal_workflow_update_prevents_a_hot_snapshot_from_erasing_it() {
        let snapshot = snapshot();
        assert!(session_projection_events(&snapshot, Some(&snapshot), true).is_empty());
    }
}
