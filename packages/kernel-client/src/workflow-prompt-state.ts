import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "./kernel-types.js"

export type WorkflowPromptAgentLike = {
  readonly id: string
  readonly agent_ref?: string | null
  readonly alias?: string | null
}

export type WorkflowPromptWorkflowNodeLike = {
  readonly id: string
  readonly agent_id: string
}

export type WorkflowPromptWorkflowLike = {
  readonly id: string
  readonly nodes?: readonly WorkflowPromptWorkflowNodeLike[] | null
}

export type WorkflowPromptRunLike = {
  readonly id: string
  readonly workflow_id: string
  readonly status: string
  readonly created_at_ms: number
}

export type WorkflowPromptState<
  TWorkflow extends WorkflowPromptWorkflowLike = WorkflowDefinition,
  TRun extends WorkflowPromptRunLike = WorkflowRun,
  TAgent extends WorkflowPromptAgentLike = AgentInstance,
> = {
  workflow: TWorkflow | null
  workflowRun: TRun | null
  selectedNodeId: string | null
  selectedAgent: TAgent | null
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

export function resolveActiveWorkflowRun<TRun extends WorkflowPromptRunLike>(
  workflowId: string,
  workflowRuns: readonly TRun[],
): TRun | null {
  const matchingRuns = workflowRuns.filter((workflowRun) => workflowRun.workflow_id === workflowId)
  if (matchingRuns.length === 0) {
    return null
  }
  const nonTerminalRuns = matchingRuns.filter((workflowRun) => !isTerminalWorkflowRunStatus(workflowRun.status))
  const candidates = nonTerminalRuns.length > 0 ? nonTerminalRuns : []
  return [...candidates].sort((left, right) => right.created_at_ms - left.created_at_ms)[0] ?? null
}

export function deriveWorkflowPromptState<
  TWorkflow extends WorkflowPromptWorkflowLike,
  TRun extends WorkflowPromptRunLike,
  TAgent extends WorkflowPromptAgentLike,
>(options: {
  workflowScreenActive: boolean
  workflows: readonly TWorkflow[]
  workflowRuns: readonly TRun[]
  agents: readonly TAgent[]
  selectedWorkflowId: string | null
  selectedWorkflowNodeId: string | null
}): WorkflowPromptState<TWorkflow, TRun, TAgent> {
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

export function isWorkflowCommandInput(value: string): boolean {
  return value.startsWith("/")
}

export function validateWorkflowPromptSubmit(options: {
  state: WorkflowPromptState<WorkflowPromptWorkflowLike, WorkflowPromptRunLike, WorkflowPromptAgentLike>
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
  state: WorkflowPromptState<WorkflowPromptWorkflowLike, WorkflowPromptRunLike, WorkflowPromptAgentLike>
  attachedPlaceholder: string
  detachedPlaceholder: string
}): string {
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

export function formatWorkflowAgentLabel(agent: WorkflowPromptAgentLike): string {
  const alias = agent.alias?.trim()
  const ref = agent.agent_ref?.trim()
  return alias ? `${ref || agent.id} (${alias})` : ref || agent.id
}

export function resolveSelectedWorkflow<TWorkflow extends WorkflowPromptWorkflowLike>(
  workflows: readonly TWorkflow[],
  selectedWorkflowId: string | null,
): TWorkflow | null {
  if (workflows.length === 0) {
    return null
  }
  if (selectedWorkflowId) {
    const exact = workflows.find((workflow) => workflow.id === selectedWorkflowId)
    if (exact) {
      return exact
    }
  }
  return workflows[0] ?? null
}

export function resolveSelectedWorkflowNodeId(
  workflow: WorkflowPromptWorkflowLike | null,
  selectedNodeId: string | null,
): string | null {
  const nodes = workflow?.nodes ?? []
  if (nodes.length === 0) {
    return null
  }
  if (selectedNodeId && nodes.some((node) => node.id === selectedNodeId)) {
    return selectedNodeId
  }
  return nodes[0]?.id ?? null
}

function isTerminalWorkflowRunStatus(status: string): boolean {
  return status === "Completed" || status === "Failed" || status === "Stopped"
}
