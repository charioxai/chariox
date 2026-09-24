//! Legacy entry deferral to the existing owned workflow scheduler.
use super::{DaemonApp, DaemonError};

/// Legacy entry points may observe a session managed by the owned workflow
/// timer. Deferral preserves its queued/Ready work without reporting failure or
/// introducing a second prompt-admission authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkflowSchedulerOwner {
    Legacy,
    Owned,
}

pub(crate) fn workflow_entry_scheduler_owner(
    app: &DaemonApp,
    session: &str,
    run: &str,
    node: &str,
    allow_submitted_resume: bool,
) -> Result<WorkflowSchedulerOwner, DaemonError> {
    let store = app.durable_state_store();
    store.require_writer_healthy()?;
    Ok(
        if store
            .workflow_dispatch_intent(&app.config().daemon_id, session, run)?
            .is_some_and(|intent| {
                intent.node_id == node && (!intent.submitted || !allow_submitted_resume)
            })
        {
            WorkflowSchedulerOwner::Owned
        } else {
            WorkflowSchedulerOwner::Legacy
        },
    )
}

pub(super) fn workflow_queue_scheduler_owner(
    store: &crate::durable_state::DurableKernelStateStore,
    session: &crate::session::RuntimeSession,
) -> Result<WorkflowSchedulerOwner, DaemonError> {
    store.require_writer_healthy()?;
    let has_app_queue = session.workflow_queued_prompts().iter().any(|queued| {
        queued
            .publication_invocation()
            .is_some_and(|invocation| invocation.transport == "app_event")
    });
    Ok(
        if has_app_queue
            || store
                .pending_workflow_dispatch_intent(session.host_daemon_id(), session.id())?
                .is_some()
        {
            WorkflowSchedulerOwner::Owned
        } else {
            WorkflowSchedulerOwner::Legacy
        },
    )
}
