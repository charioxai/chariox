pub(super) fn summarize_meta_agent(agent: &crate::agent::AgentInstance) -> serde_json::Value {
    serde_json::json!({
        "id": agent.id(),
        "agent_ref": agent.agent_ref(),
        "alias": agent.alias(),
        "role": agent.role(),
        "provider": agent.provider(),
        "model": agent.model(),
    })
}

pub(super) fn summarize_meta_workflow(
    workflow: &crate::session::WorkflowDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": workflow.id(),
        "alias": workflow.alias(),
        "revision": workflow.revision(),
        "nodes": workflow
            .nodes()
            .iter()
            .map(summarize_meta_workflow_node)
            .collect::<Vec<_>>(),
        "edges": workflow
            .edges()
            .iter()
            .map(summarize_meta_workflow_edge)
            .collect::<Vec<_>>(),
        "endpoints": workflow
            .endpoints()
            .iter()
            .map(summarize_meta_workflow_endpoint)
            .collect::<Vec<_>>(),
    })
}

pub(super) fn summarize_meta_workflow_node(
    node: &crate::session::WorkflowNodeDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": node.id(),
        "agent_id": node.agent_id(),
        "public_label": node.public_label(),
        "can_complete_workflow_run": node.can_complete_workflow_run(),
        "can_emit_intermediate_run_output": node.can_emit_intermediate_run_output(),
        "wait_for_all_inputs": node.wait_for_all_inputs(),
    })
}

pub(super) fn summarize_meta_workflow_edge(
    edge: &crate::session::WorkflowEdgeDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": edge.id(),
        "from_node_id": edge.from_node_id(),
        "to_node_id": edge.to_node_id(),
        "source_side": edge.source_side(),
        "target_side": edge.target_side(),
        "handoff_schema_ref": edge.handoff_schema_ref(),
        "validation_policy": edge.validation_policy(),
    })
}

pub(super) fn summarize_meta_workflow_endpoint(
    endpoint: &crate::session::WorkflowEndpointDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": endpoint.id(),
        "alias": endpoint.alias(),
        "entry_node_id": endpoint.entry_node_id(),
    })
}

pub(super) fn summarize_meta_workflow_run(run: &crate::session::WorkflowRun) -> serde_json::Value {
    let active_node_run = run
        .active_node_run_id()
        .and_then(|active_node_run_id| {
            run.node_runs()
                .iter()
                .find(|node_run| node_run.id() == active_node_run_id)
        })
        .map(summarize_meta_workflow_node_run);
    let unconsumed_messages = run
        .messages()
        .iter()
        .filter(|message| message.consumed_by_node_run_id().is_none())
        .count();
    let latest_failure = run
        .failure_events()
        .last()
        .map(summarize_meta_workflow_failure);
    let latest_intermediate_output = run
        .intermediate_outputs()
        .last()
        .map(summarize_meta_workflow_intermediate_output);
    serde_json::json!({
        "id": run.id(),
        "workflow_id": run.workflow_id(),
        "endpoint_id": run.endpoint_id(),
        "entry_node_id": run.entry_node_id(),
        "status": run.status(),
        "invocation_prompt_present": run.invocation_prompt().is_some(),
        "active_node_run_id": run.active_node_run_id(),
        "active_node_run": active_node_run,
        "node_runs": run
            .node_runs()
            .iter()
            .map(summarize_meta_workflow_node_run)
            .collect::<Vec<_>>(),
        "node_run_counts_by_status": summarize_meta_workflow_node_run_counts(run),
        "message_count": run.messages().len(),
        "unconsumed_message_count": unconsumed_messages,
        "messages": run
            .messages()
            .iter()
            .map(summarize_meta_workflow_message)
            .collect::<Vec<_>>(),
        "failure_count": run.failure_events().len(),
        "latest_failure": latest_failure,
        "failure_events": run
            .failure_events()
            .iter()
            .map(summarize_meta_workflow_failure)
            .collect::<Vec<_>>(),
        "intermediate_output_count": run.intermediate_outputs().len(),
        "latest_intermediate_output": latest_intermediate_output,
        "final_output_present": run.final_output().is_some(),
        "final_output_valid": run.final_output_valid(),
        "final_output_warning": run.final_output_warning(),
        "final_output": run.final_output().map(summarize_meta_workflow_output_payload),
        "completed_by_node_run_id": run.completed_by_node_run_id(),
    })
}

fn summarize_meta_workflow_node_run(
    node_run: &crate::session::WorkflowNodeRun,
) -> serde_json::Value {
    let turn = node_run.turn_envelope();
    let completion = node_run.completion();
    serde_json::json!({
        "id": node_run.id(),
        "node_id": node_run.node_id(),
        "agent_id": node_run.agent_id(),
        "status": node_run.status(),
        "summary": node_run.summary(),
        "created_at_ms": node_run.created_at_ms(),
        "started_at_ms": node_run.started_at_ms(),
        "completed_at_ms": node_run.completed_at_ms(),
        "completion": completion.map(summarize_meta_workflow_completion),
        "turn": turn.map(summarize_meta_workflow_turn),
        "thinking_trace_count": node_run.thinking_traces().len(),
        "has_valid_pending_final_output": node_run.has_valid_pending_final_output(),
    })
}

fn summarize_meta_workflow_node_run_counts(run: &crate::session::WorkflowRun) -> serde_json::Value {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for node_run in run.node_runs() {
        *counts
            .entry(format!("{:?}", node_run.status()))
            .or_default() += 1;
    }
    serde_json::json!(counts)
}

fn summarize_meta_workflow_turn(turn: &crate::session::WorkflowTurnEnvelope) -> serde_json::Value {
    serde_json::json!({
        "delivery_token": turn.delivery_token(),
        "state": turn.state(),
        "rendered_prompt_present": turn.rendered_prompt().is_some(),
        "mailbox_content_present": turn.mailbox_content().is_some(),
        "handoff_payloads_present": turn.handoff_payloads_json().is_some(),
        "runtime_tool_call_count": turn.runtime_tool_calls().len(),
        "pending_output_submissions": turn
            .pending_output_submissions()
            .map(summarize_meta_workflow_pending_outputs),
    })
}

fn summarize_meta_workflow_pending_outputs(
    submissions: &crate::session::WorkflowTurnOutputSubmissions,
) -> serde_json::Value {
    serde_json::json!({
        "intermediate": submissions
            .intermediate()
            .map(summarize_meta_workflow_output_submission),
        "final": submissions
            .final_output()
            .map(summarize_meta_workflow_output_submission),
    })
}

fn summarize_meta_workflow_output_submission(
    submission: &crate::session::WorkflowRunOutputSubmission,
) -> serde_json::Value {
    serde_json::json!({
        "valid": submission.valid(),
        "warning": submission.warning(),
        "submitted_at_ms": submission.submitted_at_ms(),
        "output": summarize_meta_workflow_output_payload(submission.output()),
    })
}

fn summarize_meta_workflow_completion(
    completion: &crate::session::WorkflowCompletionSnapshot,
) -> serde_json::Value {
    serde_json::json!({
        "summary": trim_meta_text(completion.summary(), 512),
        "output": completion.output().map(summarize_meta_workflow_output_payload),
    })
}

fn summarize_meta_workflow_message(message: &crate::session::WorkflowMessage) -> serde_json::Value {
    serde_json::json!({
        "id": message.id(),
        "source_node_run_id": message.source_node_run_id(),
        "target_node_id": message.target_node_id(),
        "message_type": message.message_type(),
        "summary": trim_meta_text(message.summary(), 512),
        "handoff_payload_present": !message.handoff_payload().is_empty(),
        "consumed_by_node_run_id": message.consumed_by_node_run_id(),
        "created_at_ms": message.created_at_ms(),
    })
}

fn summarize_meta_workflow_failure(
    failure: &crate::session::WorkflowFailureEvent,
) -> serde_json::Value {
    serde_json::json!({
        "kind": failure.kind(),
        "source_node_run_id": failure.source_node_run_id(),
        "edge_ids": failure.edge_ids(),
        "message": trim_meta_text(failure.message(), 1024),
        "timestamp_ms": failure.timestamp_ms(),
    })
}

fn summarize_meta_workflow_intermediate_output(
    output: &crate::session::WorkflowIntermediateOutput,
) -> serde_json::Value {
    serde_json::json!({
        "id": output.id(),
        "source_node_run_id": output.source_node_run_id(),
        "valid": output.valid(),
        "warning": output.warning(),
        "timestamp_ms": output.timestamp_ms(),
        "output": summarize_meta_workflow_output_payload(output.output()),
    })
}

fn summarize_meta_workflow_output_payload(
    output: &crate::session::WorkflowOutputPayload,
) -> serde_json::Value {
    serde_json::json!({
        "message": trim_meta_text(output.message(), 1024),
        "artifacts": output
            .artifacts()
            .iter()
            .map(|artifact| serde_json::json!({
                "id": artifact.id(),
                "kind": artifact.kind(),
                "path": artifact.path(),
                "display_name": artifact.display_name(),
            }))
            .collect::<Vec<_>>(),
    })
}

fn trim_meta_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let trimmed = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{trimmed}...")
    } else {
        trimmed
    }
}
