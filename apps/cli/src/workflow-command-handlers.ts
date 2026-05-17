import type {
  AgentInstance,
  QueuedWorkflowLaunch,
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
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
  handleWorkflowAddAllNodesCommand,
  handleWorkflowNodeCommand,
  type WorkflowNodePayload,
} from "./workflow-node-command-handlers.js"
import {
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
    await handleWorkflowAddAllNodesCommand(deps, context, args)
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
