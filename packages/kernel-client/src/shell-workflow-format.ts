import type {
  QueuedWorkflowLaunch,
  WorkflowDefinition,
  WorkflowPublicationDefinition,
  WorkflowPublicationTrustedSender,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./kernel-types.js"

export function formatWorkflowLabel(workflow: WorkflowDefinition): string {
  return workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
}

export function formatWorkflowList(workflows: WorkflowDefinition[], currentWorkflowId?: string): string {
  if (workflows.length === 0) {
    return "no workflows in session"
  }
  return workflows.map((workflow) => {
    const current = workflow.id === currentWorkflowId ? " current" : ""
    return `${formatWorkflowLabel(workflow)} nodes=${workflow.nodes?.length ?? 0} edges=${workflow.edges?.length ?? 0} endpoints=${workflow.endpoints?.length ?? 0}${current}`
  }).join("\n")
}

export function formatWorkflowDetails(workflow: WorkflowDefinition): string {
  return [
    `workflow ${formatWorkflowLabel(workflow)}`,
    `nodes=${workflow.nodes?.length ?? 0} edges=${workflow.edges?.length ?? 0} endpoints=${workflow.endpoints?.length ?? 0}`,
    `flush_context=${String(workflow.flush_agent_context_before_run ?? true)}`,
    workflow.run_output_schema_ref ? `run_output_schema=${workflow.run_output_schema_ref}` : null,
    workflow.intermediate_output_schema_ref ? `intermediate_output_schema=${workflow.intermediate_output_schema_ref}` : null,
  ].filter(Boolean).join("\n")
}

export function formatWorkflowRunList(workflowRuns: WorkflowRun[], workflowRef: string | null): string {
  if (workflowRuns.length === 0) {
    return workflowRef ? `no workflow runs for ${workflowRef}` : "no workflow runs in session"
  }
  return workflowRuns.map((run) => {
    const failures = (run.failure_events?.length ?? 0) > 0 ? ` failures=${run.failure_events?.length ?? 0}` : ""
    return `${run.id} workflow=${run.workflow_id} endpoint=${run.endpoint_id} [${String(run.status).toLowerCase()}${failures}]`
  }).join("\n")
}

export function formatWorkflowPublicationLabel(publication: WorkflowPublicationDefinition): string {
  return publication.alias ? `${publication.id} (${publication.alias})` : publication.id
}

export function formatWorkflowPublications(publications: WorkflowPublicationDefinition[]): string {
  if (publications.length === 0) {
    return "no workflow publications configured"
  }
  return publications.map((publication) => {
    const route = publication.route ? ` route=${publication.route}` : ""
    const methods = publication.methods?.length ? ` methods=${publication.methods.join(",")}` : ""
    return `${formatWorkflowPublicationLabel(publication)} workflow=${publication.workflow_id} endpoint=${publication.endpoint_id} enabled=${String(publication.enabled)}${route}${methods}`
  }).join("\n")
}

export function formatWorkflowPublicationSenders(senders: WorkflowPublicationTrustedSender[]): string {
  if (senders.length === 0) {
    return "no trusted senders configured"
  }
  return senders.map((sender) => {
    const name = sender.display_name ? ` (${sender.display_name})` : ""
    const transports = sender.allowed_transports?.length ? ` transports=${sender.allowed_transports.join(",")}` : ""
    const revoked = sender.revoked_at_ms ? " revoked=true" : ""
    return `${sender.sender_id}${name} publication=${sender.publication_id}${transports}${revoked}`
  }).join("\n")
}

export function formatWorkflowWatchdogs(watchdogs: WorkflowWatchdogDefinition[]): string {
  if (watchdogs.length === 0) {
    return "no workflow watchdogs configured"
  }
  return watchdogs.map((watchdog) => (
    `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"}`
  )).join("\n")
}

export function formatQueuedWorkflowLaunches(queuedLaunches: QueuedWorkflowLaunch[]): string {
  if (queuedLaunches.length === 0) {
    return "workflow queue is empty"
  }
  return queuedLaunches.map((queued) => (
    `${queued.id} workflow=${queued.workflow_id} endpoint=${queued.endpoint_id} source=${queued.source}`
  )).join("\n")
}
