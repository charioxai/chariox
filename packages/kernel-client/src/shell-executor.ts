import type {
  AgentInstance,
  QueuedWorkflowLaunch,
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./kernel-types.js"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  aliasWorkflowEndpointRequest,
  aliasWorkflowRequest,
  bindWorkflowEndpointRequest,
  cancelActivePromptRequest,
  cancelWorkflowRunRequest,
  clearQueuedWorkflowLaunchesRequest,
  createWorkflowEndpointRequest,
  createWorkflowRequest,
  createWorkflowWatchdogRequest,
  deleteKernelRequest,
  getSessionStateRequest,
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
  listQueuedWorkflowLaunchesRequest,
  listWorkflowWatchdogsRequest,
  listWorkflowRunsRequest,
  listWorkflowsRequest,
  removeQueuedWorkflowLaunchRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  removeWorkflowWatchdogRequest,
  resolveWorkflowRequest,
  resumeWorkflowRunRequest,
  setWorkflowFlushContextRequest,
  setWorkflowIntermediateOutputSchemaRequest,
  setWorkflowLaunchPolicyRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowNodeMaxTurnsRequest,
  setWorkflowRunOutputSchemaRequest,
  setWorkflowWatchdogEnabledRequest,
  updateSessionConfigRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { executeShellLocalCommand } from "./shell-local-command.js"
import { executeAgentCommand } from "./shell-agent-command.js"
import {
  executeMcpCommand,
  executeSkillCommand,
} from "./shell-capability-command.js"
import {
  resolveShellAgent,
} from "./shell-agent-resolver.js"
import { executeHistoryCommand } from "./shell-history-command.js"
import {
  executeConfigCommand,
  executeCredentialCommand,
} from "./shell-config-command.js"
import { executeContextCommand } from "./shell-context-command.js"
import {
  executeClientCommand,
  executeMachineCommand,
  executeRelayCommand,
} from "./shell-remote-command.js"
import { executeSessionCommand } from "./shell-session-command.js"
import { executeCloudCommand } from "./shell-cloud-command.js"
import { executeSliceCommand } from "./shell-slice-command.js"
import { executePromptCommand } from "./shell-prompt-command.js"
import { resolveShellAttachmentId } from "./shell-session-attachment.js"
import { executeProviderCommand } from "./shell-provider-command.js"
import {
  type LocalGitWorktreeOptions,
  type ShellPlacementDeps,
} from "./shell-placement.js"
import { executeWorkflowPublicationCommand } from "./shell-workflow-publication-command.js"
import {
  formatQueuedWorkflowLaunches,
  formatWorkflowDetails,
  formatWorkflowLabel,
  formatWorkflowList,
  formatWorkflowRunList,
  formatWorkflowWatchdogs,
} from "./shell-workflow-format.js"
import { executeWorkspaceCommand } from "./shell-workspace-command.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellExecutorDeps = ShellPlacementDeps & {
  client: ShellKernelClient
  clientId?: string | undefined
  readSecret?: ((prompt: string) => Promise<string>) | undefined
  prepareLocalGitWorktree?: ((options: LocalGitWorktreeOptions) => Promise<string>) | undefined
}

export async function executeShellCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  if (parsed.kind === "empty") {
    return { ok: true, message: "" }
  }
  if (parsed.kind === "invalid") {
    return { ok: false, message: parsed.reason ?? "invalid command" }
  }
  if (parsed.kind === "tui-only") {
    return { ok: false, message: parsed.reason ?? `${parsed.command ?? "command"} is only available in the TUI client` }
  }
  if (parsed.kind === "shell-local") {
    return executeShellLocalCommand(parsed, context)
  }
  switch (parsed.command) {
    case "session":
      return executeSessionCommand(parsed, context, deps)
    case "agent":
      return executeAgentCommand(parsed, context, deps)
    case "kernel":
      return executeKernelCommand(parsed, context, deps)
    case "client":
      return executeClientCommand(parsed, deps)
    case "machine":
      return executeMachineCommand(parsed, deps)
    case "slice":
      return executeSliceCommand(parsed, context, deps)
    case "relay":
      return executeRelayCommand(parsed, deps)
    case "cloud":
      return executeCloudCommand(parsed, context, deps)
    case "config":
      return executeConfigCommand(parsed, deps)
    case "credential":
      return executeCredentialCommand(parsed, deps)
    case "mcp":
      return executeMcpCommand(parsed, context, deps)
    case "skill":
      return executeSkillCommand(parsed, context, deps)
    case "workflow":
      return executeWorkflowCommand(parsed, context, deps)
    case "workspace":
      return executeWorkspaceCommand(parsed, context, deps)
    case "history":
      return executeHistoryCommand(parsed, context, deps)
    case "prompt":
      return executePromptCommand(parsed, context, deps)
    case "stop":
    case "cancel":
      return executeStopCommand(parsed, context, deps)
    case "provider":
      return executeProviderCommand(parsed, context, deps)
    case "context":
      return executeContextCommand(context, deps)
    default:
      return {
        ok: false,
        message: `${parsed.command ?? "command"} is not implemented in arroba-shell yet`,
      }
  }
}

async function executeKernelCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  if (action !== "delete" || args.length > 0) {
    return { ok: false, message: "usage: kernel delete" }
  }
  const response = await deps.client.send(deleteKernelRequest())
  const payload = expectVariant<{ kernel_id: string; deleted_sessions: RuntimeSession[] }>(response, "KernelDeleted")
  const deletedCurrentSession = context.sessionId
    ? payload.deleted_sessions.some((session) => session.id === context.sessionId)
    : false
  return {
    ok: true,
    message: `deleted kernel ${payload.kernel_id} (${payload.deleted_sessions.length} session${payload.deleted_sessions.length === 1 ? "" : "s"})`,
    contextUpdates: deletedCurrentSession
      ? { sessionId: undefined, attachmentId: undefined, agentId: undefined }
      : undefined,
    data: payload,
  }
}

async function executeWorkflowCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listWorkflowsRequest(sessionId))
      const workflows = expectVariant<{ workflows: WorkflowDefinition[] }>(response, "WorkflowsListed").workflows
      return { ok: true, message: formatWorkflowList(workflows, context.workflowId), data: { workflows } }
    }
    case "new":
    case "create": {
      const response = await deps.client.send(createWorkflowRequest(sessionId, args[0] ?? null))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowCreated")
      return resourceResult(
        `created workflow ${formatWorkflowLabel(payload.workflow)}`,
        parsed.assignment,
        payload.workflow.id,
        { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
        payload,
      )
    }
    case "show": {
      const workflowRef = args[0] ?? context.workflowId
      if (!workflowRef) {
        return { ok: false, message: "usage: workflow show <workflow-ref>" }
      }
      const response = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
      const workflow = expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved").workflow
      return { ok: true, message: formatWorkflowDetails(workflow), data: { workflow }, contextUpdates: { workflowId: workflow.id } }
    }
    case "alias": {
      const [workflowRef, alias] = args
      if (!workflowRef || !alias) {
        return { ok: false, message: "usage: workflow alias <workflow-ref> <alias>" }
      }
      const response = await deps.client.send(aliasWorkflowRequest(sessionId, workflowRef, alias))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowAliased")
      return { ok: true, message: `workflow ${payload.workflow.id} aliased as ${payload.workflow.alias}`, data: payload, contextUpdates: { workflowId: payload.workflow.id } }
    }
    case "run":
    case "start": {
      const [workflowRef, endpointRef, ...promptParts] = args
      if (!workflowRef || !endpointRef) {
        return { ok: false, message: `usage: workflow ${action} <workflow-ref> <endpoint-ref> [prompt]` }
      }
      const prompt = promptParts.join(" ").trim() || null
      const response = await deps.client.send(invokeWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, prompt))
      if ("WorkflowRunInvoked" in response) {
        const payload = response.WorkflowRunInvoked as {
          workflow_run: WorkflowRun
          workflow: WorkflowDefinition
          endpoint: WorkflowEndpointDefinition
          session: RuntimeSession
        }
        return {
          ok: true,
          message: `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
          data: payload,
          contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
        }
      }
      const payload = expectVariant<{
        queued_launch: QueuedWorkflowLaunch
        workflow: WorkflowDefinition
        endpoint: WorkflowEndpointDefinition
        session: RuntimeSession
      }>(response, "WorkflowRunQueued")
      return {
        ok: true,
        message: `queued workflow launch ${payload.queued_launch.id}; active workflow run in session`,
        data: payload,
        contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    case "launch-policy": {
      const value = args[0]?.trim().toLowerCase()
      if (!value) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        return { ok: true, message: `workflow launch policy: ${session.workflow_launch_policy ?? "reject"}`, data: { session } }
      }
      if (value !== "reject" && value !== "queue") {
        return { ok: false, message: "usage: workflow launch-policy <reject|queue>" }
      }
      const response = await deps.client.send(setWorkflowLaunchPolicyRequest(sessionId, value))
      const payload = expectVariant<{ session: RuntimeSession }>(response, "WorkflowLaunchPolicyUpdated")
      return { ok: true, message: `workflow launch policy set to ${value}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "flush-context": {
      const first = args[0]?.trim().toLowerCase()
      const firstIsValue = first === "true" || first === "false"
      const workflowRef = firstIsValue ? context.workflowId : (args[0] ?? context.workflowId)
      const value = firstIsValue ? first : args[1]?.trim().toLowerCase()
      if (!workflowRef || (value !== "true" && value !== "false")) {
        return { ok: false, message: "usage: workflow flush-context [workflow-ref] <true|false>" }
      }
      const response = await deps.client.send(setWorkflowFlushContextRequest(sessionId, workflowRef, value === "true"))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowFlushContextUpdated")
      return { ok: true, message: `workflow ${payload.workflow.id} flush-context set to ${String(payload.workflow.flush_agent_context_before_run ?? true)}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "run-output-schema":
    case "intermediate-output-schema": {
      const explicit = args.length >= 2 ? args[0] : null
      const workflowRef = explicit ?? context.workflowId
      const rawValue = explicit ? args[1] : args[0]
      if (!workflowRef || rawValue === undefined) {
        return { ok: false, message: `usage: workflow ${action} [workflow-ref] <schema-ref|none>` }
      }
      const schemaRef = rawValue.trim().toLowerCase() === "none" ? null : rawValue
      const response = await deps.client.send(action === "run-output-schema"
        ? setWorkflowRunOutputSchemaRequest(sessionId, workflowRef, schemaRef)
        : setWorkflowIntermediateOutputSchemaRequest(sessionId, workflowRef, schemaRef))
      const variant = action === "run-output-schema" ? "WorkflowRunOutputSchemaUpdated" : "WorkflowIntermediateOutputSchemaUpdated"
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, variant)
      const field = action === "run-output-schema" ? "run-output-schema" : "intermediate-output-schema"
      const value = action === "run-output-schema" ? payload.workflow.run_output_schema_ref : payload.workflow.intermediate_output_schema_ref
      return { ok: true, message: `workflow ${payload.workflow.id} ${field} set to ${value ?? "none"}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "max-turns": {
      const value = args[0]?.trim().toLowerCase()
      if (!value) {
        return { ok: false, message: "usage: workflow max-turns <count|off>" }
      }
      const nextValue = value === "off" || value === "0"
        ? "0"
        : Number.isFinite(Number(value)) ? String(Math.max(1, Math.floor(Number(value)))) : null
      if (!nextValue) {
        return { ok: false, message: "usage: workflow max-turns <count|off>" }
      }
      const attachmentId = await resolveShellAttachmentId(context, deps)
      if (!attachmentId.ok) {
        return { ok: false, message: attachmentId.message }
      }
      const response = await deps.client.send(updateSessionConfigRequest(sessionId, attachmentId.attachmentId, { "workflow.max_turns": nextValue }, false))
      const payload = expectVariant<{ session: RuntimeSession; config: SessionConfigState }>(response, "SessionConfigUpdated")
      return { ok: true, message: nextValue === "0" ? "workflow max turns disabled" : `workflow max turns set to ${nextValue}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "runs": {
      const response = await deps.client.send(listWorkflowRunsRequest(sessionId, args[0] ?? null))
      const workflowRuns = expectVariant<{ workflow_runs: WorkflowRun[] }>(response, "WorkflowRunsListed").workflow_runs
      return { ok: true, message: formatWorkflowRunList(workflowRuns, args[0] ?? null), data: { workflow_runs: workflowRuns } }
    }
    case "run-show":
    case "run-get": {
      const workflowRunRef = args[0]
      if (!workflowRunRef) {
        return { ok: false, message: `usage: workflow ${action} <run-ref>` }
      }
      const response = await deps.client.send(getWorkflowRunRequest(sessionId, workflowRunRef))
      const workflowRun = expectVariant<{ workflow_run: WorkflowRun }>(response, "WorkflowRun").workflow_run
      return { ok: true, message: JSON.stringify(workflowRun, null, 2), data: { workflow_run: workflowRun }, format: "json" }
    }
    case "cancel":
    case "resume": {
      const workflowRunRef = args[0]
      if (!workflowRunRef) {
        return { ok: false, message: `usage: workflow ${action} <run-ref>` }
      }
      const response = await deps.client.send(action === "cancel"
        ? cancelWorkflowRunRequest(sessionId, workflowRunRef)
        : resumeWorkflowRunRequest(sessionId, workflowRunRef))
      const variant = action === "cancel" ? "WorkflowRunCancelled" : "WorkflowRunResumed"
      const payload = expectVariant<{ workflow_run: WorkflowRun; session: RuntimeSession }>(response, variant)
      return {
        ok: true,
        message: `${action === "cancel" ? "cancelled" : "resumed"} workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    case "node":
      return executeWorkflowNodeCommand(args, parsed, context, deps)
    case "edge":
      return executeWorkflowEdgeCommand(args, context, deps)
    case "endpoint":
      return executeWorkflowEndpointCommand(args, context, deps)
    case "publication":
    case "publish":
      return executeWorkflowPublicationCommand(args, context, deps)
    case "watchdog":
      return executeWorkflowWatchdogCommand(args, context, deps)
    case "queue":
      return executeWorkflowQueueCommand(args, context, deps)
    default:
      return { ok: false, message: "usage: workflow list|new|show|alias|run|runs|run-show|cancel|resume|node|edge|endpoint|publication|watchdog|queue" }
  }
}

async function executeWorkflowNodeCommand(
  args: string[],
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action, maybeWorkflowRef, maybeNodeOrAgent] = args
  const workflowRef = args.length >= 3 ? maybeWorkflowRef : context.workflowId
  const target = args.length >= 3 ? maybeNodeOrAgent : maybeWorkflowRef
  if (action === "add") {
    if (!workflowRef || !target) {
      return { ok: false, message: "usage: workflow node add [workflow-ref] <agent-ref>" }
    }
    const agent = await resolveShellAgent(context, deps, target)
    if (!agent.ok) {
      return { ok: false, message: agent.message }
    }
    const response = await deps.client.send(addWorkflowNodeRequest(sessionId, workflowRef, agent.agent.id))
    const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowNodeAdded")
    return resourceResult(
      `added workflow node ${payload.node.id} for agent ${agent.agent.agent_ref}`,
      parsed.assignment,
      payload.node.id,
      { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      payload,
    )
  }
  if (action === "remove") {
    if (!workflowRef || !target) {
      return { ok: false, message: "usage: workflow node remove [workflow-ref] <node-id>" }
    }
    const response = await deps.client.send(removeWorkflowNodeRequest(sessionId, workflowRef, target))
    const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowNodeRemoved")
    return { ok: true, message: `removed workflow node ${payload.node.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (
    action === "can-complete-run"
    || action === "can-emit-intermediate-output"
    || action === "intermediate-output-schema"
    || action === "max-turns"
  ) {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const nodeId = explicitWorkflowRef ? args[2] : args[1]
    const value = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !nodeId || value === undefined) {
      return { ok: false, message: "usage: workflow node can-complete-run|can-emit-intermediate-output|intermediate-output-schema|max-turns [workflow-ref] <node-id> <value>" }
    }
    let request: Record<string, unknown>
    let variant: string
    let renderedValue: string
    if (action === "can-complete-run" || action === "can-emit-intermediate-output") {
      const normalized = value.trim().toLowerCase()
      if (normalized !== "true" && normalized !== "false") {
        return { ok: false, message: `usage: workflow node ${action} [workflow-ref] <node-id> <true|false>` }
      }
      const bool = normalized === "true"
      request = action === "can-complete-run"
        ? setWorkflowNodeCanCompleteRunRequest(sessionId, workflowRef, nodeId, bool)
        : setWorkflowNodeCanEmitIntermediateOutputRequest(sessionId, workflowRef, nodeId, bool)
      variant = action === "can-complete-run" ? "WorkflowNodeCanCompleteRunUpdated" : "WorkflowNodeCanEmitIntermediateOutputUpdated"
      renderedValue = normalized
    } else if (action === "intermediate-output-schema") {
      const schemaRef = value.trim().toLowerCase() === "none" ? null : value
      request = setWorkflowNodeIntermediateOutputSchemaRequest(sessionId, workflowRef, nodeId, schemaRef)
      variant = "WorkflowNodeIntermediateOutputSchemaUpdated"
      renderedValue = schemaRef ?? "none"
    } else {
      const normalized = value.trim().toLowerCase()
      const maxTurns = normalized === "none" ? null : Number.parseInt(normalized, 10)
      if (maxTurns !== null && (!Number.isFinite(maxTurns) || maxTurns <= 0)) {
        return { ok: false, message: "usage: workflow node max-turns [workflow-ref] <node-id> <count|none>" }
      }
      request = setWorkflowNodeMaxTurnsRequest(sessionId, workflowRef, nodeId, maxTurns)
      variant = "WorkflowNodeMaxTurnsUpdated"
      renderedValue = maxTurns === null ? "none" : String(maxTurns)
    }
    const response = await deps.client.send(request)
    const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, variant)
    return { ok: true, message: `workflow node ${payload.node.id} ${action} set to ${renderedValue}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow node add [workflow-ref] <agent-ref> | remove [workflow-ref] <node-id> | can-complete-run|can-emit-intermediate-output|intermediate-output-schema|max-turns ..." }
}

async function executeWorkflowEdgeCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "add") {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const fromNodeId = explicitWorkflowRef ? args[2] : args[1]
    const toNodeId = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !fromNodeId || !toNodeId) {
      return { ok: false, message: "usage: workflow edge add [workflow-ref] <from-node-id> <to-node-id>" }
    }
    const response = await deps.client.send(addWorkflowEdgeRequest(sessionId, workflowRef, fromNodeId, toNodeId))
    const payload = expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEdgeAdded")
    return { ok: true, message: `added workflow edge ${payload.edge.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "remove") {
    const explicitWorkflowRef = args.length >= 3 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const edgeId = explicitWorkflowRef ? args[2] : args[1]
    if (!workflowRef || !edgeId) {
      return { ok: false, message: "usage: workflow edge remove [workflow-ref] <edge-id>" }
    }
    const response = await deps.client.send(removeWorkflowEdgeRequest(sessionId, workflowRef, edgeId))
    const payload = expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEdgeRemoved")
    return { ok: true, message: `removed workflow edge ${payload.edge.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow edge add [workflow-ref] <from-node-id> <to-node-id> | remove [workflow-ref] <edge-id>" }
}

async function executeWorkflowEndpointCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "new" || action === "create") {
    const explicitWorkflowRef = args.length >= 3 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const entryNodeId = explicitWorkflowRef ? args[2] : args[1]
    const alias = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !entryNodeId) {
      return { ok: false, message: "usage: workflow endpoint new [workflow-ref] <entry-node-id> [alias]" }
    }
    const response = await deps.client.send(createWorkflowEndpointRequest(sessionId, workflowRef, entryNodeId, alias ?? null))
    const payload = expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEndpointCreated")
    return { ok: true, message: `created workflow endpoint ${payload.endpoint.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "alias") {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? args[2] : args[1]
    const alias = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !endpointRef || !alias) {
      return { ok: false, message: "usage: workflow endpoint alias [workflow-ref] <endpoint-ref> <alias>" }
    }
    const response = await deps.client.send(aliasWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, alias))
    const payload = expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEndpointAliased")
    return { ok: true, message: `workflow endpoint ${payload.endpoint.id} aliased as ${payload.endpoint.alias}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "bind") {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? args[2] : args[1]
    const entryNodeId = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !endpointRef || !entryNodeId) {
      return { ok: false, message: "usage: workflow endpoint bind [workflow-ref] <endpoint-ref> <entry-node-id>" }
    }
    const response = await deps.client.send(bindWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, entryNodeId))
    const payload = expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEndpointBound")
    return { ok: true, message: `workflow endpoint ${payload.endpoint.id} bound to node ${payload.endpoint.entry_node_id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind [workflow-ref] <endpoint-ref> <entry-node-id>" }
}

async function executeWorkflowWatchdogCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "list" || !action) {
    const workflowRef = args[1] ?? null
    const response = await deps.client.send(listWorkflowWatchdogsRequest(sessionId, workflowRef))
    const watchdogs = expectVariant<{ watchdogs: WorkflowWatchdogDefinition[] }>(response, "WorkflowWatchdogsListed").watchdogs
    return { ok: true, message: formatWorkflowWatchdogs(watchdogs), data: { watchdogs } }
  }
  if (action === "enable" || action === "disable") {
    const watchdogRef = args[1]
    if (!watchdogRef) {
      return { ok: false, message: `usage: workflow watchdog ${action} <watchdog-ref>` }
    }
    const response = await deps.client.send(setWorkflowWatchdogEnabledRequest(sessionId, watchdogRef, action === "enable"))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(response, "WorkflowWatchdogUpdated")
    return { ok: true, message: `${action === "enable" ? "enabled" : "disabled"} workflow watchdog ${payload.watchdog.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "remove") {
    const watchdogRef = args[1]
    if (!watchdogRef) {
      return { ok: false, message: "usage: workflow watchdog remove <watchdog-ref>" }
    }
    const response = await deps.client.send(removeWorkflowWatchdogRequest(sessionId, watchdogRef))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(response, "WorkflowWatchdogRemoved")
    return { ok: true, message: `removed workflow watchdog ${payload.watchdog.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "add") {
    const explicitWorkflowRef = args[3] === "every" ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? args[2] : args[1]
    const everyLiteral = explicitWorkflowRef ? args[3] : args[2]
    const intervalLiteral = explicitWorkflowRef ? args[4] : args[3]
    const optionStart = explicitWorkflowRef ? 5 : 4
    if (!workflowRef || !endpointRef || everyLiteral !== "every" || !intervalLiteral) {
      return { ok: false, message: "usage: workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [prompt]" }
    }
    const intervalSeconds = parseWatchdogIntervalSeconds(intervalLiteral)
    if (!intervalSeconds) {
      return { ok: false, message: "watchdog interval must be like 30s, 5m, 1h, or 1d" }
    }
    const hasPolicy = args[optionStart] === "skip" || args[optionStart] === "queue"
    const policy = (hasPolicy ? args[optionStart] : "skip") as "skip" | "queue"
    const prompt = args.slice(optionStart + (hasPolicy ? 1 : 0)).join(" ").trim() || "Run the workflow exactly as instructed."
    const response = await deps.client.send(createWorkflowWatchdogRequest(sessionId, workflowRef, endpointRef, intervalSeconds, prompt, policy))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; workflow: WorkflowDefinition; endpoint: WorkflowEndpointDefinition; session: RuntimeSession }>(response, "WorkflowWatchdogCreated")
    return { ok: true, message: `created workflow watchdog ${payload.watchdog.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow watchdog add|list|enable|disable|remove" }
}

async function executeWorkflowQueueCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const action = args[0] ?? "list"
  if (action === "list") {
    const response = await deps.client.send(listQueuedWorkflowLaunchesRequest(sessionId))
    const queuedLaunches = expectVariant<{ queued_launches: QueuedWorkflowLaunch[] }>(response, "QueuedWorkflowLaunchesListed").queued_launches
    return { ok: true, message: formatQueuedWorkflowLaunches(queuedLaunches), data: { queued_launches: queuedLaunches } }
  }
  if (action === "flush" || action === "clear") {
    const response = await deps.client.send(clearQueuedWorkflowLaunchesRequest(sessionId))
    const payload = expectVariant<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>(response, "QueuedWorkflowLaunchesCleared")
    return { ok: true, message: payload.queued_launches.length === 0 ? "workflow queue already empty" : `cleared ${payload.queued_launches.length} queued workflow launch${payload.queued_launches.length === 1 ? "" : "es"}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "remove") {
    const queueItemRef = args[1]
    if (!queueItemRef) {
      return { ok: false, message: "usage: workflow queue remove <queue-item-ref>" }
    }
    const response = await deps.client.send(removeQueuedWorkflowLaunchRequest(sessionId, queueItemRef))
    const payload = expectVariant<{ queued_launch: QueuedWorkflowLaunch; session: RuntimeSession }>(response, "QueuedWorkflowLaunchRemoved")
    return { ok: true, message: `removed queued workflow launch ${payload.queued_launch.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow queue [list|flush|remove <queue-item-ref>]" }
}

async function executeStopCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  if (parsed.args.length > 0) {
    return { ok: false, message: "usage: stop" }
  }
  if (!context.sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const attachmentId = await resolveShellAttachmentId(context, deps)
  if (!attachmentId.ok) {
    return { ok: false, message: attachmentId.message }
  }
  const response = await deps.client.send(cancelActivePromptRequest(context.sessionId, attachmentId.attachmentId))
  const payload = expectVariant<{ cancellation: { prompt?: { id?: string | null } | null } }>(response, "PromptCancelled")
  return { ok: true, message: `cancellation requested${payload.cancellation.prompt?.id ? ` for prompt ${payload.cancellation.prompt.id}` : ""}`, data: payload }
}

function parseWatchdogIntervalSeconds(value: string | undefined): number | null {
  const match = value?.trim().match(/^(\d+)([smhd])$/i)
  if (!match) return null
  const amount = Number.parseInt(match[1]!, 10)
  if (!Number.isFinite(amount) || amount <= 0) return null
  const unit = match[2]!.toLowerCase()
  const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
  return amount * multiplier
}

function resourceResult(
  message: string,
  assignment: string | undefined,
  value: string,
  contextUpdates: ShellCommandResult["contextUpdates"],
  data: unknown,
): ShellCommandResult {
  return {
    ok: true,
    message,
    data,
    bindings: assignment ? { [assignment]: value } : undefined,
    contextUpdates,
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
