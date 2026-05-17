import type {
  AgentInstance,
  QueuedWorkflowLaunch,
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
import {
  handleWorkflowEdgeCommand,
  handleWorkflowEdgeShorthandCommand,
  hasWorkflowEdgeShorthandArgs,
  type WorkflowEdgePayload,
} from "./workflow-edge-command-handlers.js"
import {
  handleWorkflowEndpointCommand,
  type WorkflowEndpointPayload,
} from "./workflow-endpoint-command-handlers.js"
import {
  handleWorkflowInvokeCommand,
  type WorkflowRunInvokePayload,
} from "./workflow-invoke-command-handler.js"
import {
  handleWorkflowNodeInstructionsCommand,
  type WorkflowNodeInstructionsPayload,
} from "./workflow-node-instructions-command-handler.js"
import {
  handleWorkflowQueueCommand,
  type QueuedWorkflowLaunchPayload,
} from "./workflow-queue-command-handlers.js"
import {
  handleWorkflowRunCancelCommand,
  handleWorkflowRunResumeCommand,
  handleWorkflowRunsCommand,
  type WorkflowRunCancelPayload,
  type WorkflowRunResumePayload,
} from "./workflow-run-command-handlers.js"
import {
  handleWorkflowSettingsCommand,
  isWorkflowSettingsCommand,
} from "./workflow-settings-command-handlers.js"
import { handleWorkflowTerminalCommand } from "./workflow-terminal-command-handler.js"
import {
  handleWorkflowWatchdogCommand,
  type WorkflowWatchdogPayload,
} from "./workflow-watchdog-command-handlers.js"

type FooterTone = "info" | "error"

type WorkflowCreatePayload = {
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

type WorkflowNodePayload = {
  node: WorkflowNodeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

export type WorkflowCommandHandlerDeps = {
  currentWorkspaceTarget: () => string
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  attachmentState: () => RuntimeAttachment | null
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  updateSessionConfig: (
    sessionId: string,
    attachmentId: string,
    values: Record<string, string>,
    requiresIdle: boolean,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  workflowScreenActive: () => boolean
  showWorkflowScreen: () => void
  selectedWorkflowId?: () => string | null
  selectWorkflowCanvas: (workflowId: string | null) => void
  replaceWorkflowDefinitions: (workflows: WorkflowDefinition[]) => void
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  createWorkflow: (alias?: string | null) => Promise<WorkflowCreatePayload>
  listWorkflows: () => Promise<WorkflowDefinition[]>
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  assignWorkflowAlias: (workflowId: string, alias: string) => Promise<WorkflowDefinition | null>
  createWorkflowEndpoint: (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => Promise<WorkflowEndpointPayload>
  assignWorkflowEndpointAlias: (
    workflowRef: string,
    endpointRef: string,
    alias: string,
  ) => Promise<WorkflowEndpointPayload>
  bindWorkflowEndpoint: (
    workflowRef: string,
    endpointRef: string,
    entryNodeId: string,
  ) => Promise<WorkflowEndpointPayload>
  addWorkflowNode: (workflowRef: string, agentId: string) => Promise<WorkflowNodePayload>
  removeWorkflowNode: (workflowRef: string, nodeId: string) => Promise<WorkflowNodePayload>
  addWorkflowEdge: (
    workflowRef: string,
    fromNodeId: string,
    toNodeId: string,
  ) => Promise<WorkflowEdgePayload>
  removeWorkflowEdge: (workflowRef: string, edgeId: string) => Promise<WorkflowEdgePayload>
  invokeWorkflowEndpoint?: (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
  ) => Promise<WorkflowRunInvokePayload>
  createWorkflowWatchdog?: (
    workflowRef: string,
    endpointRef: string,
    intervalSeconds: number,
    invocationPrompt: string,
    policy: "skip" | "queue",
    maxWakeups?: number | null,
  ) => Promise<WorkflowWatchdogPayload>
  listWorkflowWatchdogs?: (workflowRef?: string | null) => Promise<{ watchdogs: WorkflowWatchdogDefinition[] }>
  setWorkflowWatchdogEnabled?: (watchdogRef: string, enabled: boolean) => Promise<WorkflowWatchdogPayload>
  removeWorkflowWatchdog?: (watchdogRef: string) => Promise<WorkflowWatchdogPayload>
  setWorkflowFlushContext?: (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowLaunchPolicy?: (policy: "reject" | "queue") => Promise<{ session: RuntimeSession }>
  listQueuedWorkflowLaunches?: () => Promise<QueuedWorkflowLaunch[]>
  removeQueuedWorkflowLaunch?: (queueItemRef: string) => Promise<QueuedWorkflowLaunchPayload>
  clearQueuedWorkflowLaunches?: () => Promise<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>
  listWorkflowRuns?: (workflowRef?: string | null) => Promise<WorkflowRun[]>
  cancelWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunCancelPayload>
  resumeWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunResumePayload>
  updateWorkflowNodeInstructions?: (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => Promise<WorkflowNodeInstructionsPayload>
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
  setWorkflowRunOutputSchema?: (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowIntermediateOutputSchema?: (
    workflowRef: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  openWorkflowNodeInstructionsEditor?: (workflowId: string, nodeId: string, draft: string) => void
  closeWorkflowNodeInstructionsEditor?: () => void
  getWorkflowNodeInstructionsDraft?: () => string
  getWorkflowNodeInstructionsContext?: () => { workflowId: string; nodeId: string } | null
  openWorkflowTerminalPanel?: (workflowId: string) => void
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
}

export async function handleWorkflowSlashCommand(
  deps: WorkflowCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "workflow" }>,
): Promise<void> {
  const context = workflowCommandContext(deps)

  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to manage workflows", "error")
    return
  }

  const args = command.args
  const subcommand = args[0]

  if (!subcommand) {
    const knownWorkflows = deps.sessionState().workflows ?? []
    if (knownWorkflows.length > 0) {
      if (!deps.workflowScreenActive()) {
        deps.selectWorkflowCanvas(knownWorkflows[0]?.id ?? null)
        deps.showWorkflowScreen()
      }
      return
    }

    if (!deps.workflowScreenActive()) {
      deps.showWorkflowScreen()
      return
    }

    const workflows = await deps.listWorkflows()
    if (workflows.length > 0) {
      deps.replaceWorkflowDefinitions(workflows)
      deps.selectWorkflowCanvas(workflows[0]?.id ?? null)
    } else {
      const payload = await deps.createWorkflow(null)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.applySessionState(payload.session)
      deps.flashFooter(`created workflow ${payload.workflow.id}`, "info")
    }
    return
  }

  if (subcommand === "list") {
    const workflows = await deps.listWorkflows()
    deps.replaceWorkflowDefinitions(workflows)
    deps.flashFooter(
      workflows.length === 0
        ? "no workflows in workspace"
        : `workflows: ${workflows.map((workflow) => workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id).join(", ")}`,
      "info",
    )
    return
  }

  if (subcommand === "show") {
    const workflowRef = context.workflowRefOrSelected(args[1])
    if (!workflowRef) {
      deps.flashFooter("usage: /workflow show [workflow-ref]", "error")
      return
    }
    const payload = await deps.resolveWorkflow(workflowRef)
    deps.upsertWorkflowDefinition(payload.workflow)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.showWorkflowScreen()
    deps.flashFooter(
      `workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
      "info",
    )
    return
  }

  if (subcommand === "new") {
    const payload = await deps.createWorkflow(args[1] ?? null)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.showWorkflowScreen()
    deps.applySessionState(payload.session)
    deps.flashFooter(
      `created workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
      "info",
    )
    return
  }

  if (subcommand === "run" || subcommand === "start") {
    await handleWorkflowInvokeCommand(deps, context, args)
    return
  }

  if (isWorkflowSettingsCommand(subcommand)) {
    await handleWorkflowSettingsCommand(deps, context, args)
    return
  }

  if (subcommand === "queue") {
    await handleWorkflowQueueCommand(deps, args)
    return
  }

  if (subcommand === "runs") {
    await handleWorkflowRunsCommand(deps, args)
    return
  }

  if (subcommand === "cancel") {
    await handleWorkflowRunCancelCommand(deps, args)
    return
  }

  if (subcommand === "resume") {
    await handleWorkflowRunResumeCommand(deps, args)
    return
  }

  if (subcommand === "terminal") {
    await handleWorkflowTerminalCommand({
      ...deps,
      sessionWorkflows: () => deps.sessionState().workflows ?? [],
      workflowRefOrSelected: context.workflowRefOrSelected,
    }, args)
    return
  }

  if (subcommand === "add" && args[1] === "node") {
    const explicitWorkflowRef = args.length >= 4 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const target = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || target !== "all") {
      deps.flashFooter("usage: /workflow add node [workflow-ref] all", "error")
      return
    }
    await addAllRemainingWorkflowNodes(deps, workflowRef)
    return
  }

  if (subcommand === "node") {
    await handleWorkflowNodeCommand(deps, context, args)
    return
  }

  if (subcommand === "edge") {
    await handleWorkflowEdgeCommand(deps, context, args)
    return
  }

  if (subcommand === "endpoint") {
    await handleWorkflowEndpointCommand(deps, context, args)
    return
  }

  if (subcommand === "watchdog") {
    await handleWorkflowWatchdogCommand(deps, context, args)
    return
  }

  if (hasWorkflowEdgeShorthandArgs(args)) {
    await handleWorkflowEdgeShorthandCommand(deps, args)
    return
  }

  const alias = args[1]
  if (!alias) {
    deps.flashFooter(
      "usage: /workflow | /workflow list | /workflow show [workflow-ref] | /workflow new [alias] | /workflow run|start [workflow-ref] <endpoint-ref> [prompt] | /workflow max-turns <count|off> | /workflow run-output-schema [workflow-ref] [schema-ref|none] | /workflow intermediate-output-schema [workflow-ref] [schema-ref|none] | /workflow runs [workflow-ref] | /workflow cancel <run-ref> | /workflow resume <run-ref> | /workflow terminal [workflow-ref] | /workflow <workflow-ref> <alias> | /workflow <workflow-ref> <from-node-or-agent-ref> <to-node-or-agent-ref> | /workflow node ... | /workflow edge ... | /workflow endpoint ...",
      "error",
    )
    return
  }

  const workflow = await deps.assignWorkflowAlias(subcommand, alias)
  if (!workflow) {
    deps.flashFooter(`unknown workflow: ${subcommand}`, "error")
    return
  }
  deps.upsertWorkflowDefinition(workflow)
  deps.showWorkflowScreen()
  deps.flashFooter(`workflow ${workflow.id} aliased as ${workflow.alias}`, "info")
}

type WorkflowCommandContext = ReturnType<typeof workflowCommandContext>

function workflowCommandContext(deps: WorkflowCommandHandlerDeps) {
  const selectedWorkflowRef = () => deps.selectedWorkflowId?.() ?? null
  const workflowRefOrSelected = (workflowRef: string | null | undefined) => workflowRef ?? selectedWorkflowRef()
  const isKnownWorkflowReference = (reference: string | undefined) => {
    if (!reference) {
      return false
    }
    if (reference === selectedWorkflowRef()) {
      return true
    }
    return (deps.sessionState().workflows ?? []).some((workflow) => (
      workflow.id === reference || workflow.alias === reference
    ))
  }
  const firstWorkflowArgIsExplicit = (workflowRef: string | undefined) => (
    !selectedWorkflowRef() || isKnownWorkflowReference(workflowRef)
  )
  return {
    firstWorkflowArgIsExplicit,
    isKnownWorkflowReference,
    selectedWorkflowRef,
    workflowRefOrSelected,
  }
}

async function handleWorkflowNodeCommand(
  deps: WorkflowCommandHandlerDeps,
  context: WorkflowCommandContext,
  args: string[],
): Promise<void> {
  const action = args[1]
  if (action === "add") {
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
    return
  }
  if (action === "remove") {
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
    return
  }
  if (action === "instructions") {
    await handleWorkflowNodeInstructionsCommand(deps, context, args)
    return
  }
  if (action === "can-complete-run") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const nodeId = explicitWorkflowRef ? args[3] : args[2]
    const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
    if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
      deps.flashFooter("usage: /workflow node can-complete-run [workflow-ref] <node-id> <true|false>", "error")
      return
    }
    if (!deps.setWorkflowNodeCanCompleteRun) {
      deps.flashFooter("workflow runtime commands unavailable", "error")
      return
    }
    const payload = await deps.setWorkflowNodeCanCompleteRun(workflowRef, nodeId, value === "true")
    deps.applySessionState(payload.session)
    deps.upsertWorkflowDefinition(payload.workflow)
    deps.flashFooter(
      `workflow node ${payload.node.id} can-complete-run set to ${payload.node.can_complete_workflow_run ? "true" : "false"}`,
      "info",
    )
    return
  }
  if (action === "can-emit-intermediate-output") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const nodeId = explicitWorkflowRef ? args[3] : args[2]
    const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
    if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
      deps.flashFooter("usage: /workflow node can-emit-intermediate-output [workflow-ref] <node-id> <true|false>", "error")
      return
    }
    if (!deps.setWorkflowNodeCanEmitIntermediateOutput) {
      deps.flashFooter("workflow runtime commands unavailable", "error")
      return
    }
    const payload = await deps.setWorkflowNodeCanEmitIntermediateOutput(workflowRef, nodeId, value === "true")
    deps.applySessionState(payload.session)
    deps.upsertWorkflowDefinition(payload.workflow)
    deps.flashFooter(
      `workflow node ${payload.node.id} can-emit-intermediate-output set to ${payload.node.can_emit_intermediate_run_output ? "true" : "false"}`,
      "info",
    )
    return
  }
  if (action === "intermediate-output-schema") {
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
    return
  }
  if (action === "max-turns") {
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
    let maxTurns: number | null
    if (value === "none") {
      maxTurns = null
    } else {
      const parsed = Number.parseInt(value, 10)
      if (!Number.isFinite(parsed) || parsed <= 0) {
        deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
        return
      }
      maxTurns = parsed
    }
    const payload = await deps.setWorkflowNodeMaxTurns(workflowRef, nodeId, maxTurns)
    deps.applySessionState(payload.session)
    deps.upsertWorkflowDefinition(payload.workflow)
    deps.flashFooter(
      `workflow node ${payload.node.id} max-turns set to ${payload.node.max_turns ?? "none"}`,
      "info",
    )
    return
  }
  deps.flashFooter(
    "usage: /workflow node add [workflow-ref] <agent-id|all> | remove [workflow-ref] <node-id> | instructions ... | can-complete-run [workflow-ref] <node-id> <true|false> | can-emit-intermediate-output [workflow-ref] <node-id> <true|false> | intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none> | max-turns [workflow-ref] <node-id> <count|none>",
    "error",
  )
}

async function addAllRemainingWorkflowNodes(
  deps: WorkflowCommandHandlerDeps,
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
