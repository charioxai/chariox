import type {
  WorkflowDefinition,
  WorkflowPromptQueueDefinition,
  WorkflowPublicationDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
  WorkflowScheduleDefinition,
  WorkflowScheduleTrigger,
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
  const edgeLines = (workflow.edges ?? []).map((edge) => {
    const handoffSchema = edge.handoff_schema_ref ? ` handoff_schema=${edge.handoff_schema_ref}` : ""
    const validationPolicy = edge.validation_policy ? ` validation_policy=${edge.validation_policy}` : ""
    return `edge ${edge.id} ${edge.from_node_id}->${edge.to_node_id}${handoffSchema}${validationPolicy}`
  })
  return [
    `workflow ${formatWorkflowLabel(workflow)}`,
    `nodes=${workflow.nodes?.length ?? 0} edges=${workflow.edges?.length ?? 0} endpoints=${workflow.endpoints?.length ?? 0}`,
    `flush_context=${String(workflow.flush_agent_context_before_run ?? true)}`,
    workflow.run_output_schema_ref ? `run_output_schema=${workflow.run_output_schema_ref}` : null,
    ...edgeLines,
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
    return "no workflow triggers configured"
  }
  return publications.map((publication) => {
    const queue = publication.queue_ref ? ` queue=${publication.queue_ref}` : ""
    const route = publication.route ? ` route=${publication.route}` : ""
    const methods = publication.methods?.length ? ` methods=${publication.methods.join(",")}` : ""
    return `${formatWorkflowPublicationLabel(publication)} workflow=${publication.workflow_id} endpoint=${publication.endpoint_id}${queue} enabled=${String(publication.enabled)}${route}${methods}`
  }).join("\n")
}

export function formatWorkflowWatchdogs(watchdogs: WorkflowWatchdogDefinition[]): string {
  return formatWorkflowSchedules(watchdogs)
}

export function formatWorkflowSchedules(schedules: WorkflowScheduleDefinition[]): string {
  if (schedules.length === 0) {
    return "no workflow schedules configured"
  }
  return schedules.map((schedule) => (
    `${schedule.id} workflow=${schedule.workflow_id} endpoint=${schedule.endpoint_id} trigger=${formatWorkflowScheduleTrigger(schedule.trigger)} overlap=${schedule.overlap_policy} enabled=${String(schedule.enabled)} runs=${schedule.runs_started}/${schedule.max_runs ?? "unbounded"}`
  )).join("\n")
}

function formatWorkflowScheduleTrigger(trigger: WorkflowScheduleTrigger): string {
  if (trigger.kind === "interval") {
    return `every:${trigger.every_seconds}s`
  }
  return `cron:${trigger.expression} tz=${trigger.timezone}`
}

export function formatLegacyWorkflowWatchdogs(watchdogs: WorkflowWatchdogDefinition[]): string {
  if (watchdogs.length === 0) {
    return "no workflow watchdogs configured"
  }
  return watchdogs.map((watchdog) => (
    `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"}`
  )).join("\n")
}

export function formatWorkflowPromptQueues(
  queues: WorkflowPromptQueueDefinition[],
  queuedPrompts: WorkflowQueuedPrompt[] = [],
): string {
  if (queues.length === 0) {
    return "workflow queues unavailable"
  }
  return queues.map((queue) => {
    const depth = queuedPrompts.filter((prompt) => prompt.workflow_id === queue.workflow_id && prompt.queue_id === queue.id).length
    return `${queue.id} (${queue.alias}) workflow=${queue.workflow_id} priority=${queue.priority} enabled=${String(queue.enabled)} depth=${depth}`
  }).join("\n")
}

export function formatWorkflowQueuedPrompts(queuedPrompts: WorkflowQueuedPrompt[]): string {
  if (queuedPrompts.length === 0) {
    return "workflow queue is empty"
  }
  return queuedPrompts.map((queued) => (
    `${queued.id} workflow=${queued.workflow_id} queue=${queued.queue_id} endpoint=${queued.endpoint_id} source=${queued.source} status=${queued.status}`
  )).join("\n")
}
