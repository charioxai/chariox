import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
} from "./cli-types.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"

type FooterTone = "info" | "error"

export type WorkflowEdgePayload = {
  edge: WorkflowEdgeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

export type WorkflowEdgeCommandContext = {
  isKnownWorkflowReference: (reference: string | undefined) => boolean
  selectedWorkflowRef: () => string | null
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowEdgeCommandDeps = {
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  addWorkflowEdge: (
    workflowRef: string,
    fromNodeId: string,
    toNodeId: string,
  ) => Promise<WorkflowEdgePayload>
  removeWorkflowEdge: (workflowRef: string, edgeId: string) => Promise<WorkflowEdgePayload>
  applySessionState: (session: RuntimeSession) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  showWorkflowScreen: () => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export function hasWorkflowEdgeShorthandArgs(args: readonly string[]): boolean {
  return Boolean(args[1] && args[2])
}

export async function handleWorkflowEdgeCommand(
  deps: WorkflowEdgeCommandDeps,
  context: WorkflowEdgeCommandContext,
  args: readonly string[],
): Promise<void> {
  const action = args[1]
  if (action === "add") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = explicitWorkflowRef ?? context.selectedWorkflowRef()
    const fromRef = explicitWorkflowRef ? args[3] : args[2]
    const toRef = explicitWorkflowRef ? args[4] : args[3]
    if (!workflowRef || !fromRef || !toRef) {
      deps.flashFooter(workflowEdgeAddUsage, "error")
      return
    }
    if (!explicitWorkflowRef && context.isKnownWorkflowReference(fromRef)) {
      deps.flashFooter(workflowEdgeAddUsage, "error")
      return
    }
    await createWorkflowEdge(deps, workflowRef, fromRef, toRef, false)
    return
  }
  if (action === "remove") {
    const explicitWorkflowRef = args.length >= 4 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const edgeId = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !edgeId) {
      deps.flashFooter("usage: /workflow edge remove [workflow-ref] <edge-id>", "error")
      return
    }
    const payload = await deps.removeWorkflowEdge(workflowRef, edgeId)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(`removed workflow edge ${payload.edge.id}`, "info")
    return
  }
  deps.flashFooter(
    `${workflowEdgeAddUsage} | remove [workflow-ref] <edge-id>`,
    "error",
  )
}

export async function handleWorkflowEdgeShorthandCommand(
  deps: WorkflowEdgeCommandDeps,
  args: readonly string[],
): Promise<void> {
  const workflowRef = args[0]!
  const fromRef = args[1]!
  const toRef = args[2]!
  await createWorkflowEdge(deps, workflowRef, fromRef, toRef, true)
}

async function createWorkflowEdge(
  deps: WorkflowEdgeCommandDeps,
  workflowRef: string,
  fromRef: string,
  toRef: string,
  showWorkflowAfterCreate: boolean,
): Promise<void> {
  const resolvedWorkflow = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(resolvedWorkflow.workflow)
  const fromNode = resolveWorkflowNodeReference(deps, resolvedWorkflow.workflow, workflowRef, fromRef)
  if ("error" in fromNode) {
    deps.flashFooter(fromNode.error, "error")
    return
  }
  const toNode = resolveWorkflowNodeReference(deps, resolvedWorkflow.workflow, workflowRef, toRef)
  if ("error" in toNode) {
    deps.flashFooter(toNode.error, "error")
    return
  }
  if (fromNode.nodeId === toNode.nodeId) {
    deps.flashFooter("workflow edges must connect two different nodes", "error")
    return
  }
  if (hasDuplicateWorkflowEdge(resolvedWorkflow.workflow, fromNode.nodeId, toNode.nodeId)) {
    deps.flashFooter("workflow edge already exists between those nodes", "error")
    return
  }
  const payload = await deps.addWorkflowEdge(workflowRef, fromNode.nodeId, toNode.nodeId)
  deps.applySessionState(payload.session)
  deps.selectWorkflowCanvas(payload.workflow.id)
  if (showWorkflowAfterCreate) {
    deps.showWorkflowScreen()
  }
  deps.flashFooter(`added workflow edge ${payload.edge.id}`, "info")
}

function hasDuplicateWorkflowEdge(
  workflow: WorkflowDefinition,
  fromNodeId: string,
  toNodeId: string,
) {
  return (workflow.edges ?? []).some((edge) => (
    edge.from_node_id === fromNodeId && edge.to_node_id === toNodeId
  ))
}

const workflowEdgeAddUsage = "usage: /workflow edge add [workflow-ref] <from-node-id|from-agent-ref> <to-node-id|to-agent-ref>"

function resolveWorkflowNodeReference(
  deps: WorkflowEdgeCommandDeps,
  workflow: WorkflowDefinition,
  workflowRef: string,
  reference: string,
): { nodeId: string } | { error: string } {
  const nodes = workflow.nodes ?? []
  const nodeMatch = nodes.find((node) => node.id === reference)
  if (nodeMatch) {
    return { nodeId: nodeMatch.id }
  }

  const resolvedAgent = deps.resolveSessionAgent(reference)
  if (!resolvedAgent.agent) {
    if (resolvedAgent.error?.startsWith("multiple agents match")) {
      return { error: resolvedAgent.error }
    }
    return { nodeId: reference }
  }

  const matches = nodes.filter((node) => node.agent_id === resolvedAgent.agent?.id)
  if (matches.length === 1) {
    const [node] = matches
    return { nodeId: node?.id ?? reference }
  }
  if (matches.length > 1) {
    return {
      error: `agent '${reference}' maps to multiple nodes in workflow '${workflow.id}'; use explicit node ids`,
    }
  }
  return {
    error: `agent '${reference}' is not a node in workflow '${workflowRef}'; add it first with /workflow node add <workflow-ref> <agent-id>`,
  }
}
