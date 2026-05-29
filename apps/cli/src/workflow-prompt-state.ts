import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { resolveSelectedWorkflow, resolveSelectedWorkflowNodeId } from "./workflow-graph/selection.js"

export type WorkflowPromptState = {
  workflow: WorkflowDefinition | null
  workflowRun: WorkflowRun | null
  selectedNodeId: string | null
  selectedAgent: AgentInstance | null
  enabled: boolean
  disabledReason: string | null
}

export type WorkflowPromptSubmitDecision = {
  ok: true
  targetAgentId: string
} | {
  ok: false
  message: string
  tone: "error" | "info"
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
  agents: AgentInstance[]
  selectedWorkflowId: string | null
  selectedWorkflowNodeId: string | null
}): WorkflowPromptState {
  const workflow = resolveSelectedWorkflow(options.workflows, options.selectedWorkflowId)
  const selectedNodeId = resolveSelectedWorkflowNodeId(workflow, options.selectedWorkflowNodeId)
  const selectedNode = selectedNodeId
    ? workflow?.nodes?.find((node) => node.id === selectedNodeId) ?? null
    : null
  const selectedAgent = selectedNode
    ? options.agents.find((agent) => agent.id === selectedNode.agent_id) ?? null
    : null

  if (!options.workflowScreenActive) {
    return {
      workflow,
      workflowRun: workflow ? resolveActiveWorkflowRun(workflow.id, options.workflowRuns) : null,
      selectedNodeId,
      selectedAgent,
      enabled: false,
      disabledReason: null,
    }
  }

  if (!workflow) {
    return {
      workflow: null,
      workflowRun: null,
      selectedNodeId: null,
      selectedAgent: null,
      enabled: false,
      disabledReason: "no workflow selected",
    }
  }

  if (!selectedNodeId || !selectedNode) {
    return {
      workflow,
      workflowRun: resolveActiveWorkflowRun(workflow.id, options.workflowRuns),
      selectedNodeId,
      selectedAgent: null,
      enabled: false,
      disabledReason: "no workflow node selected",
    }
  }

  if (!selectedAgent) {
    return {
      workflow,
      workflowRun: resolveActiveWorkflowRun(workflow.id, options.workflowRuns),
      selectedNodeId,
      selectedAgent: null,
      enabled: false,
      disabledReason: "selected node agent unavailable",
    }
  }

  return {
    workflow,
    workflowRun: resolveActiveWorkflowRun(workflow.id, options.workflowRuns),
    selectedNodeId,
    selectedAgent,
    enabled: true,
    disabledReason: null,
  }
}

export function isWorkflowCommandInput(value: string) {
  return value.startsWith("/")
}

export function validateWorkflowPromptSubmit(options: {
  state: WorkflowPromptState
  pendingAttachmentCount: number
}): WorkflowPromptSubmitDecision {
  if (!options.state.enabled) {
    return {
      ok: false,
      message: `prompt disabled: ${options.state.disabledReason ?? "workflow prompt unavailable"}`,
      tone: "info",
    }
  }
  if (!options.state.workflow || !options.state.selectedAgent) {
    return {
      ok: false,
      message: "workflow prompt target unavailable",
      tone: "error",
    }
  }
  return {
    ok: true,
    targetAgentId: options.state.selectedAgent.id,
  }
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
  if (options.state.enabled && options.state.selectedAgent) {
    return `Prompt workflow agent ${formatWorkflowAgentLabel(options.state.selectedAgent)}`
  }
  return options.state.disabledReason
    ? `Workflow prompt disabled: ${options.state.disabledReason}. Use /workflow ...`
    : "Workflow prompt disabled. Use /workflow ..."
}

export function formatWorkflowAgentLabel(agent: AgentInstance) {
  const alias = agent.alias?.trim()
  const ref = agent.agent_ref?.trim()
  return alias ? `${ref || agent.id} (${alias})` : ref || agent.id
}

function isTerminalWorkflowRunStatus(status: string) {
  return status === "Completed" || status === "Failed" || status === "Stopped"
}
