//! How a notifying exit is named to its subscribers.
use super::*;
use crate::session::WorkflowDefinition;

/// The workflow's alias (or id) when it has one notifying exit, otherwise
/// `<alias>-<ordinal>`, the number the Exit inspector shows as `exit-<ordinal>`.
pub(crate) fn display_name(name: &str, ordinal: u32, notifying_exits: usize) -> String {
    if notifying_exits > 1 {
        format!("{name}-{ordinal}")
    } else {
        name.to_owned()
    }
}

/// The name of a workflow's "run finished" source.
pub(crate) fn run_display_name(name: &str) -> String {
    format!("{name} run finished")
}

/// 1-based position of `exit_node_id` among the workflow's exits.
pub(crate) fn exit_ordinal(workflow: &WorkflowDefinition, exit_node_id: &str) -> Option<u32> {
    workflow
        .nodes()
        .iter()
        .filter(|node| node.can_complete_workflow_run())
        .position(|node| node.id() == exit_node_id)
        .map(|index| index as u32 + 1)
}

/// An exit named by its node id, or by the Exit inspector's `exit-<ordinal>`.
pub(crate) fn resolve_exit(workflow: &WorkflowDefinition, reference: &str) -> Option<String> {
    let exits = || {
        workflow
            .nodes()
            .iter()
            .filter(|node| node.can_complete_workflow_run())
    };
    exits()
        .find(|node| node.id() == reference)
        .or_else(|| {
            let ordinal: usize = reference.strip_prefix("exit-")?.parse().ok()?;
            exits().nth(ordinal.checked_sub(1)?)
        })
        .map(|node| node.id().to_owned())
}

pub(crate) fn workflow_name(workflow: &WorkflowDefinition) -> String {
    workflow.alias().unwrap_or(workflow.id()).to_owned()
}

/// Display name of a source, derived from its workflow's current definition.
pub(super) fn display_name_in(
    tx: &Transaction<'_>,
    source: &WorkflowNotificationSource,
) -> rusqlite::Result<String> {
    let workflow: Option<String> = tx.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='workflow' AND entity_id=?3",params![source.kernel_id,source.session_id,source.workflow_id],|r|r.get(0)).optional()?;
    let workflow: Option<WorkflowDefinition> =
        workflow.and_then(|json| serde_json::from_str(&json).ok());
    let notifying: i64 = tx.query_row("SELECT count(*) FROM workflow_notification_exit_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND enabled=1 AND json_extract(payload_json,'$.kind')<>'workflow_run'",params![source.kernel_id,source.session_id,source.workflow_id],|r|r.get(0))?;
    Ok(match workflow {
        Some(workflow) if source.kind == crate::local::NotificationSourceKind::WorkflowRun => {
            run_display_name(&workflow_name(&workflow))
        }
        Some(workflow) => display_name(
            &workflow_name(&workflow),
            exit_ordinal(&workflow, &source.exit_node_id).unwrap_or(0),
            notifying as usize,
        ),
        None => source.name.clone(),
    })
}
