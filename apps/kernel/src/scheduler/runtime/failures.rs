//! Workflow failure recording and mailbox routing policy.

use crate::app::DaemonApp;
use crate::session::{WorkflowFailureEvent, WorkflowFailurePolicy, WorkflowFailurePolicyMode};

use super::control_mailbox::write_workflow_control_mailbox_entry;

pub(super) fn provider_run_terminal_diagnostic(
    app: &DaemonApp,
    provider_run_id: &str,
) -> Option<String> {
    app.providers()
        .get_run(provider_run_id)
        .ok()
        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
        .filter(|message| !message.trim().is_empty())
}

pub(super) fn record_and_route_workflow_failure(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    failure: &WorkflowFailureEvent,
) {
    let _ = app.sessions_mut().record_workflow_failure_event(
        session_id,
        workflow_run_id,
        failure.clone(),
    );
    let policy = workflow_failure_policy();
    if policy.mode() != WorkflowFailurePolicyMode::Notify {
        return;
    }
    route_workflow_failure_mailboxes(app, session_id, workflow_run_id, failure, &policy);
}

fn workflow_failure_policy() -> WorkflowFailurePolicy {
    WorkflowFailurePolicy::default()
}

fn route_workflow_failure_mailboxes(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    failure: &WorkflowFailureEvent,
    policy: &WorkflowFailurePolicy,
) {
    let workflow_run = match app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
    {
        Ok(run) => run,
        Err(_) => return,
    };
    let workflow = match app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
    {
        Ok(workflow) => workflow,
        Err(_) => return,
    };
    let Some(source_node_run) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == failure.source_node_run_id())
    else {
        return;
    };
    if policy.notify_source_node() {
        write_workflow_control_mailbox_entry(
            app,
            session_id,
            workflow_run_id,
            source_node_run.node_id(),
            failure,
        );
    }
    if !policy.notify_sink_nodes() {
        return;
    }
    for edge_id in failure.edge_ids() {
        let Some(edge) = workflow.edge(edge_id) else {
            continue;
        };
        write_workflow_control_mailbox_entry(
            app,
            session_id,
            workflow_run_id,
            edge.to_node_id(),
            failure,
        );
    }
}
