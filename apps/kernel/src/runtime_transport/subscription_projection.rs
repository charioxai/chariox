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
    let same_public = previous.is_some_and(|before| {
        before.session.equivalent_except_presence(&snapshot.session)
            && public_run(before) == public_run(snapshot)
            && before.agent_activity == snapshot.agent_activity
            && before.room_workflows == snapshot.room_workflows
    });
    if same_public
        && previous.is_some_and(|before| {
            before.processes == snapshot.processes
                && before.disk_usage == snapshot.disk_usage
                && before.jobs == snapshot.jobs
        })
    {
        return Vec::new();
    }
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
        previous
            .session
            .equivalent_except_presence(&snapshot.session)
            && public_run(previous) == public_run(snapshot)
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
        let full_snapshot = KernelEvent::SessionSnapshot {
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
        };
        if previous.is_none() {
            // Establish the view before its initial resource samples arrive.
            events.insert(0, full_snapshot);
        } else {
            events.push(full_snapshot);
        }
    }
    events
}

fn public_run(snapshot: &SessionSnapshotProjection) -> Option<crate::provider::PublicProviderRun> {
    snapshot
        .provider_run
        .as_ref()
        .map(|run| crate::provider::PublicProviderRun::from(run).without_activity_timestamp())
}

/// Ordinary views need admission waits; per-process samples belong to Health.
pub(crate) fn session_projection_events_for_scope(
    snapshot: &SessionSnapshotProjection,
    previous: Option<&SessionSnapshotProjection>,
    has_terminal_workflow_update: bool,
    scope: Option<&str>,
) -> Vec<KernelEvent> {
    let events = session_projection_events(snapshot, previous, has_terminal_workflow_update);
    if scope == Some("session_terminal") {
        return Vec::new();
    }
    let same_waits =
        previous.is_some_and(|before| waiting_processes(before).eq(waiting_processes(snapshot)));
    events
        .into_iter()
        .filter(|event| {
            !(scope == Some("session_metadata")
                && same_waits
                && matches!(event, KernelEvent::SessionProcessesChanged { .. }))
        })
        .map(|mut event| {
            if scope == Some("session_metadata") {
                if let KernelEvent::SessionProcessesChanged {
                    processes,
                    disk_usage,
                    ..
                } = &mut event
                {
                    processes.retain(|process| {
                        !matches!(
                            process.state,
                            crate::runtime::process_resources::SessionProcessState::Running
                        )
                    });
                    disk_usage.clear();
                }
            }
            event
        })
        .collect()
}

fn waiting_processes(
    snapshot: &SessionSnapshotProjection,
) -> impl Iterator<
    Item = (
        &crate::runtime::process_resources::SessionProcessOwner,
        &str,
        &crate::runtime::process_resources::SessionProcessState,
    ),
> {
    snapshot
        .processes
        .iter()
        .filter(|process| {
            !matches!(
                process.state,
                crate::runtime::process_resources::SessionProcessState::Running
            )
        })
        .map(|process| {
            (
                &process.owner,
                process.process.process_id.as_str(),
                &process.state,
            )
        })
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
            jobs: Vec::new(),
            processes: Vec::new(),
            disk_usage: Vec::new(),
            room_workflows: crate::runtime::projection::RoomWorkflowInventory::project(
                &session,
                crate::session::DEFAULT_LOCAL_USER_ID,
            ),
            metadata: crate::runtime::projection::ProjectionMetadata::new(3, 1),
            session,
            provider_run: None,
            agent_activity: Default::default(),
            agent_tree_revs: Default::default(),
        }
    }

    #[test]
    fn initial_projection_supplies_the_full_session_before_output() {
        assert!(matches!(
            session_projection_events(&snapshot(), None, false).as_slice(),
            [
                KernelEvent::SessionSnapshot { .. },
                KernelEvent::SessionJobsChanged { jobs, .. },
            ] if jobs.is_empty()
        ));
    }

    #[test]
    fn a_jobs_only_change_reaches_subscribers() {
        let before = snapshot();
        let mut after = before.clone();
        after.jobs.push(crate::local::JobView {
            job_id: "job".into(),
            agent_id: "agent".into(),
            command: "cargo build".into(),
            state: "running".into(),
            placement: crate::local::JobPlacement::Local,
            started_at_ms: 1,
            machine_label: None,
            address: None,
            class_id: None,
            rate_per_minute: None,
            estimate_micros: None,
            deleted: false,
            result: None,
        });
        assert!(matches!(
            session_projection_events(&after, Some(&before), false).as_slice(),
            [KernelEvent::SessionJobsChanged { jobs, .. }] if jobs.len() == 1
        ));
    }

    #[test]
    fn mk03_return_projection_change_survives_other_deltas() {
        let mut before = snapshot();
        let mut agent = crate::agent::AgentInstance::new(
            "agent",
            "agent-ref",
            before.session.id(),
            None,
            "codex",
            Some("gpt-6-luna".into()),
            Some("low".into()),
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        );
        agent.set_remote_execution(Some(crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker".into(),
            worker_machine_id: "machine".into(),
            execution_lease_id: "lease".into(),
            leased_agent_id: "leased-agent".into(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(240),
            worker_worktree_id: None,
        }));
        before.session.set_agents(vec![agent.clone()]);
        let mut after = before.clone();
        agent.set_remote_execution(None);
        agent.set_account_profile(Some("returned-account".into()));
        after.session.set_agents(vec![agent]);
        after
            .session
            .add_active_interaction(crate::session::RuntimeInteraction::new(
                "interaction",
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
            events.iter().any(
                |event| matches!(event, KernelEvent::SessionSnapshot { session, .. }
                    if session.agents()[0].remote_execution().is_none()
                        && session.agents()[0].provider_account_profile() == "returned-account"
                )
            ),
            "Return route/account changes must reach subscribers beside another projection delta"
        );
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (transport3): every client attach, detach and reaped attachment
    // resent the whole session (4.6 KB with 50 attachment ids) to each subscription of every viewer,
    // although no view shows who is attached.
    #[test]
    fn attachment_churn_sends_no_session_snapshot() {
        let mut before = snapshot();
        before.session.add_attachment("attachment-a");
        let mut after = before.clone();
        after.session.add_attachment("attachment-b");
        assert!(session_projection_events(&after, Some(&before), false).is_empty());
        let mut used = before.clone();
        used.session.touch();
        std::thread::sleep(std::time::Duration::from_millis(2));
        used.session.touch();
        assert_ne!(
            used.session.last_used_at_ms(),
            before.session.last_used_at_ms()
        );
        assert!(
            session_projection_events(&used, Some(&before), false).is_empty(),
            "the last use alone is presence, not a Room change"
        );
        after.session.remove_attachment("attachment-a");
        after.session.set_alias(Some("after".into()));
        assert!(matches!(
            session_projection_events(&after, Some(&before), false).as_slice(),
            [KernelEvent::SessionMetadataChanged { .. }]
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
            ProcessSample, SessionProcess, SessionProcessOwner, SessionProcessState,
        };
        snapshot.processes.push(SessionProcess {
            remote_available: false,
            remote_refusal: None,
            estimate: None,
            placement: None,
            owner: SessionProcessOwner::Agent("agent".into()),
            process: ProcessSample {
                process_id: "process".into(),
                pid: 42,
                command: "provider".into(),
                resident_set_bytes: 1024,
                disk_io_bytes: Some(4096),
                disk_written_bytes: Some(4096),
                deleted_open_bytes: 0,
                outlived: false,
                cpu_millis: 0,
                sampled_at_ms: 10,
                age_ms: 10,
            },
            state: SessionProcessState::Running,
        });
    }

    #[test]
    fn initial_snapshot_precedes_its_resource_samples() {
        let mut current = snapshot();
        add_process(&mut current);
        current
            .disk_usage
            .push(crate::runtime::disk_usage::DiskUsageEntry {
                id: "workspace".into(),
                path: Some("/work".into()),
                used_bytes: Some(8192),
                sampled_at_ms: 10,
                owner: None,
                process_id: None,
            });
        let events = session_projection_events(&current, None, false);
        assert!(matches!(
            events.first(),
            Some(KernelEvent::SessionSnapshot { .. })
        ));
        assert!(events.iter().skip(1).any(|event| matches!(event, KernelEvent::SessionProcessesChanged { disk_usage, .. } if !disk_usage.is_empty())));
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

    // MP-08 / MP-09 / MP-10 / MP-11 (transport4): a resource sample that lands in the same pass as a reaped
    // attachment compared the whole session, presence included, and sent the full snapshot to each subscription.
    #[test]
    fn process_sample_beside_attachment_churn_sends_only_the_sample() {
        let mut before = snapshot();
        before.session.add_attachment("attachment-a");
        let mut after = before.clone();
        after.session.remove_attachment("attachment-a");
        after.session.add_attachment("attachment-b");
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
            "obligations":[], "wait":null, "last_progress_at_ms":1, "correction_used":false, "pending_prompt_id":null
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
    #[test]
    fn notification_trigger_availability_survives_tree_delta() {
        let mut before = snapshot();
        let trigger = crate::session::AgentTrigger::notification(
            "trigger".into(),
            "agent",
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::session::AgentTriggerNotificationSource::Workflow {
                source_id: "source".into(),
            },
            "subscription".into(),
            "source display".into(),
            "review source".into(),
            1,
        );
        before.session.add_agent_trigger(trigger);

        let mut after = before.clone();
        assert!(after.session.agent_triggers_mut()[0].set_notification_source_available(false));
        after.agent_tree_revs.insert("root".into(), 2);

        let events = session_projection_events(&after, Some(&before), false);
        assert!(events.iter().any(|event| matches!(
            event,
            KernelEvent::AgentTreeChanged { root, rev: 2, .. } if root == "root"
        )));
        assert!(
            events.iter().any(|event| matches!(
                event,
                KernelEvent::SessionSnapshot { session, .. }
                    if session.agent_triggers().iter().any(|trigger| matches!(
                        trigger.source(),
                        crate::session::AgentTriggerSource::Notification {
                            source_available: false,
                            ..
                        }
                    ))
            )),
            "the tree delta cannot carry notification availability"
        );
    }

    #[test]
    fn private_provider_updates_do_not_repeat_public_state() {
        let mut before = snapshot();
        before.provider_run = Some(
            crate::provider::RuntimeProviderRun::from_control_capability_inference(
                "run",
                before.session.id().into(),
                Some("agent".into()),
                "codex".into(),
            ),
        );
        let mut after = before.clone();
        after
            .provider_run
            .as_mut()
            .unwrap()
            .set_terminal_diagnostic("synthetic private diagnostic");
        assert!(session_projection_events(&after, Some(&before), false).is_empty());
    }

    #[test]
    fn metadata_keeps_admission_waits_while_health_keeps_samples() {
        let before = snapshot();
        let mut running = before.clone();
        add_process(&mut running);
        assert!(session_projection_events_for_scope(
            &running,
            Some(&before),
            false,
            Some("session_metadata")
        )
        .is_empty());
        assert!(matches!(
            session_projection_events_for_scope(
                &running,
                Some(&before),
                false,
                Some("session_metadata_processes")
            )
            .as_slice(),
            [KernelEvent::SessionProcessesChanged { .. }]
        ));
        let mut held = running.clone();
        held.processes[0].state = crate::runtime::process_resources::SessionProcessState::Held(
            crate::runtime::process_resources::Waiting {
                reason: crate::runtime::process_resources::WaitReason::Memory,
                position: 1,
                need_bytes: 100,
                free_bytes: 10,
                command_line: "build".into(),
                cwd: None,
                since_ms: 1,
            },
        );
        assert!(matches!(
            session_projection_events_for_scope(
                &held,
                Some(&running),
                false,
                Some("session_metadata")
            )
            .as_slice(),
            [KernelEvent::SessionProcessesChanged { .. }]
        ));
        assert!(matches!(
            session_projection_events_for_scope(
                &running,
                Some(&held),
                false,
                Some("session_metadata")
            )
            .as_slice(),
            [KernelEvent::SessionProcessesChanged { .. }]
        ));
        assert!(session_projection_events_for_scope(
            &held,
            Some(&running),
            false,
            Some("session_terminal")
        )
        .is_empty());
    }

    #[test]
    fn provider_activity_timestamp_alone_does_not_resend_run_metadata() {
        let mut before = snapshot();
        before.provider_run = Some(
            crate::provider::RuntimeProviderRun::from_control_capability_inference(
                "run",
                before.session.id().into(),
                Some("agent".into()),
                "codex".into(),
            ),
        );
        let mut after = before.clone();
        let mut value = serde_json::to_value(after.provider_run.take().unwrap()).unwrap();
        value["last_activity_at_ms"] =
            serde_json::json!(before.provider_run.as_ref().unwrap().last_activity_at_ms() + 1);
        after.provider_run = Some(serde_json::from_value(value).unwrap());
        assert!(session_projection_events(&after, Some(&before), false).is_empty());
        after.provider_run.as_mut().unwrap().mark_running();
        assert!(
            !session_projection_events(&after, Some(&before), false).is_empty(),
            "state changes still reach the view"
        );
    }

    #[test]
    fn ordinary_metadata_initial_samples_contain_only_admission_waits() {
        let mut current = snapshot();
        add_process(&mut current);
        let events =
            session_projection_events_for_scope(&current, None, false, Some("session_metadata"));
        assert!(events.iter().all(|event| !matches!(event, KernelEvent::SessionProcessesChanged { processes, disk_usage, .. } if !processes.is_empty() || !disk_usage.is_empty())));
        let health = session_projection_events_for_scope(
            &current,
            None,
            false,
            Some("session_metadata_processes"),
        );
        assert!(health.iter().any(|event| matches!(event, KernelEvent::SessionProcessesChanged { processes, .. } if !processes.is_empty())));
    }
}
