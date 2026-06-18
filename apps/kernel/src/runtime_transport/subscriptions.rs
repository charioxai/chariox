use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::error::DaemonError;
use crate::local::{RelayStatus, RemoteMachineRecord};
use crate::runtime::event_log::{ReplayGap, ReplayOutcome};
use crate::runtime::projection::SessionSnapshotProjection;
use crate::runtime::router::CommandRouter;
use crate::session::WorkflowRun;
use crate::terminal::{
    AssistantMessageCompletionRecord, RuntimeNoticeRecord, TerminalOutputRecord,
};
use crate::transport::kernel_protocol::{
    event_is_relevant_to_attachment, event_session_id, event_stream_id_for_event,
    kernel_event_trace_payload, subscription_event_stream_id, KernelEvent, KernelOutgoingFrame,
    KernelSubscriptionScope, WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
};

use super::outgoing::{try_send_outgoing_frame, KernelOutgoingSender};
use super::{
    ConnectionCloseCommand, KernelSubscription, KernelTransportRuntime, HEARTBEAT_INTERVAL_TICKS,
    RELAY_DISCOVERY_INTERVAL_TICKS, STATE_INTERVAL_TICKS, WAITING_ROOM_INVENTORY_INTERVAL_TICKS,
    WATCH_INTERVAL_MS,
};

pub(super) async fn run_subscription_loop(
    router: Arc<CommandRouter>,
    runtime: Arc<KernelTransportRuntime>,
    outgoing_tx: KernelOutgoingSender,
    close_tx: mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: Arc<AtomicBool>,
    subscription: KernelSubscription,
) {
    if subscription.subscription_scope == KernelSubscriptionScope::WaitingRoomInventory {
        run_waiting_room_inventory_subscription_loop(
            router,
            runtime,
            outgoing_tx,
            close_tx,
            close_requested,
        )
        .await;
        return;
    }
    let mut previous_snapshot: Option<SessionSnapshotProjection> = None;
    let mut previous_relay_status: Option<RelayStatus> = None;
    let mut previous_remote_machines: Option<Vec<RemoteMachineRecord>> = None;
    let mut previous_inventory_version: Option<String> = None;
    let mut last_workflow_design_sequence = 0_u64;
    let mut tick: u64 = 0;
    let event_stream_id =
        subscription_event_stream_id(&subscription.session_id, &subscription.attachment_id);

    loop {
        let watch_result = router
            .relay_watch_subscription_state(
                &subscription.session_id,
                &subscription.attachment_id,
                tick,
                previous_snapshot.clone(),
                last_workflow_design_sequence,
            )
            .await;

        match watch_result {
            WatchResult::Ok {
                records,
                notices,
                completions,
                workflow_design_events,
                snapshot,
            } => {
                if !records.is_empty()
                    && !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::TerminalOutput { records },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
                }
                if !notices.is_empty()
                    && !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::RuntimeNotices { notices },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
                }
                for completion in completions {
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::AssistantMessageCompleted {
                            session_id: completion.session_id,
                            provider_run_id: completion.provider_run_id,
                            agent_id: completion.agent_id,
                            recipient_attachment_ids: completion.recipient_attachment_ids,
                            message_id: completion.message_id,
                            completed_at_ms: completion.completed_at_ms,
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                    {
                        break;
                    }
                }
                for workflow_event in workflow_design_events {
                    last_workflow_design_sequence =
                        last_workflow_design_sequence.max(workflow_event.kernel_sequence);
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::WorkflowDesignOp {
                            design_op: workflow_event,
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                    {
                        break;
                    }
                }
                if let Some(snapshot) = *snapshot {
                    let workflow_run_updates =
                        changed_workflow_runs(previous_snapshot.as_ref(), &snapshot);
                    for workflow_run in workflow_run_updates {
                        if !emit_kernel_event(
                            &runtime,
                            &outgoing_tx,
                            &close_tx,
                            &close_requested,
                            KernelEvent::WorkflowRunUpdated {
                                session_id: snapshot.session.id().to_string(),
                                workflow_run,
                            },
                            Some(&event_stream_id),
                            Some(&subscription.session_id),
                            Some(&subscription.attachment_id),
                        )
                        .await
                        {
                            break;
                        }
                    }
                    previous_snapshot = Some(snapshot.clone());
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::SessionSnapshot {
                            session: Box::new(snapshot.session),
                            provider_run: Box::new(snapshot.provider_run),
                            agent_activity: Box::new(snapshot.agent_activity),
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                    {
                        break;
                    }
                }
                if tick.is_multiple_of(HEARTBEAT_INTERVAL_TICKS)
                    && !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::Heartbeat {
                            session_id: subscription.session_id.clone(),
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
                }
                if tick.is_multiple_of(HEARTBEAT_INTERVAL_TICKS) {
                    let status = router.transport_relay_status_snapshot().await;
                    if previous_relay_status.as_ref() != Some(&status) {
                        previous_relay_status = Some(status.clone());
                        if !emit_kernel_event(
                            &runtime,
                            &outgoing_tx,
                            &close_tx,
                            &close_requested,
                            KernelEvent::RelayStatusChanged { status },
                            Some(&event_stream_id),
                            Some(&subscription.session_id),
                            Some(&subscription.attachment_id),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
                if tick.is_multiple_of(RELAY_DISCOVERY_INTERVAL_TICKS) {
                    let machines = router.transport_remote_machines_snapshot();
                    if previous_remote_machines.as_ref() != Some(&machines) {
                        previous_remote_machines = Some(machines.clone());
                        if !emit_kernel_event(
                            &runtime,
                            &outgoing_tx,
                            &close_tx,
                            &close_requested,
                            KernelEvent::RemoteMachinesChanged { machines },
                            Some(&event_stream_id),
                            Some(&subscription.session_id),
                            Some(&subscription.attachment_id),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
                if tick.is_multiple_of(WAITING_ROOM_INVENTORY_INTERVAL_TICKS) {
                    match router.waiting_room_inventory_version().await {
                        Ok(inventory_version) => {
                            if previous_inventory_version.as_ref() != Some(&inventory_version) {
                                previous_inventory_version = Some(inventory_version.clone());
                                if !emit_kernel_event(
                                    &runtime,
                                    &outgoing_tx,
                                    &close_tx,
                                    &close_requested,
                                    KernelEvent::WaitingRoomInventoryChanged { inventory_version },
                                    Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE),
                                    Some(&subscription.session_id),
                                    Some(&subscription.attachment_id),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        Err(error) => {
                            crate::logging::warn_with_fields(
                                "daemon.runtime_transport",
                                "kernel event loop failed to build waiting-room inventory version",
                                serde_json::json!({
                                    "session_id": subscription.session_id,
                                    "attachment_id": subscription.attachment_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                }
            }
            WatchResult::Unavailable(message) => {
                let _ = emit_kernel_event(
                    &runtime,
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    KernelEvent::SessionUnavailable {
                        session_id: subscription.session_id.clone(),
                        message,
                    },
                    Some(&event_stream_id),
                    Some(&subscription.session_id),
                    Some(&subscription.attachment_id),
                )
                .await;
                break;
            }
        }

        tick = tick.wrapping_add(1);
        sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
    }
}

fn changed_workflow_runs(
    previous_snapshot: Option<&SessionSnapshotProjection>,
    snapshot: &SessionSnapshotProjection,
) -> Vec<WorkflowRun> {
    let Some(previous_snapshot) = previous_snapshot else {
        return snapshot.session.workflow_runs().to_vec();
    };
    let previous_runs = previous_snapshot
        .session
        .workflow_runs()
        .iter()
        .map(|run| (run.id(), run))
        .collect::<std::collections::BTreeMap<_, _>>();
    snapshot
        .session
        .workflow_runs()
        .iter()
        .filter(|run| previous_runs.get(run.id()).copied() != Some(*run))
        .cloned()
        .collect()
}

fn build_session_snapshot(
    app: &mut crate::app::DaemonApp,
    session_id: &str,
    attachment_owner_user_id: &str,
) -> Result<SessionSnapshotProjection, DaemonError> {
    let mut snapshot = SessionSnapshotProjection::from_daemon_app(app, session_id, 0)?;
    snapshot.session = snapshot.session.redacted_for_user(attachment_owner_user_id);
    snapshot.agent_activity.retain(|agent_id, _| {
        snapshot
            .session
            .agents()
            .iter()
            .any(|agent| agent.id() == agent_id)
    });
    if snapshot
        .provider_run
        .as_ref()
        .and_then(|run| run.agent_instance_id())
        .is_some_and(|agent_id| {
            !snapshot
                .session
                .agents()
                .iter()
                .any(|agent| agent.id() == agent_id)
        })
    {
        snapshot.provider_run = None;
    }
    Ok(snapshot)
}

pub(crate) enum WatchResult {
    Ok {
        records: Vec<TerminalOutputRecord>,
        notices: Vec<RuntimeNoticeRecord>,
        completions: Vec<AssistantMessageCompletionRecord>,
        workflow_design_events: Vec<crate::local::WorkflowDesignOpForwarded>,
        snapshot: Box<Option<SessionSnapshotProjection>>,
    },
    Unavailable(String),
}

pub(crate) fn watch_subscription_state(
    app: &mut crate::app::DaemonApp,
    session_id: &str,
    attachment_id: &str,
    tick: u64,
    previous_snapshot: Option<SessionSnapshotProjection>,
    last_workflow_design_sequence: u64,
) -> WatchResult {
    let attachment = if let Ok(attachment) = crate::app::KernelSessionReadService::new(app)
        .ensure_attachment_in_session(session_id, attachment_id)
    {
        attachment
    } else {
        return WatchResult::Unavailable("Current session is no longer available.".to_string());
    };
    let attachment_owner_user_id = attachment.owner_user_id().to_string();

    let records = match crate::app::provider_output::pump_terminal_output_for_attachment(
        app,
        session_id,
        attachment_id,
    ) {
        Ok(records) => records,
        Err(DaemonError::NoActiveProviderRun { .. }) => Vec::new(),
        Err(DaemonError::SessionNotFound { .. })
        | Err(DaemonError::AttachmentNotFound { .. })
        | Err(DaemonError::AttachmentNotInSession { .. }) => {
            return WatchResult::Unavailable("Current session is no longer available.".to_string());
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.runtime_transport",
                "kernel event loop failed to pump terminal output",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "error": error.to_string(),
                }),
            );
            Vec::new()
        }
    };

    let notices = app
        .terminal_mut()
        .drain_notice_records(session_id, attachment_id);
    let completions = app
        .terminal_mut()
        .drain_completion_records(session_id, attachment_id);
    let workflow_design_events = app.workflow_design_event_store().events_since(
        session_id,
        last_workflow_design_sequence,
        attachment_id,
    );
    let snapshot = if tick.is_multiple_of(STATE_INTERVAL_TICKS) {
        match build_session_snapshot(app, session_id, &attachment_owner_user_id) {
            Ok(snapshot) => {
                if previous_snapshot.as_ref() != Some(&snapshot) {
                    Box::new(Some(snapshot))
                } else {
                    Box::new(None)
                }
            }
            Err(DaemonError::SessionNotFound { .. }) => {
                return WatchResult::Unavailable(
                    "Current session is no longer available.".to_string(),
                );
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel event loop failed to build session snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "error": error.to_string(),
                    }),
                );
                Box::new(None)
            }
        }
    } else {
        Box::new(None)
    };

    WatchResult::Ok {
        records,
        notices,
        completions,
        workflow_design_events,
        snapshot,
    }
}

async fn emit_kernel_event(
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &KernelOutgoingSender,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    event: KernelEvent,
    event_stream_id: Option<&str>,
    session_id: Option<&str>,
    attachment_id: Option<&str>,
) -> bool {
    let stream_id = event_stream_id
        .map(str::to_string)
        .or_else(|| event_stream_id_for_event(&event, session_id));
    let event_id = if let Some(stream_id) = stream_id.as_deref() {
        match runtime
            .event_log
            .append(stream_id.to_string(), event.clone())
            .await
        {
            Ok(logged) => logged.event_id,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "failed to reserve kernel event id",
                    serde_json::json!({
                        "stream_id": stream_id,
                        "error": error.to_string(),
                    }),
                );
                return false;
            }
        }
    } else {
        match runtime.event_log.append("daemon", event.clone()).await {
            Ok(logged) => logged.event_id,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "failed to reserve kernel event id",
                    serde_json::json!({
                        "stream_id": "daemon",
                        "error": error.to_string(),
                    }),
                );
                return false;
            }
        }
    };
    if let Some(trace_session_id) = event_session_id(&event).or(session_id) {
        crate::debug_trace::record_terminal_turn(
            trace_session_id,
            "kernel_event_emit",
            kernel_event_trace_payload(event_id, &event),
        );
    }
    runtime.transport_health.record_emitted_event();
    try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        &runtime.transport_health,
        KernelOutgoingFrame::Event {
            event_id,
            event: Box::new(event),
        },
        session_id,
        attachment_id,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplaySubscriptionResult {
    NoCursor,
    Complete,
    Gap(ReplayGap),
    Overflow,
}

pub(super) async fn replay_recent_events(
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &KernelOutgoingSender,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    session_id: &str,
    attachment_id: &str,
    resume_from_event_id: Option<u64>,
) -> ReplaySubscriptionResult {
    let Some(cursor) = resume_from_event_id else {
        return ReplaySubscriptionResult::NoCursor;
    };
    let stream_id = subscription_event_stream_id(session_id, attachment_id);
    let replay = runtime.event_log.replay_after(&stream_id, cursor).await;

    let events = match replay {
        ReplayOutcome::Replayed(events) => events,
        ReplayOutcome::Gap(gap) => {
            runtime.transport_health.record_replay_gap();
            let _ = emit_kernel_event(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                KernelEvent::ReplayGap {
                    session_id: session_id.to_string(),
                    requested_from_event_id: gap.requested_from_event_id,
                    first_retained_event_id: gap.first_retained_event_id,
                    latest_event_id: gap.latest_event_id,
                    message: "Replay cursor is outside the retained kernel event window; refresh the session projection.".to_string(),
                },
                Some(&stream_id),
                Some(session_id),
                Some(attachment_id),
            )
            .await;
            return ReplaySubscriptionResult::Gap(gap);
        }
    };

    for persisted in events {
        if !event_is_relevant_to_attachment(&persisted.event, attachment_id) {
            continue;
        }
        if !try_send_outgoing_frame(
            outgoing_tx,
            close_tx,
            close_requested,
            &runtime.transport_health,
            KernelOutgoingFrame::Event {
                event_id: persisted.event_id,
                event: Box::new(persisted.event.clone()),
            },
            Some(session_id),
            Some(attachment_id),
        ) {
            return ReplaySubscriptionResult::Overflow;
        }
    }

    if !try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        &runtime.transport_health,
        KernelOutgoingFrame::Event {
            event_id: match runtime
                .event_log
                .append(
                    stream_id,
                    KernelEvent::TransportResumed {
                        session_id: session_id.to_string(),
                        resumed_from_event_id: Some(cursor),
                    },
                )
                .await
            {
                Ok(logged) => logged.event_id,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.runtime_transport",
                        "failed to reserve transport-resumed event id",
                        serde_json::json!({
                            "session_id": session_id,
                            "attachment_id": attachment_id,
                            "error": error.to_string(),
                        }),
                    );
                    return ReplaySubscriptionResult::Overflow;
                }
            },
            event: Box::new(KernelEvent::TransportResumed {
                session_id: session_id.to_string(),
                resumed_from_event_id: Some(cursor),
            }),
        },
        Some(session_id),
        Some(attachment_id),
    ) {
        return ReplaySubscriptionResult::Overflow;
    }
    ReplaySubscriptionResult::Complete
}

pub(super) async fn emit_replay_gap_snapshot(
    router: &Arc<CommandRouter>,
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &KernelOutgoingSender,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    session_id: &str,
    attachment_id: &str,
) {
    let event_stream_id = subscription_event_stream_id(session_id, attachment_id);
    let snapshot = router.session_snapshot_projection_for_attachment(session_id, attachment_id, 0);
    match snapshot {
        Ok(projection) => {
            let _ = emit_kernel_event(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                KernelEvent::SessionSnapshot {
                    session: Box::new(projection.session),
                    provider_run: Box::new(projection.provider_run),
                    agent_activity: Box::new(projection.agent_activity),
                },
                Some(&event_stream_id),
                Some(session_id),
                Some(attachment_id),
            )
            .await;
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.runtime_transport",
                "kernel replay gap snapshot failed",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

async fn run_waiting_room_inventory_subscription_loop(
    router: Arc<CommandRouter>,
    runtime: Arc<KernelTransportRuntime>,
    outgoing_tx: KernelOutgoingSender,
    close_tx: mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: Arc<AtomicBool>,
) {
    let mut previous_inventory_version: Option<String> = None;
    loop {
        match router.waiting_room_inventory_version().await {
            Ok(inventory_version) => {
                if previous_inventory_version.as_ref() != Some(&inventory_version) {
                    previous_inventory_version = Some(inventory_version.clone());
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::WaitingRoomInventoryChanged { inventory_version },
                        Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE),
                        None,
                        None,
                    )
                    .await
                    {
                        break;
                    }
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel waiting-room inventory subscription failed to build version",
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
        }
        sleep(Duration::from_millis(
            WATCH_INTERVAL_MS * WAITING_ROOM_INVENTORY_INTERVAL_TICKS,
        ))
        .await;
    }
}
