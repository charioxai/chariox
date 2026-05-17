import type {
  AgentInstance,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
import {
  handleWorkflowNodeInstructionsCommand,
  type WorkflowNodeInstructionsCommandContext,
  type WorkflowNodeInstructionsCommandDeps,
  type WorkflowNodeInstructionsPayload,
} from "./workflow-node-instructions-command-handler.js"

type FooterTone = "info" | "error"

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

export type WorkflowNodePayload = {
  node: WorkflowNodeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

export type WorkflowNodeCommandContext = WorkflowNodeInstructionsCommandContext

export type WorkflowNodeCommandDeps = WorkflowNodeInstructionsCommandDeps & {
  sessionState: () => RuntimeSession
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  addWorkflowNode: (workflowRef: string, agentId: string) => Promise<WorkflowNodePayload>
  removeWorkflowNode: (workflowRef: string, nodeId: string) => Promise<WorkflowNodePayload>
  setWorkflowNodeCanCompleteRun?: (
    workflowRef: string,
    nodeId: string,
    canCompleteWorkflowRun: boolean,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeCanEmitIntermediateOutput?: (
    workflowRef: string,
    nodeId: string,
    canEmitIntermediateWorkflowRunOutput: boolean,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeIntermediateOutputSchema?: (
    workflowRef: string,
    nodeId: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeMaxTurns?: (
    workflowRef: string,
    nodeId: string,
    maxTurns: number | null,
  ) => Promise<WorkflowNodePayload>
  updateWorkflowNodeInstructions?: (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => Promise<WorkflowNodeInstructionsPayload>
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowAddAllNodesCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const explicitWorkflowRef = args.length >= 4 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const target = explicitWorkflowRef ? args[3] : args[2]
  if (!workflowRef || target !== "all") {
    deps.flashFooter("usage: /workflow add node [workflow-ref] all", "error")
    return
  }
  await addAllRemainingWorkflowNodes(deps, workflowRef)
}

export async function handleWorkflowNodeCommand(
  deps: WorkflowNodeCommandDeps,
  context: WorkflowNodeCommandContext,
  args: readonly string[],
): Promise<void> {
  const action = args[1]
  if (action === "add") {
    await handleWorkflowNodeAddCommand(deps, context, args)
    return
  }
  if (action === "remove") {
    await handleWorkflowNodeRemoveCommand(deps, context, args)
    return
  }
  if (action === "instructions") {
    await handleWorkflowNodeInstructionsCommand(deps, context, args)
    return
  }
  if (action === "can-complete-run") {
    await handleWorkflowNodeBooleanSettingCommand(
      deps,
      context,
      args,
      "can-complete-run",
      deps.setWorkflowNodeCanCompleteRun,
      "can_complete_workflow_run",
    )
    return
  }
  if (action === "can-emit-intermediate-output") {
    await handleWorkflowNodeBooleanSettingCommand(
      deps,
      context,
      args,
      "can-emit-intermediate-output",
      deps.setWorkflowNodeCanEmitIntermediateOutput,
      "can_emit_intermediate_run_output",
    )
    return
  }
  if (action === "intermediate-output-schema") {
    await handleWorkflowNodeSchemaCommand(deps, context, args)
    return
  }
  if (action === "max-turns") {
    await handleWorkflowNodeMaxTurnsCommand(deps, context, args)
    return
  }
  deps.flashFooter(
    "usage: /workflow node add [workflow-ref] <agent-id|all> | remove [workflow-ref] <node-id> | instructions ... | can-complete-run [workflow-ref] <node-id> <true|false> | can-emit-intermediate-output [workflow-ref] <node-id> <true|false> | intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none> | max-turns [workflow-ref] <node-id> <count|none>",
    "error",
  )
}

async function handleWorkflowNodeAddCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const explicitWorkflowRef = args.length >= 4 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const agentRef = explicitWorkflowRef ? args[3] : args[2]
  if (!workflowRef || !agentRef) {
    deps.flashFooter("usage: /workflow node add [workflow-ref] <agent-id|all>", "error")
    return
  }
  if (agentRef === "all") {
    await addAllRemainingWorkflowNodes(deps, workflowRef)
    return
  }
  const resolvedAgent = deps.resolveSessionAgent(agentRef)
  if (!resolvedAgent.agent || resolvedAgent.error) {
    deps.flashFooter(resolvedAgent.error ?? `agent '${agentRef}' not found`, "error")
    return
  }
  const payload = await deps.addWorkflowNode(workflowRef, resolvedAgent.agent.id)
  deps.applySessionState(payload.session)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.flashFooter(`added workflow node ${payload.node.id} for agent ${deps.formatAgentLabel(resolvedAgent.agent)}`, "info")
}

async function handleWorkflowNodeRemoveCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const explicitWorkflowRef = args.length >= 4 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[3] : args[2]
  if (!workflowRef || !nodeId) {
    deps.flashFooter("usage: /workflow node remove [workflow-ref] <node-id>", "error")
    return
  }
  const payload = await deps.removeWorkflowNode(workflowRef, nodeId)
  deps.applySessionState(payload.session)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.flashFooter(`removed workflow node ${payload.node.id}`, "info")
}

async function handleWorkflowNodeBooleanSettingCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
  action: "can-complete-run" | "can-emit-intermediate-output",
  setter: WorkflowNodeCommandDeps["setWorkflowNodeCanCompleteRun"],
  resultKey: "can_complete_workflow_run" | "can_emit_intermediate_run_output",
): Promise<void> {
  const explicitWorkflowRef = args.length >= 5 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[3] : args[2]
  const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
  if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
    deps.flashFooter(`usage: /workflow node ${action} [workflow-ref] <node-id> <true|false>`, "error")
    return
  }
  if (!setter) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await setter(workflowRef, nodeId, value === "true")
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.flashFooter(
    `workflow node ${payload.node.id} ${action} set to ${payload.node[resultKey] ? "true" : "false"}`,
    "info",
  )
}

async function handleWorkflowNodeSchemaCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const explicitWorkflowRef = args.length >= 5 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[3] : args[2]
  const value = explicitWorkflowRef ? args[4] : args[3]
  if (!workflowRef || !nodeId || value === undefined) {
    deps.flashFooter("usage: /workflow node intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none>", "error")
    return
  }
  if (!deps.setWorkflowNodeIntermediateOutputSchema) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const schemaRef = value.trim().toLowerCase() === "none" ? null : value
  const payload = await deps.setWorkflowNodeIntermediateOutputSchema(workflowRef, nodeId, schemaRef)
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.flashFooter(
    `workflow node ${payload.node.id} intermediate-output-schema set to ${payload.node.intermediate_output_schema_ref ?? "none"}`,
    "info",
  )
}

async function handleWorkflowNodeMaxTurnsCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const explicitWorkflowRef = args.length >= 5 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[3] : args[2]
  const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
  if (!workflowRef || !nodeId || !value) {
    deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
    return
  }
  if (!deps.setWorkflowNodeMaxTurns) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const maxTurns = parseMaxTurns(value)
  if (maxTurns === undefined) {
    deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
    return
  }
  const payload = await deps.setWorkflowNodeMaxTurns(workflowRef, nodeId, maxTurns)
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.flashFooter(
    `workflow node ${payload.node.id} max-turns set to ${payload.node.max_turns ?? "none"}`,
    "info",
  )
}

function parseMaxTurns(value: string): number | null | undefined {
  if (value === "none") {
    return null
  }
  const parsed = Number.parseInt(value, 10)
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return undefined
  }
  return parsed
}

async function addAllRemainingWorkflowNodes(
  deps: WorkflowNodeCommandDeps,
  workflowRef: string,
): Promise<void> {
  const resolved = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(resolved.workflow)

  const existingAgentIds = new Set((resolved.workflow.nodes ?? []).map((node) => node.agent_id))
  const agentsToAdd = deps.sessionState().agents.filter((agent) => !existingAgentIds.has(agent.id))
  if (agentsToAdd.length === 0) {
    deps.selectWorkflowCanvas(resolved.workflow.id)
    deps.flashFooter(`workflow ${resolved.workflow.id} already has nodes for all session agents`, "info")
    return
  }

  let latestWorkflow = resolved.workflow
  for (const agent of agentsToAdd) {
    const payload = await deps.addWorkflowNode(latestWorkflow.id, agent.id)
    latestWorkflow = payload.workflow
    deps.applySessionState(payload.session)
    deps.upsertWorkflowDefinition(payload.workflow)
  }

  deps.selectWorkflowCanvas(latestWorkflow.id)
  deps.flashFooter(
    `added ${agentsToAdd.length} workflow node${agentsToAdd.length === 1 ? "" : "s"} for ${agentsToAdd.map((agent) => deps.formatAgentLabel(agent)).join(", ")}`,
    "info",
  )
}
