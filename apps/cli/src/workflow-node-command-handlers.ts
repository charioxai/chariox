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
  grantAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  grantAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  grantAgentScript?: (agentRef: string, name: string, environment: string) => Promise<AgentInstance>
  revokeAgentScript?: (agentRef: string, name: string) => Promise<AgentInstance>
  grantAgentConnector?: (agentRef: string, name: string, credential?: string | null, maxSafety?: string | null) => Promise<AgentInstance>
  revokeAgentConnector?: (agentRef: string, name: string) => Promise<AgentInstance>
  updateWorkflowNodeInstructions?: (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => Promise<WorkflowNodeInstructionsPayload>
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  appendNotice?: (message: string) => void
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
  if (action === "extensions") {
    await handleWorkflowNodeExtensionsCommand(deps, context, args)
    return
  }
  if (action === "extension") {
    await handleWorkflowNodeExtensionCommand(deps, context, args)
    return
  }
  deps.flashFooter(
    "usage: /workflow node add [workflow-ref] <agent-id|all> | remove [workflow-ref] <node-id> | instructions ... | can-complete-run [workflow-ref] <node-id> <true|false> | can-emit-intermediate-output [workflow-ref] <node-id> <true|false> | intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none> | max-turns [workflow-ref] <node-id> <count|none> | extensions [workflow-ref] <node-id> | extension grant|revoke [workflow-ref] <node-id> <mcp|skill|script|connector> <name>",
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

async function handleWorkflowNodeExtensionsCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const explicitWorkflowRef = args.length >= 4 ? args[2] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[3] : args[2]
  if (!workflowRef || !nodeId) {
    deps.flashFooter("usage: /workflow node extensions [workflow-ref] <node-id>", "error")
    return
  }
  const { node, agent } = await resolveWorkflowNodeAgent(deps, workflowRef, nodeId)
  if (!node || !agent) return
  const grants = agent.extension_grants ?? []
  deps.appendNotice?.(grants.length
    ? grants.map((grant) => `${grant.kind}:${grant.name}${grant.environment ? `@${grant.environment}` : ""}${grant.max_safety ? ` allow=${grant.max_safety}` : ""}`).join("\n")
    : `node ${node.id} agent ${agent.agent_ref} has no extensions`)
  deps.flashFooter(`showing ${grants.length} extension${grants.length === 1 ? "" : "s"} for node ${node.id}`, "info")
}

async function handleWorkflowNodeExtensionCommand(
  deps: WorkflowNodeCommandDeps,
  context: Pick<WorkflowNodeCommandContext, "workflowRefOrSelected">,
  args: readonly string[],
): Promise<void> {
  const action = args[2]
  const hasExplicitWorkflow = args.length >= 7
  const workflowRef = context.workflowRefOrSelected(hasExplicitWorkflow ? args[3] : null)
  const nodeId = hasExplicitWorkflow ? args[4] : args[3]
  const kind = (hasExplicitWorkflow ? args[5] : args[4]) as "mcp" | "skill" | "script" | "connector" | undefined
  const name = hasExplicitWorkflow ? args[6] : args[5]
  if ((action !== "grant" && action !== "revoke") || !workflowRef || !nodeId || !isExtensionKind(kind) || !name) {
    deps.flashFooter("usage: /workflow node extension grant|revoke [workflow-ref] <node-id> <mcp|skill|script|connector> <name> [--environment <name>] [--credential <id>] [--allow read|write|destructive]", "error")
    return
  }
  const { node, agent } = await resolveWorkflowNodeAgent(deps, workflowRef, nodeId)
  if (!node || !agent) return
  const updated = action === "grant"
    ? await grantNodeExtension(deps, agent.agent_ref, kind, name, args)
    : await revokeNodeExtension(deps, agent.agent_ref, kind, name)
  applyUpdatedAgent(deps, updated)
  deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} ${kind} ${name} ${action === "grant" ? "to" : "from"} workflow node ${node.id}`, "info")
}

async function resolveWorkflowNodeAgent(
  deps: WorkflowNodeCommandDeps,
  workflowRef: string,
  nodeId: string,
): Promise<{ node: WorkflowNodeDefinition | null; agent: AgentInstance | null }> {
  const resolved = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(resolved.workflow)
  const node = (resolved.workflow.nodes ?? []).find((candidate) => candidate.id === nodeId) ?? null
  if (!node) {
    deps.flashFooter(`workflow node '${nodeId}' not found`, "error")
    return { node: null, agent: null }
  }
  const agent = deps.sessionState().agents.find((candidate) => candidate.id === node.agent_id) ?? null
  if (!agent) {
    deps.flashFooter("Extensions are managed by the collaborator who owns this node.", "error")
    return { node: null, agent: null }
  }
  return { node, agent }
}

async function grantNodeExtension(
  deps: WorkflowNodeCommandDeps,
  agentRef: string,
  kind: "mcp" | "skill" | "script" | "connector",
  name: string,
  args: readonly string[],
): Promise<AgentInstance> {
  if (kind === "mcp" && deps.grantAgentMcp) return deps.grantAgentMcp(agentRef, name)
  if (kind === "skill" && deps.grantAgentSkill) return deps.grantAgentSkill(agentRef, name)
  if (kind === "script" && deps.grantAgentScript) {
    const environment = readOption(args, "--environment")
    if (!environment) throw new Error("script grants require --environment <name>")
    return deps.grantAgentScript(agentRef, name, environment)
  }
  if (kind === "connector" && deps.grantAgentConnector) {
    return deps.grantAgentConnector(agentRef, name, readOption(args, "--credential"), readOption(args, "--allow"))
  }
  throw new Error(`${kind} extension grant command unavailable`)
}

async function revokeNodeExtension(
  deps: WorkflowNodeCommandDeps,
  agentRef: string,
  kind: "mcp" | "skill" | "script" | "connector",
  name: string,
): Promise<AgentInstance> {
  if (kind === "mcp" && deps.revokeAgentMcp) return deps.revokeAgentMcp(agentRef, name)
  if (kind === "skill" && deps.revokeAgentSkill) return deps.revokeAgentSkill(agentRef, name)
  if (kind === "script" && deps.revokeAgentScript) return deps.revokeAgentScript(agentRef, name)
  if (kind === "connector" && deps.revokeAgentConnector) return deps.revokeAgentConnector(agentRef, name)
  throw new Error(`${kind} extension revoke command unavailable`)
}

function applyUpdatedAgent(deps: WorkflowNodeCommandDeps, updated: AgentInstance): void {
  const session = deps.sessionState()
  deps.applySessionState({
    ...session,
    agents: session.agents.map((agent) => agent.id === updated.id ? updated : agent),
  })
}

function readOption(args: readonly string[], name: string): string | null {
  const index = args.indexOf(name)
  return index >= 0 ? args[index + 1] ?? null : null
}

function isExtensionKind(value: unknown): value is "mcp" | "skill" | "script" | "connector" {
  return value === "mcp" || value === "skill" || value === "script" || value === "connector"
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
