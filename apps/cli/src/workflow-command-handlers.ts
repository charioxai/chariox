import type {
  AgentInstance,
  QueuedWorkflowLaunch,
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
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
import { readFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

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

type WorkflowWatchdogPayload = {
  watchdog: WorkflowWatchdogDefinition
  workflow?: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition
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
  ) => Promise<WorkflowNodePayload>
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

async function handleWorkflowNodeInstructionsCommand(
  deps: WorkflowCommandHandlerDeps,
  context: WorkflowCommandContext,
  args: string[],
): Promise<void> {
  const instructionsAction = args[2]
  if (!instructionsAction) {
    deps.flashFooter(
      "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
      "error",
    )
    return
  }
  if (instructionsAction === "close") {
    deps.closeWorkflowNodeInstructionsEditor?.()
    deps.flashFooter("closed node instructions editor", "info")
    return
  }
  if (instructionsAction === "save") {
    const contextState = deps.getWorkflowNodeInstructionsContext?.()
    if (!contextState || !deps.updateWorkflowNodeInstructions || !deps.getWorkflowNodeInstructionsDraft) {
      deps.flashFooter("no workflow node instructions editor is open", "error")
      return
    }
    const payload = await deps.updateWorkflowNodeInstructions(
      contextState.workflowId,
      contextState.nodeId,
      deps.getWorkflowNodeInstructionsDraft(),
    )
    deps.applySessionState(payload.session)
    deps.upsertWorkflowDefinition(payload.workflow)
    deps.closeWorkflowNodeInstructionsEditor?.()
    deps.flashFooter(`saved node instructions for ${payload.node.id}`, "info")
    return
  }
  const explicitWorkflowRef = context.firstWorkflowArgIsExplicit(args[3]) ? args[3] : null
  const instructionsWorkflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[4] : args[3]
  const fileRef = explicitWorkflowRef ? args[5] : args[4]
  if (!instructionsWorkflowRef || !nodeId) {
    deps.flashFooter(
      "usage: /workflow node instructions show|set [workflow-ref] <node-id> [file]",
      "error",
    )
    return
  }
  const resolved = await deps.resolveWorkflow(instructionsWorkflowRef)
  deps.upsertWorkflowDefinition(resolved.workflow)
  const node = resolved.workflow.nodes?.find((entry) => entry.id === nodeId)
  if (!node) {
    deps.flashFooter(`workflow node ${nodeId} not found`, "error")
    return
  }
  if (instructionsAction === "show") {
    deps.openWorkflowNodeInstructionsEditor?.(
      resolved.workflow.id,
      node.id,
      node.instructions ?? "",
    )
    deps.selectWorkflowCanvas(resolved.workflow.id)
    deps.flashFooter(`opened node ${node.id} instructions in the I/O panel`, "info")
    return
  }
  if (instructionsAction !== "set") {
    deps.flashFooter(
      "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
      "error",
    )
    return
  }
  if (fileRef) {
    if (!deps.updateWorkflowNodeInstructions) {
      deps.flashFooter("workflow instructions unavailable", "error")
      return
    }
    const content = await readFile(resolvePath(deps.currentWorkspaceTarget(), fileRef), "utf8")
    const payload = await deps.updateWorkflowNodeInstructions(resolved.workflow.id, node.id, content)
    deps.applySessionState(payload.session)
    deps.upsertWorkflowDefinition(payload.workflow)
    deps.flashFooter(`updated node instructions for ${payload.node.id}`, "info")
    return
  }
  deps.openWorkflowNodeInstructionsEditor?.(resolved.workflow.id, node.id, node.instructions ?? "")
  deps.selectWorkflowCanvas(resolved.workflow.id)
  deps.flashFooter("editing node instructions in the I/O panel; submit text then /workflow node instructions save", "info")
}

async function handleWorkflowWatchdogCommand(
  deps: WorkflowCommandHandlerDeps,
  context: WorkflowCommandContext,
  args: string[],
): Promise<void> {
  const action = args[1]
  if (action === "add") {
    if (!deps.createWorkflowWatchdog) {
      deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
      return
    }
    const explicitWorkflowRef = args[4] === "every" ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const endpointRef = explicitWorkflowRef ? args[3] : args[2]
    const everyLiteral = explicitWorkflowRef ? args[4] : args[3]
    const intervalLiteral = explicitWorkflowRef ? args[5] : args[4]
    const optionStartIndex = explicitWorkflowRef ? 6 : 5
    const hasPolicyArg = args[optionStartIndex] === "skip" || args[optionStartIndex] === "queue"
    const policy = (hasPolicyArg ? args[optionStartIndex] : "skip") as "skip" | "queue"
    const maxWakeupsKeyword = args[optionStartIndex + (hasPolicyArg ? 1 : 0)]
    const hasMaxWakeupsArg = maxWakeupsKeyword === "max-wakeups"
    const maxWakeupsLiteral = hasMaxWakeupsArg ? args[optionStartIndex + (hasPolicyArg ? 2 : 1)] : undefined
    const maxWakeups = hasMaxWakeupsArg ? parseWatchdogMaxWakeups(maxWakeupsLiteral) : undefined
    const prompt = args
      .slice(optionStartIndex + (hasPolicyArg ? 1 : 0) + (hasMaxWakeupsArg ? 2 : 0))
      .join(" ")
      .trim() || "Run the workflow exactly as instructed."
    if (!workflowRef || !endpointRef || everyLiteral !== "every") {
      deps.flashFooter(
        "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt]",
        "error",
      )
      return
    }
    const intervalSeconds = parseWatchdogIntervalSeconds(intervalLiteral)
    if (!intervalSeconds) {
      deps.flashFooter("watchdog interval must be like 30s, 5m, 1h, or 1d", "error")
      return
    }
    if (hasMaxWakeupsArg && maxWakeups === undefined) {
      deps.flashFooter("max-wakeups must be a positive integer or `null`", "error")
      return
    }
    const payload = await deps.createWorkflowWatchdog(
      workflowRef,
      endpointRef,
      intervalSeconds,
      prompt,
      policy,
      maxWakeups,
    )
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow?.id ?? workflowRef)
    deps.flashFooter(`created workflow watchdog ${payload.watchdog.id}`, "info")
    return
  }
  if (action === "list") {
    if (!deps.listWorkflowWatchdogs) {
      deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
      return
    }
    const workflowRef = args[2] ?? null
    const payload = await deps.listWorkflowWatchdogs(workflowRef)
    if (payload.watchdogs.length === 0) {
      deps.flashFooter("no workflow watchdogs configured", "info")
      return
    }
    deps.appendNotice(payload.watchdogs.map((watchdog) =>
      `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"} next=${new Date(watchdog.next_run_at_ms).toISOString()}${watchdog.pending_run ? " pending=true" : ""}`
    ).join("\n"))
    deps.flashFooter(`listed ${payload.watchdogs.length} workflow watchdog(s)`, "info")
    return
  }
  if (action === "enable" || action === "disable") {
    if (!deps.setWorkflowWatchdogEnabled) {
      deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
      return
    }
    const watchdogRef = args[2]
    if (!watchdogRef) {
      deps.flashFooter(`usage: /workflow watchdog ${action} <watchdog-ref>`, "error")
      return
    }
    const payload = await deps.setWorkflowWatchdogEnabled(watchdogRef, action === "enable")
    deps.applySessionState(payload.session)
    deps.flashFooter(
      `${action === "enable" ? "enabled" : "disabled"} workflow watchdog ${payload.watchdog.id}`,
      "info",
    )
    return
  }
  if (action === "remove") {
    if (!deps.removeWorkflowWatchdog) {
      deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
      return
    }
    const watchdogRef = args[2]
    if (!watchdogRef) {
      deps.flashFooter("usage: /workflow watchdog remove <watchdog-ref>", "error")
      return
    }
    const payload = await deps.removeWorkflowWatchdog(watchdogRef)
    deps.applySessionState(payload.session)
    deps.flashFooter(`removed workflow watchdog ${payload.watchdog.id}`, "info")
    return
  }
  deps.flashFooter(
    "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt] | list [workflow-ref] | enable <watchdog-ref> | disable <watchdog-ref> | remove <watchdog-ref>",
    "error",
  )
}

function parseWatchdogIntervalSeconds(value: string | undefined): number | null {
  if (!value) return null
  const match = value.trim().toLowerCase().match(/^(\d+)(s|m|h|d)$/)
  if (!match) return null
  const amount = Number(match[1])
  const unit = match[2]
  if (!Number.isFinite(amount) || amount <= 0) return null
  const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
  return amount * multiplier
}

function parseWatchdogMaxWakeups(value: string | undefined): number | null | undefined {
  if (value == null) return undefined
  const normalized = value.trim().toLowerCase()
  if (!normalized) return undefined
  if (normalized === "null" || normalized === "unbounded") return null
  const numeric = Number(normalized)
  if (!Number.isFinite(numeric) || numeric <= 0 || !Number.isInteger(numeric)) {
    return undefined
  }
  return numeric
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
