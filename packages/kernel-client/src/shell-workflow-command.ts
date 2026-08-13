import type {
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
} from "./kernel-types.js"
import {
  aliasWorkflowRequest,
  cancelWorkflowRunRequest,
  createWorkflowRequest,
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
  listWorkflowRunsRequest,
  listWorkflowsRequest,
  resolveWorkflowRequest,
  resumeWorkflowRunRequest,
  setWorkflowFlushContextRequest,
  setWorkflowRunOutputSchemaRequest,
  updateSessionConfigRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAttachmentId } from "./shell-session-attachment.js"
import { sessionContextAgentId } from "./shell-session-context.js"
import {
  executeWorkflowQueueCommand,
  executeWorkflowScheduleCommand,
  executeWorkflowWatchdogCommand,
} from "./shell-workflow-automation-command.js"
import {
  executeWorkflowEdgeCommand,
  executeWorkflowEndpointCommand,
  executeWorkflowNodeCommand,
} from "./shell-workflow-graph-command.js"
import { executeWorkflowCodeCommand } from "./shell-workflow-code-command.js"
import { executeWorkflowPublicationCommand } from "./shell-workflow-publication-command.js"
import {
  executeWorkflowRegistryCommand,
  executeWorkflowRegistryLoadCommand,
  executeWorkflowRegistryRunCommand,
  workflowRunLooksLikeRegistryLoad,
} from "./shell-workflow-registry-command.js"
import {
  formatWorkflowDetails,
  formatWorkflowLabel,
  formatWorkflowList,
  formatWorkflowRunList,
} from "./shell-workflow-format.js"
import {
  applyShellWorkflowDesignOp,
  createShellWorkflowDesignId,
} from "./shell-workflow-design-op.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellWorkflowCommandDeps = {
  client: ShellKernelClient
  clientId?: string | undefined
}

export async function executeWorkflowCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellWorkflowCommandDeps,
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
      if (deps.clientId) {
        const workflowId = createShellWorkflowDesignId("workflow")
        const payload = await applyShellWorkflowDesignOp(
          { client: deps.client, clientId: deps.clientId },
          sessionId,
          {
            kind: "workflow_create",
            workflow: { id: workflowId, alias: args[0] ?? null },
          },
        )
        const workflow = workflowFromSession(payload.session, workflowId)
        return resourceResult(
          `created workflow ${formatWorkflowLabel(workflow)}`,
          parsed.assignment,
          workflow.id,
          { workflowId: workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
          { ...payload, workflow },
        )
      }
      const response = await deps.client.send(createWorkflowRequest(sessionId, args[0] ?? null))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowCreated")
      return resourceResult(
        `created workflow ${formatWorkflowLabel(payload.workflow)}`,
        parsed.assignment,
        payload.workflow.id,
        { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
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
      if (deps.clientId) {
        const resolved = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
        const workflow = expectVariant<{ workflow: WorkflowDefinition }>(resolved, "WorkflowResolved").workflow
        const payload = await applyShellWorkflowDesignOp(
          { client: deps.client, clientId: deps.clientId },
          sessionId,
          { kind: "workflow_update", workflow_id: workflow.id, patch: { alias } },
        )
        const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
        return {
          ok: true,
          message: `workflow ${updatedWorkflow.id} aliased as ${updatedWorkflow.alias}`,
          data: { ...payload, workflow: updatedWorkflow },
          contextUpdates: { workflowId: updatedWorkflow.id },
        }
      }
      const response = await deps.client.send(aliasWorkflowRequest(sessionId, workflowRef, alias))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowAliased")
      return { ok: true, message: `workflow ${payload.workflow.id} aliased as ${payload.workflow.alias}`, data: payload, contextUpdates: { workflowId: payload.workflow.id } }
    }
    case "delete":
    case "remove":
    case "rm": {
      const workflowRef = args[0] ?? context.workflowId
      if (!workflowRef) {
        return { ok: false, message: `usage: workflow ${action} <workflow-ref>` }
      }
      if (!deps.clientId) {
        return { ok: false, message: "workflow deletion requires a connected client id" }
      }
      const resolved = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
      const workflow = expectVariant<{ workflow: WorkflowDefinition }>(resolved, "WorkflowResolved").workflow
      const payload = await applyShellWorkflowDesignOp(
        { client: deps.client, clientId: deps.clientId },
        sessionId,
        { kind: "workflow_remove", workflow_id: workflow.id },
      )
      return {
        ok: true,
        message: `deleted workflow ${formatWorkflowLabel(workflow)}`,
        data: payload,
        contextUpdates: { workflowId: undefined, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
      }
    }
    case "run":
    case "start": {
      if (action === "run" && workflowRunLooksLikeRegistryLoad(args)) {
        return executeWorkflowRegistryRunCommand(args, context, deps)
      }
      const [workflowRef, endpointRef, ...promptParts] = args
      if (!workflowRef || !endpointRef) {
        return { ok: false, message: `usage: workflow ${action} <workflow-ref> <endpoint-ref> [prompt] [--queue <queue-ref>]` }
      }
      const queueFlagIndex = promptParts.findIndex((part) => part === "--queue")
      const queueRef = queueFlagIndex >= 0 ? promptParts[queueFlagIndex + 1] ?? null : null
      const prompt = promptParts
        .filter((_, index) => queueFlagIndex < 0 || (index !== queueFlagIndex && index !== queueFlagIndex + 1))
        .join(" ")
        .trim() || null
      const response = await deps.client.send(invokeWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, prompt, queueRef))
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
          contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
        }
      }
      const payload = expectVariant<{
        queued_prompt: WorkflowQueuedPrompt
        workflow: WorkflowDefinition
        endpoint: WorkflowEndpointDefinition
        session: RuntimeSession
      }>(response, "WorkflowPromptEnqueued")
      return {
        ok: true,
        message: `queued workflow prompt ${payload.queued_prompt.id}`,
        data: payload,
        contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
      }
    }
    case "flush-context": {
      const first = args[0]?.trim().toLowerCase()
      const firstIsValue = first === "true" || first === "false"
      const workflowRef = firstIsValue ? context.workflowId : (args[0] ?? context.workflowId)
      const value = firstIsValue ? first : args[1]?.trim().toLowerCase()
      if (!workflowRef || (value !== "true" && value !== "false")) {
        return { ok: false, message: "usage: workflow flush-context [workflow-ref] <true|false>" }
      }
      if (deps.clientId) {
        const resolved = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
        const workflow = expectVariant<{ workflow: WorkflowDefinition }>(resolved, "WorkflowResolved").workflow
        const payload = await applyShellWorkflowDesignOp(
          { client: deps.client, clientId: deps.clientId },
          sessionId,
          {
            kind: "workflow_update",
            workflow_id: workflow.id,
            patch: { flush_agent_context_before_run: value === "true" },
          },
        )
        const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
        return {
          ok: true,
          message: `workflow ${updatedWorkflow.id} flush-context set to ${String(updatedWorkflow.flush_agent_context_before_run ?? true)}`,
          data: { ...payload, workflow: updatedWorkflow },
          contextUpdates: { workflowId: updatedWorkflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
        }
      }
      const response = await deps.client.send(setWorkflowFlushContextRequest(sessionId, workflowRef, value === "true"))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowFlushContextUpdated")
      return { ok: true, message: `workflow ${payload.workflow.id} flush-context set to ${String(payload.workflow.flush_agent_context_before_run ?? true)}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
    }
    case "run-output-schema": {
      const explicit = args.length >= 2 ? args[0] : null
      const workflowRef = explicit ?? context.workflowId
      const rawValue = explicit ? args[1] : args[0]
      if (!workflowRef || rawValue === undefined) {
        return { ok: false, message: "usage: workflow run-output-schema [workflow-ref] <schema-ref|none>" }
      }
      const schemaRef = rawValue.trim().toLowerCase() === "none" ? null : rawValue
      if (deps.clientId) {
        const resolved = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
        const workflow = expectVariant<{ workflow: WorkflowDefinition }>(resolved, "WorkflowResolved").workflow
        const payload = await applyShellWorkflowDesignOp(
          { client: deps.client, clientId: deps.clientId },
          sessionId,
          {
            kind: "workflow_update",
            workflow_id: workflow.id,
            patch: { run_output_schema_ref: schemaRef },
          },
        )
        const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
        return {
          ok: true,
          message: `workflow ${updatedWorkflow.id} run-output-schema set to ${updatedWorkflow.run_output_schema_ref ?? "none"}`,
          data: { ...payload, workflow: updatedWorkflow },
          contextUpdates: { workflowId: updatedWorkflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
        }
      }
      const response = await deps.client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflowRef, schemaRef))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowRunOutputSchemaUpdated")
      return { ok: true, message: `workflow ${payload.workflow.id} run-output-schema set to ${payload.workflow.run_output_schema_ref ?? "none"}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
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
      return { ok: true, message: nextValue === "0" ? "workflow max turns disabled" : `workflow max turns set to ${nextValue}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
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
        contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
      }
    }
    case "node":
      return executeWorkflowNodeCommand(args, parsed, context, deps)
    case "edge":
      return executeWorkflowEdgeCommand(args, context, deps)
    case "endpoint":
      return executeWorkflowEndpointCommand(args, context, deps)
    case "trigger":
      return executeWorkflowPublicationCommand(args, context, deps)
    case "watchdog":
      return executeWorkflowWatchdogCommand(args, context, deps)
    case "schedule":
      return executeWorkflowScheduleCommand(args, context, deps)
    case "queue":
      return executeWorkflowQueueCommand(args, context, deps)
    case "registry":
      return executeWorkflowRegistryCommand(args, context, deps)
    case "load":
      return executeWorkflowRegistryLoadCommand(args, context, deps)
    case "code":
    case "workflow-code":
      return executeWorkflowCodeCommand(args, context, deps)
    default:
      return { ok: false, message: "usage: workflow list|new|show|alias|delete|run|load|runs|run-show|cancel|resume|node|edge|endpoint|trigger|schedule|queue|registry|code" }
  }
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

function workflowFromSession(session: RuntimeSession, workflowId: string): WorkflowDefinition {
  const workflow = session.workflows?.find((candidate) => candidate.id === workflowId)
  if (!workflow) {
    throw new Error(`workflow design response did not include workflow ${workflowId}`)
  }
  return workflow
}
