import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt,
  WorkflowScheduleDefinition,
  WorkflowScheduleTrigger,
} from "./kernel-types.js"
import {
  clearWorkflowPromptQueueRequest,
  createWorkflowPromptQueueRequest,
  createWorkflowScheduleRequest,
  listQueuedWorkflowPromptsRequest,
  listWorkflowPromptQueuesRequest,
  listWorkflowSchedulesRequest,
  previewWorkflowScheduleRequest,
  removeQueuedWorkflowPromptRequest,
  removeWorkflowScheduleRequest,
  setWorkflowScheduleEnabledRequest,
  updateQueuedWorkflowPromptRequest,
  updateWorkflowPromptQueueRequest,
} from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import { sessionContextAgentId } from "./shell-session-context.js"
import {
  formatWorkflowPromptQueues,
  formatWorkflowQueuedPrompts,
  formatWorkflowSchedules,
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
  return executeWorkflowScheduleCommand(args, context, deps)
}

export async function executeWorkflowScheduleCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowAutomationCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "list" || !action) {
    const workflowRef = args[1] ?? null
    const response = await deps.client.send(listWorkflowSchedulesRequest(sessionId, workflowRef))
    const schedules = expectVariant<{ schedules: WorkflowScheduleDefinition[] }>(response, "WorkflowSchedulesListed").schedules
    return { ok: true, message: formatWorkflowSchedules(schedules), data: { schedules } }
  }
  if (action === "enable" || action === "disable") {
    const scheduleRef = args[1]
    if (!scheduleRef) {
      return { ok: false, message: `usage: workflow schedule ${action} <schedule-ref>` }
    }
    const response = await deps.client.send(setWorkflowScheduleEnabledRequest(sessionId, scheduleRef, action === "enable"))
    const payload = expectVariant<{ schedule: WorkflowScheduleDefinition; session: RuntimeSession }>(response, "WorkflowScheduleUpdated")
    return { ok: true, message: `${action === "enable" ? "enabled" : "disabled"} workflow schedule ${payload.schedule.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "remove") {
    const scheduleRef = args[1]
    if (!scheduleRef) {
      return { ok: false, message: "usage: workflow schedule remove <schedule-ref>" }
    }
    const response = await deps.client.send(removeWorkflowScheduleRequest(sessionId, scheduleRef))
    const payload = expectVariant<{ schedule: WorkflowScheduleDefinition; session: RuntimeSession }>(response, "WorkflowScheduleRemoved")
    return { ok: true, message: `removed workflow schedule ${payload.schedule.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "preview") {
    const parsed = parseWorkflowScheduleOptions(args.slice(1))
    if (parsed.error || !parsed.trigger) {
      return { ok: false, message: parsed.error ?? "usage: workflow schedule preview --cron \"15 30 14 * * *\" --tz UTC" }
    }
    const response = await deps.client.send(previewWorkflowScheduleRequest(parsed.trigger, null, 3))
    const payload = expectVariant<{ preview: { trigger: WorkflowScheduleTrigger; next_run_at_ms: number[] } }>(response, "WorkflowSchedulePreviewed")
    return {
      ok: true,
      message: payload.preview.next_run_at_ms.map((runAtMs) => new Date(runAtMs).toISOString()).join("\n"),
      data: payload,
    }
  }
  if (action === "add") {
    const positionalEnd = args.findIndex((arg, index) => index > 0 && arg.startsWith("--"))
    const positional = positionalEnd === -1 ? args.slice(1) : args.slice(1, positionalEnd)
    const options = positionalEnd === -1 ? [] : args.slice(positionalEnd)
    const parsed = parseWorkflowScheduleOptions(options)
    if (parsed.error || !parsed.trigger) {
      return { ok: false, message: parsed.error ?? workflowScheduleUsage() }
    }
    const explicitWorkflowRef = positional.length >= 2 ? positional[0] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? positional[1] : positional[0]
    if (!workflowRef || !endpointRef) {
      return { ok: false, message: workflowScheduleUsage() }
    }
    const response = await deps.client.send(createWorkflowScheduleRequest(
      sessionId,
      workflowRef,
      endpointRef,
      parsed.trigger,
      parsed.prompt ?? "Run the workflow exactly as instructed.",
      parsed.overlapPolicy ?? "skip",
      parsed.maxRuns,
      parsed.queueRef,
    ))
    const payload = expectVariant<{ schedule: WorkflowScheduleDefinition; workflow: WorkflowDefinition; endpoint: WorkflowEndpointDefinition; session: RuntimeSession }>(response, "WorkflowScheduleCreated")
    return { ok: true, message: `created workflow schedule ${payload.schedule.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  return { ok: false, message: "usage: workflow schedule add|list|enable|disable|remove|preview" }
}

function workflowScheduleUsage(): string {
  return "usage: workflow schedule add [workflow-ref] <endpoint-ref> (--every 5m | --cron \"15 30 14 * * *\" --tz UTC) [--queue <queue-ref>] [--overlap skip|queue] [--max-runs <n|null>] [--prompt <text>]"
}

function parseWorkflowScheduleOptions(args: string[]): {
  trigger?: WorkflowScheduleTrigger
  queueRef?: string | null
  overlapPolicy?: "skip" | "queue"
  maxRuns?: number | null
  prompt?: string
  error?: string
} {
  let trigger: WorkflowScheduleTrigger | undefined
  let timezone = "UTC"
  let queueRef: string | null | undefined
  let overlapPolicy: "skip" | "queue" | undefined
  let maxRuns: number | null | undefined
  let prompt: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if (arg === "--every") {
      const seconds = parseScheduleIntervalSeconds(next)
      if (!seconds) return { error: "schedule interval must be like 30s, 5m, 1h, or 1d" }
      trigger = { kind: "interval", every_seconds: seconds }
      index += 1
      continue
    }
    if (arg === "--cron") {
      if (!next) return { error: "schedule cron expression is required" }
      trigger = { kind: "cron", expression: next, timezone }
      index += 1
      continue
    }
    if (arg === "--tz") {
      if (!next) return { error: "schedule timezone is required" }
      timezone = next
      if (trigger?.kind === "cron") trigger = { ...trigger, timezone }
      index += 1
      continue
    }
    if (arg === "--queue") {
      if (!next) return { error: "schedule queue is required" }
      queueRef = next
      index += 1
      continue
    }
    if (arg === "--overlap") {
      if (next !== "skip" && next !== "queue") return { error: "schedule overlap must be skip or queue" }
      overlapPolicy = next
      index += 1
      continue
    }
    if (arg === "--max-runs") {
      const parsed = parseScheduleMaxRuns(next)
      if (parsed === undefined) return { error: "max-runs must be a positive integer or `null`" }
      maxRuns = parsed
      index += 1
      continue
    }
    if (arg === "--prompt") {
      prompt = args.slice(index + 1).join(" ").trim()
      break
    }
    return { error: `unknown workflow schedule option ${arg}` }
  }
  const result: {
    trigger?: WorkflowScheduleTrigger
    queueRef?: string | null
    overlapPolicy?: "skip" | "queue"
    maxRuns?: number | null
    prompt?: string
  } = {}
  if (trigger) result.trigger = trigger
  if (queueRef !== undefined) result.queueRef = queueRef
  if (overlapPolicy !== undefined) result.overlapPolicy = overlapPolicy
  if (maxRuns !== undefined) result.maxRuns = maxRuns
  if (prompt !== undefined) result.prompt = prompt
  return result
}

function parseScheduleIntervalSeconds(value: string | undefined): number | null {
  if (!value) return null
  const match = value.trim().toLowerCase().match(/^(\d+)(s|m|h|d)$/)
  if (!match) return null
  const amount = Number(match[1])
  const unit = match[2]
  if (!Number.isFinite(amount) || amount <= 0) return null
  const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
  return amount * multiplier
}

function parseScheduleMaxRuns(value: string | undefined): number | null | undefined {
  if (value == null) return undefined
  const normalized = value.trim().toLowerCase()
  if (!normalized) return undefined
  if (normalized === "null" || normalized === "unbounded") return null
  const numeric = Number(normalized)
  if (!Number.isFinite(numeric) || numeric <= 0 || !Number.isInteger(numeric)) return undefined
  return numeric
}

export async function executeWorkflowQueueCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowAutomationCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const parsed = parseWorkflowQueueArgs(args)
  if (parsed.error) {
    return { ok: false, message: "usage: workflow queue [--workflow <workflow-ref>] [list|create|priority|edit|move|clear|remove]" }
  }
  const action = parsed.args[0] ?? "list"
  const workflowRef = parsed.workflowRef ?? context.workflowId ?? null
  if (action === "list") {
    const [queueResponse, promptResponse] = await Promise.all([
      deps.client.send(listWorkflowPromptQueuesRequest(sessionId, workflowRef)),
      deps.client.send(listQueuedWorkflowPromptsRequest(sessionId)),
    ])
    const queues = expectVariant<{ queues: WorkflowPromptQueueDefinition[] }>(queueResponse, "WorkflowPromptQueuesListed").queues
    const queuedPrompts = expectVariant<{ queued_prompts: WorkflowQueuedPrompt[] }>(promptResponse, "QueuedWorkflowPromptsListed").queued_prompts
    const workflowIds = new Set(queues.map((queue) => queue.workflow_id))
    const workflowPrompts = queuedPrompts.filter((prompt) => workflowIds.has(prompt.workflow_id))
    return { ok: true, message: `${formatWorkflowPromptQueues(queues, workflowPrompts)}${workflowPrompts.length ? `\n${formatWorkflowQueuedPrompts(workflowPrompts)}` : ""}`, data: { queues, queued_prompts: workflowPrompts } }
  }
  if (action === "create") {
    const alias = parsed.args[1]
    const priority = Number(parsed.args[2] ?? "0")
    if (!alias || !Number.isFinite(priority)) return { ok: false, message: "usage: workflow queue create [--workflow <workflow-ref>] <alias> [priority]" }
    const response = await deps.client.send(createWorkflowPromptQueueRequest(sessionId, workflowRef, alias, priority))
    const payload = expectVariant<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>(response, "WorkflowPromptQueueCreated")
    return { ok: true, message: `created workflow queue ${payload.queue.id} (${payload.queue.alias})`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "priority") {
    const queueRef = parsed.args[1]
    const priority = Number(parsed.args[2])
    if (!queueRef || !Number.isFinite(priority)) return { ok: false, message: "usage: workflow queue priority [--workflow <workflow-ref>] <queue-ref> <priority>" }
    const response = await deps.client.send(updateWorkflowPromptQueueRequest(sessionId, workflowRef, queueRef, { priority }))
    const payload = expectVariant<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>(response, "WorkflowPromptQueueUpdated")
    return { ok: true, message: `updated workflow queue ${payload.queue.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "flush" || action === "clear") {
    const queueRef = parsed.args[1] ?? "default"
    const response = await deps.client.send(clearWorkflowPromptQueueRequest(sessionId, workflowRef, queueRef))
    const payload = expectVariant<{ queued_prompts: WorkflowQueuedPrompt[]; session: RuntimeSession }>(response, "WorkflowPromptQueueCleared")
    return { ok: true, message: payload.queued_prompts.length === 0 ? `workflow queue ${queueRef} already empty` : `cleared ${payload.queued_prompts.length} queued workflow prompt${payload.queued_prompts.length === 1 ? "" : "s"}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "remove") {
    const queueItemRef = parsed.args[1]
    if (!queueItemRef) {
      return { ok: false, message: "usage: workflow queue remove [--workflow <workflow-ref>] <queue-item-ref>" }
    }
    const response = await deps.client.send(removeQueuedWorkflowPromptRequest(sessionId, queueItemRef))
    const payload = expectVariant<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>(response, "QueuedWorkflowPromptRemoved")
    return { ok: true, message: `removed queued workflow prompt ${payload.queued_prompt.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "edit") {
    const queueItemRef = parsed.args[1]
    const prompt = parsed.args.slice(2).join(" ").trim()
    if (!queueItemRef || !prompt) return { ok: false, message: "usage: workflow queue edit [--workflow <workflow-ref>] <queue-item-ref> <prompt>" }
    const response = await deps.client.send(updateQueuedWorkflowPromptRequest(sessionId, queueItemRef, { prompt }))
    const payload = expectVariant<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>(response, "QueuedWorkflowPromptUpdated")
    return { ok: true, message: `updated queued workflow prompt ${payload.queued_prompt.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  if (action === "move") {
    const queueItemRef = parsed.args[1]
    const queueRef = parsed.args[2]
    if (!queueItemRef || !queueRef) return { ok: false, message: "usage: workflow queue move [--workflow <workflow-ref>] <queue-item-ref> <queue-ref>" }
    const response = await deps.client.send(updateQueuedWorkflowPromptRequest(sessionId, queueItemRef, { queueRef }))
    const payload = expectVariant<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>(response, "QueuedWorkflowPromptUpdated")
    return { ok: true, message: `moved queued workflow prompt ${payload.queued_prompt.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) } }
  }
  return { ok: false, message: "usage: workflow queue [list|create|priority|edit|move|clear|remove]" }
}

function parseWorkflowQueueArgs(args: readonly string[]): { args: string[]; workflowRef?: string; error?: boolean } {
  const normalized: string[] = []
  let workflowRef: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]
    if (value === undefined) {
      continue
    }
    if (value === "--workflow") {
      const next = args[index + 1]?.trim()
      if (!next) {
        return { args: normalized, error: true }
      }
      workflowRef = next
      index += 1
      continue
    }
    normalized.push(value)
  }
  return { args: normalized, ...(workflowRef ? { workflowRef } : {}) }
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
