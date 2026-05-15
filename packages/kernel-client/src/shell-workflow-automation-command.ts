import type {
  QueuedWorkflowLaunch,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowWatchdogDefinition,
} from "./kernel-types.js"
import {
  clearQueuedWorkflowLaunchesRequest,
  createWorkflowWatchdogRequest,
  listQueuedWorkflowLaunchesRequest,
  listWorkflowWatchdogsRequest,
  removeQueuedWorkflowLaunchRequest,
  removeWorkflowWatchdogRequest,
  setWorkflowWatchdogEnabledRequest,
} from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatQueuedWorkflowLaunches,
  formatWorkflowWatchdogs,
} from "./shell-workflow-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellWorkflowAutomationCommandDeps = {
  client: ShellKernelClient
}

export async function executeWorkflowWatchdogCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowAutomationCommandDeps,
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

export async function executeWorkflowQueueCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowAutomationCommandDeps,
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

function parseWatchdogIntervalSeconds(value: string | undefined): number | null {
  const match = value?.trim().match(/^(\d+)([smhd])$/i)
  if (!match) return null
  const amount = Number.parseInt(match[1]!, 10)
  if (!Number.isFinite(amount) || amount <= 0) return null
  const unit = match[2]!.toLowerCase()
  const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
  return amount * multiplier
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
