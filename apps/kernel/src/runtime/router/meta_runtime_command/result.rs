use super::summary::*;
use super::*;

pub(super) fn meta_command_requires_task_plan(tokens: &[String]) -> bool {
    match tokens.first().map(String::as_str) {
        Some("prompt") => true,
        Some("agent") => matches!(tokens.get(1).map(String::as_str), Some("spawn")),
        Some("workflow") => matches!(
            tokens.get(1).map(String::as_str),
            Some("new")
                | Some("alias")
                | Some("node")
                | Some("endpoint")
                | Some("edge")
                | Some("run")
                | Some("cancel")
                | Some("resume")
        ),
        _ => false,
    }
}

pub(super) fn metaagent_active_task_plan_is_empty(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
) -> bool {
    session.metaagent_task(metaagent.id()).is_some_and(|task| {
        task.status() == crate::session::MetaagentTaskStatus::Active
            && task.plan_markdown().trim().is_empty()
    })
}

pub(super) fn meta_task_plan_required_error() -> DaemonError {
    meta_command_error(
        "active meta task has no plan; call `chariox.meta.update_plan` with a concise plan before delegating work through run_command",
    )
}

pub(super) fn meta_command_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_meta.run_command",
        message: message.into(),
    }
}

pub(super) fn meta_command_failure_result(command: &str, error: DaemonError) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "command": redacted_meta_command_for_payload(command),
            "error": error.to_string(),
        }),
    }
}

pub(super) fn meta_command_success_result(
    command: &str,
    response: &LocalDaemonResponse,
    metaagent: &crate::agent::AgentInstance,
) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "command": redacted_meta_command_for_payload(command),
            "response": summarize_meta_command_response(response, metaagent),
        }),
    }
}

pub(super) fn redacted_meta_command_for_payload(command: &str) -> String {
    let Ok(tokens) = crate::runtime::metaagent_command_registry::tokenize_command(command) else {
        return command.to_string();
    };
    match (
        tokens.first().map(String::as_str),
        tokens.get(1).map(String::as_str),
    ) {
        (Some("credential" | "credentials"), Some("set" | "set-secret" | "delete-secret")) => {
            let credential_ref = tokens.get(2).map_or("<credential-ref>", String::as_str);
            format!(
                "{} {} {} <redacted-secret>",
                tokens[0], tokens[1], credential_ref
            )
        }
        _ => command.to_string(),
    }
}

fn summarize_meta_command_response(
    response: &LocalDaemonResponse,
    metaagent: &crate::agent::AgentInstance,
) -> serde_json::Value {
    match response {
        LocalDaemonResponse::AgentSpawned { agent } => serde_json::json!({
            "type": "AgentSpawned",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentAliased { agent, .. } => serde_json::json!({
            "type": "AgentAliased",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentDestroyed { agent } => serde_json::json!({
            "type": "AgentDestroyed",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentFocused { agent } => serde_json::json!({
            "type": "AgentFocused",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentsListed { agents } => serde_json::json!({
            "type": "AgentsListed",
            "agents": agents
                .iter()
                .filter(|agent| {
                    !agent.is_metaagent()
                        && agent.controlled_by_metaagent_id() == Some(metaagent.id())
                })
                .map(summarize_meta_agent)
                .collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => serde_json::json!({
            "type": "WorkflowCreated",
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowAliased { workflow, .. }
        | LocalDaemonResponse::WorkflowResolved { workflow } => serde_json::json!({
            "type": "Workflow",
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowsListed { workflows } => serde_json::json!({
            "type": "WorkflowsListed",
            "workflows": workflows.iter().map(summarize_meta_workflow).collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowNodeAdded { node, workflow, .. } => serde_json::json!({
            "type": "WorkflowNodeAdded",
            "node": summarize_meta_workflow_node(node),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowNodeRemoved { node, workflow, .. } => serde_json::json!({
            "type": "WorkflowNodeRemoved",
            "node": summarize_meta_workflow_node(node),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeInstructionsUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeCanCompleteRunUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
            node,
            workflow,
            ..
        } => serde_json::json!({
            "type": "WorkflowNodeCanEmitIntermediateOutputUpdated",
            "node": summarize_meta_workflow_node(node),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeWaitForAllInputsUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeMaxTurnsUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint, workflow, ..
        } => serde_json::json!({
            "type": "WorkflowEndpointCreated",
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint, workflow, ..
        } => serde_json::json!({
            "type": "WorkflowEndpointAliased",
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowEdgeAdded { edge, workflow, .. } => serde_json::json!({
            "type": "WorkflowEdgeAdded",
            "edge": summarize_meta_workflow_edge(edge),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowEdgeRemoved { edge, workflow, .. } => serde_json::json!({
            "type": "WorkflowEdgeRemoved",
            "edge": summarize_meta_workflow_edge(edge),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowRunInvoked {
            workflow_run,
            workflow,
            endpoint,
            ..
        } => serde_json::json!({
            "type": "WorkflowRunInvoked",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
            "workflow": summarize_meta_workflow(workflow),
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
        }),
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => serde_json::json!({
            "type": "WorkflowRunsListed",
            "workflow_runs": workflow_runs
                .iter()
                .map(summarize_meta_workflow_run)
                .collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowRun { workflow_run } => serde_json::json!({
            "type": "WorkflowRun",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => serde_json::json!({
            "type": "WorkflowRunCancelled",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        LocalDaemonResponse::WorkflowRunPaused { workflow_run, .. } => serde_json::json!({
            "type": "WorkflowRunPaused",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        LocalDaemonResponse::WorkflowRunResumed { workflow_run, .. } => serde_json::json!({
            "type": "WorkflowRunResumed",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        _ => serde_json::json!({
            "type": "CommandAccepted",
            "detail": "response omitted from metaagent tool output; inspect session_overview or a dedicated list command for current state",
        }),
    }
}
