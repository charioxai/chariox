import type {
  AgentInstance,
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
  WorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
import { createWorkflowCommandContext } from "./workflow-command-context.js"
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
  handleWorkflowAliasCommand,
  handleWorkflowListCommand,
  handleWorkflowNewCommand,
  handleWorkflowRootCommand,
  handleWorkflowShowCommand,
  type WorkflowCreatePayload,
  type WorkflowResolvePayload,
} from "./workflow-lifecycle-command-handlers.js"
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
    queueRef?: string | null,
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
  listWorkflowPromptQueues?: (workflowRef?: string | null) => Promise<WorkflowPromptQueueDefinition[]>
  createWorkflowPromptQueue?: (workflowRef: string | null, alias: string, priority: number) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  updateWorkflowPromptQueue?: (
    workflowRef: string | null,
    queueRef: string,
    patch: { alias?: string | null; priority?: number | null; enabled?: boolean | null },
  ) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  removeWorkflowPromptQueue?: (workflowRef: string | null, queueRef: string) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  listQueuedWorkflowPrompts?: () => Promise<WorkflowQueuedPrompt[]>
  updateQueuedWorkflowPrompt?: (
    queueItemRef: string,
    patch: { prompt?: string | null; queueRef?: string | null },
  ) => Promise<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>
  removeQueuedWorkflowPrompt?: (queueItemRef: string) => Promise<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>
  clearWorkflowPromptQueue?: (workflowRef: string | null, queueRef: string) => Promise<{ queued_prompts: WorkflowQueuedPrompt[]; session: RuntimeSession }>
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
  grantAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  grantAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  grantAgentScript?: (agentRef: string, name: string, environment: string) => Promise<AgentInstance>
  revokeAgentScript?: (agentRef: string, name: string) => Promise<AgentInstance>
  grantAgentConnector?: (agentRef: string, name: string, credential?: string | null, maxSafety?: string | null) => Promise<AgentInstance>
  revokeAgentConnector?: (agentRef: string, name: string) => Promise<AgentInstance>
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
  const context = createWorkflowCommandContext(deps)

  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to manage workflows", "error")
    return
  }

  const args = command.args
  const subcommand = args[0]

  if (!subcommand) {
    await handleWorkflowRootCommand(deps)
    return
  }

  if (subcommand === "list") {
    await handleWorkflowListCommand(deps)
    return
  }

  if (subcommand === "show") {
    await handleWorkflowShowCommand(deps, context, args)
    return
  }

  if (subcommand === "new") {
    await handleWorkflowNewCommand(deps, args)
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

  await handleWorkflowAliasCommand(deps, args)
}
