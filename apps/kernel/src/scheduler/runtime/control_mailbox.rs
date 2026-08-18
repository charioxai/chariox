//! Workflow control mailbox storage used for routed failure notifications.

use crate::app::DaemonApp;
use crate::session::{WorkflowFailureEvent, WorkflowFailureKind, WorkflowRun};

pub(super) fn workflow_node_control_contents(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
) -> Option<String> {
    let root = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-control",
    )?;
    let path = root.join(format!("node-{node_id}.md"));
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn write_workflow_control_mailbox_entry(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
    failure: &WorkflowFailureEvent,
) {
    let Some(root) = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        failure.source_node_run_id(),
        "workflow-control",
    ) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(&root) {
        tracing::debug!(
            ?error,
            "Failed to create workflow control directory at {:?}",
            root
        );
        return;
    }
    let path = root.join(format!("node-{node_id}.md"));
    let existing = std::fs::read_to_string(&path).ok();
    let header = "# Workflow Control Mailbox\n\n";
    let existing_body = existing
        .as_deref()
        .unwrap_or("")
        .strip_prefix(header)
        .unwrap_or("")
        .trim();
    let edge_label = if failure.edge_ids().is_empty() {
        "node-local".to_string()
    } else {
        failure
            .edge_ids()
            .iter()
            .map(|edge_id| format!("edge {edge_id}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let entry = format!(
        "- [{} @ {}] {}: {}",
        match failure.kind() {
            WorkflowFailureKind::MissingAck => "missing_ack",
            WorkflowFailureKind::MissingStructuredOutput => "missing_structured_output",
            WorkflowFailureKind::OutputValidationFailed => "output_validation_failed",
            WorkflowFailureKind::WorkflowRunOutputValidationFailed => {
                "workflow_run_output_validation_failed"
            }
            WorkflowFailureKind::NodeTurnBudgetExhausted => "node_turn_budget_exhausted",
            WorkflowFailureKind::RunStopped => "run_stopped",
            WorkflowFailureKind::ProviderFailure => "provider_failure",
            WorkflowFailureKind::TransportFailure => "transport_failure",
            WorkflowFailureKind::TurnStalled => "turn_stalled",
        },
        failure.timestamp_ms(),
        edge_label,
        failure.message()
    );
    let body = if existing_body.is_empty() {
        entry
    } else if existing_body.lines().any(|line| line.trim() == entry) {
        existing_body.to_string()
    } else {
        format!("{existing_body}\n{entry}")
    };
    let content = format!("{header}Notifications for node `{node_id}`:\n{body}\n");
    if let Err(error) = std::fs::write(&path, content) {
        tracing::debug!(
            ?error,
            "Failed to write workflow control mailbox at {:?}",
            path
        );
    }
}

pub(super) fn clear_workflow_control_mailbox(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    workflow_run: &WorkflowRun,
) {
    let Some(node_id) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
        .map(|node_run| node_run.node_id())
    else {
        return;
    };
    let Some(root) = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-control",
    ) else {
        return;
    };
    let path = root.join(format!("node-{node_id}.md"));
    let _ = std::fs::remove_file(path);
}

fn workflow_runtime_artifact_root(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    category: &str,
) -> Option<std::path::PathBuf> {
    validate_workflow_node_run(app, session_id, workflow_run_id, workflow_node_run_id)?;
    Some(
        app.config()
            .workflow_runtime_artifact_root()
            .join(session_id)
            .join(workflow_run_id)
            .join(category),
    )
}

fn validate_workflow_node_run(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> Option<()> {
    let session = app.sessions().get_session(session_id).ok()?;
    let workflow_run = session.workflow_run(workflow_run_id)?;
    workflow_run
        .node_runs()
        .iter()
        .find(|candidate| candidate.id() == workflow_node_run_id)?;
    Some(())
}
