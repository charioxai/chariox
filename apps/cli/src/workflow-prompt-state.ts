import type { WorkflowDefinition, WorkflowEndpointDefinition, WorkflowRun } from "./cli-types.js"
import { resolveSelectedWorkflow, resolveSelectedWorkflowNodeId } from "./workflow-graph/selection.js"

export type WorkflowPromptState = {
  workflow: WorkflowDefinition | null
  workflowRun: WorkflowRun | null
  selectedNodeId: string | null
  endpoint: WorkflowEndpointDefinition | null
  enabled: boolean
  disabledReason: string | null
}

export function resolveActiveWorkflowRun(
  workflowId: string,
  workflowRuns: readonly WorkflowRun[],
) {
  const matchingRuns = workflowRuns.filter((workflowRun) => workflowRun.workflow_id === workflowId)
  if (matchingRuns.length === 0) {
    return null
  }
  const nonTerminalRuns = matchingRuns.filter((workflowRun) => !isTerminalWorkflowRunStatus(workflowRun.status))
  const candidates = nonTerminalRuns.length > 0 ? nonTerminalRuns : []
  return [...candidates].sort((left, right) => right.created_at_ms - left.created_at_ms)[0] ?? null
}

export function deriveWorkflowPromptState(options: {
  workflowScreenActive: boolean
  workflows: WorkflowDefinition[]
  workflowRuns: WorkflowRun[]
  selectedWorkflowId: string | null
  selectedWorkflowNodeId: string | null
}): WorkflowPromptState {
  const workflow = resolveSelectedWorkflow(options.workflows, options.selectedWorkflowId)
  const selectedNodeId = resolveSelectedWorkflowNodeId(workflow, options.selectedWorkflowNodeId)

  if (!options.workflowScreenActive) {
    return {
      workflow,
      workflowRun: workflow ? resolveActiveWorkflowRun(workflow.id, options.workflowRuns) : null,
      selectedNodeId,
      endpoint: null,
      enabled: false,
      disabledReason: null,
    }
  }

  if (!workflow) {
    return {
      workflow: null,
      workflowRun: null,
      selectedNodeId: null,
      endpoint: null,
      enabled: false,
      disabledReason: "no workflow selected",
    }
  }

  const endpoints = workflow.endpoints ?? []
  if (endpoints.length === 0) {
    return {
      workflow,
      workflowRun: null,
      selectedNodeId,
      endpoint: null,
      enabled: false,
      disabledReason: "no workflow endpoints configured",
    }
  }

  const workflowRun = resolveActiveWorkflowRun(workflow.id, options.workflowRuns)
  if (!workflowRun) {
    return {
      workflow,
      workflowRun: null,
      selectedNodeId,
      endpoint: null,
      enabled: false,
      disabledReason: "no active workflow run",
    }
  }

  const endpoint = selectedNodeId
    ? endpoints.find((candidate) => candidate.entry_node_id === selectedNodeId) ?? null
    : null
  if (!endpoint) {
    return {
      workflow,
      workflowRun,
      selectedNodeId,
      endpoint: null,
      enabled: false,
      disabledReason: "selected node has no endpoint",
    }
  }

  return {
    workflow,
    workflowRun,
    selectedNodeId,
    endpoint,
    enabled: true,
    disabledReason: null,
  }
}

export function isWorkflowCommandInput(value: string) {
  return value.startsWith("/")
}

export function formatWorkflowPromptPlaceholder(options: {
  workflowScreenActive: boolean
  state: WorkflowPromptState
  attachedPlaceholder: string
  detachedPlaceholder: string
}) {
  if (!options.workflowScreenActive) {
    return options.attachedPlaceholder
  }
  if (options.state.enabled && options.state.endpoint) {
    return `Send prompt to endpoint ${formatWorkflowEndpointLabel(options.state.endpoint)}`
  }
  return options.state.disabledReason
    ? `Workflow prompt disabled: ${options.state.disabledReason}. Use /workflow ...`
    : "Workflow prompt disabled. Use /workflow ..."
}

export function formatWorkflowEndpointLabel(endpoint: WorkflowEndpointDefinition) {
  return endpoint.alias ? `${endpoint.id} (${endpoint.alias})` : endpoint.id
}

function isTerminalWorkflowRunStatus(status: string) {
  return status === "Completed" || status === "Failed" || status === "Stopped"
}
