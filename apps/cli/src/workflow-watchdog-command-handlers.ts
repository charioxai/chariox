import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowScheduleDefinition,
  WorkflowScheduleTrigger,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowWatchdogPayload = {
  watchdog: WorkflowWatchdogDefinition
  workflow?: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition
  session: RuntimeSession
}

export type WorkflowSchedulePayload = {
  schedule: WorkflowScheduleDefinition
  workflow?: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition
  session: RuntimeSession
}

export type WorkflowWatchdogCommandContext = {
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowWatchdogCommandDeps = {
  appendNotice: (message: string) => void
  createWorkflowSchedule?: (
    workflowRef: string,
    endpointRef: string,
    trigger: WorkflowScheduleTrigger,
    invocationPrompt: string,
    overlapPolicy: "skip" | "queue",
    maxRuns?: number | null,
    queueRef?: string | null,
  ) => Promise<WorkflowSchedulePayload>
  listWorkflowSchedules?: (workflowRef?: string | null) => Promise<{ schedules: WorkflowScheduleDefinition[] }>
  setWorkflowScheduleEnabled?: (scheduleRef: string, enabled: boolean) => Promise<WorkflowSchedulePayload>
  removeWorkflowSchedule?: (scheduleRef: string) => Promise<WorkflowSchedulePayload>
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
  applySessionState: (session: RuntimeSession) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowWatchdogCommand(
  deps: WorkflowWatchdogCommandDeps,
  context: WorkflowWatchdogCommandContext,
  args: readonly string[],
): Promise<void> {
  const action = args[1]
  if (args[0] === "schedule") {
    await handleWorkflowScheduleCommand(deps, context, args)
    return
  }
  if (action === "add") {
    await handleWorkflowWatchdogAddCommand(deps, context, args)
    return
  }
  if (action === "list") {
    await handleWorkflowWatchdogListCommand(deps, args)
    return
  }
  if (action === "enable" || action === "disable") {
    await handleWorkflowWatchdogToggleCommand(deps, action, args)
    return
  }
  if (action === "remove") {
    await handleWorkflowWatchdogRemoveCommand(deps, args)
    return
  }
  deps.flashFooter(
    "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt] | list [workflow-ref] | enable <watchdog-ref> | disable <watchdog-ref> | remove <watchdog-ref>",
    "error",
  )
}

async function handleWorkflowScheduleCommand(
  deps: WorkflowWatchdogCommandDeps,
  context: WorkflowWatchdogCommandContext,
  args: readonly string[],
): Promise<void> {
  const action = args[1]
  if (action === "add") {
    await handleWorkflowScheduleAddCommand(deps, context, args)
    return
  }
  if (action === "list" || !action) {
    if (!deps.listWorkflowSchedules) {
      deps.flashFooter("workflow schedules are unavailable in this build", "error")
      return
    }
    const payload = await deps.listWorkflowSchedules(args[2] ?? null)
    if (payload.schedules.length === 0) {
      deps.flashFooter("no workflow schedules configured", "info")
      return
    }
    deps.appendNotice(payload.schedules.map(formatWorkflowSchedule).join("\n"))
    deps.flashFooter(`listed ${payload.schedules.length} workflow schedule(s)`, "info")
    return
  }
  if (action === "enable" || action === "disable") {
    if (!deps.setWorkflowScheduleEnabled) {
      deps.flashFooter("workflow schedules are unavailable in this build", "error")
      return
    }
    const scheduleRef = args[2]
    if (!scheduleRef) {
      deps.flashFooter(`usage: /workflow schedule ${action} <schedule-ref>`, "error")
      return
    }
    const payload = await deps.setWorkflowScheduleEnabled(scheduleRef, action === "enable")
    deps.applySessionState(payload.session)
    deps.flashFooter(`${action === "enable" ? "enabled" : "disabled"} workflow schedule ${payload.schedule.id}`, "info")
    return
  }
  if (action === "remove") {
    if (!deps.removeWorkflowSchedule) {
      deps.flashFooter("workflow schedules are unavailable in this build", "error")
      return
    }
    const scheduleRef = args[2]
    if (!scheduleRef) {
      deps.flashFooter("usage: /workflow schedule remove <schedule-ref>", "error")
      return
    }
    const payload = await deps.removeWorkflowSchedule(scheduleRef)
    deps.applySessionState(payload.session)
    deps.flashFooter(`removed workflow schedule ${payload.schedule.id}`, "info")
    return
  }
  deps.flashFooter(scheduleUsage(), "error")
}

async function handleWorkflowScheduleAddCommand(
  deps: WorkflowWatchdogCommandDeps,
  context: WorkflowWatchdogCommandContext,
  args: readonly string[],
): Promise<void> {
  if (!deps.createWorkflowSchedule) {
    deps.flashFooter("workflow schedules are unavailable in this build", "error")
    return
  }
  const positionalEnd = args.findIndex((arg, index) => index > 1 && arg.startsWith("--"))
  const positional = positionalEnd === -1 ? args.slice(2) : args.slice(2, positionalEnd)
  const options = positionalEnd === -1 ? [] : args.slice(positionalEnd)
  const explicitWorkflowRef = positional.length >= 2 ? positional[0] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const endpointRef = explicitWorkflowRef ? positional[1] : positional[0]
  const parsed = parseWorkflowScheduleOptions(options)
  if (!workflowRef || !endpointRef || parsed.error || !parsed.trigger) {
    deps.flashFooter(parsed.error ?? scheduleUsage(), "error")
    return
  }
  const payload = await deps.createWorkflowSchedule(
    workflowRef,
    endpointRef,
    parsed.trigger,
    parsed.prompt ?? "Run the workflow exactly as instructed.",
    parsed.overlapPolicy ?? "skip",
    parsed.maxRuns,
    parsed.queueRef,
  )
  deps.applySessionState(payload.session)
  deps.selectWorkflowCanvas(payload.workflow?.id ?? workflowRef)
  deps.flashFooter(`created workflow schedule ${payload.schedule.id}`, "info")
}

async function handleWorkflowWatchdogAddCommand(
  deps: WorkflowWatchdogCommandDeps,
  context: WorkflowWatchdogCommandContext,
  args: readonly string[],
): Promise<void> {
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
}

async function handleWorkflowWatchdogListCommand(
  deps: WorkflowWatchdogCommandDeps,
  args: readonly string[],
): Promise<void> {
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
  deps.appendNotice(payload.watchdogs.map(formatWorkflowWatchdog).join("\n"))
  deps.flashFooter(`listed ${payload.watchdogs.length} workflow watchdog(s)`, "info")
}

async function handleWorkflowWatchdogToggleCommand(
  deps: WorkflowWatchdogCommandDeps,
  action: "enable" | "disable",
  args: readonly string[],
): Promise<void> {
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
}

async function handleWorkflowWatchdogRemoveCommand(
  deps: WorkflowWatchdogCommandDeps,
  args: readonly string[],
): Promise<void> {
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
}

function formatWorkflowWatchdog(watchdog: WorkflowWatchdogDefinition): string {
  const intervalSeconds = watchdog.interval_seconds ?? (watchdog.trigger.kind === "interval" ? watchdog.trigger.every_seconds : 0)
  const policy = watchdog.policy ?? watchdog.overlap_policy
  const wakeupsExecuted = watchdog.wakeups_executed ?? watchdog.runs_started
  const maxWakeups = watchdog.max_wakeups ?? watchdog.max_runs
  return `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${intervalSeconds}s policy=${policy} enabled=${String(watchdog.enabled)} wakeups=${wakeupsExecuted}/${maxWakeups ?? "unbounded"} next=${new Date(watchdog.next_run_at_ms).toISOString()}${watchdog.pending_run ? " pending=true" : ""}`
}

function formatWorkflowSchedule(schedule: WorkflowScheduleDefinition): string {
  return `${schedule.id} workflow=${schedule.workflow_id} endpoint=${schedule.endpoint_id} trigger=${formatWorkflowScheduleTrigger(schedule.trigger)} overlap=${schedule.overlap_policy} enabled=${String(schedule.enabled)} runs=${schedule.runs_started}/${schedule.max_runs ?? "unbounded"} next=${new Date(schedule.next_run_at_ms).toISOString()}${schedule.pending_run ? " pending=true" : ""}`
}

function formatWorkflowScheduleTrigger(trigger: WorkflowScheduleTrigger): string {
  if (trigger.kind === "interval") {
    return `every:${trigger.every_seconds}s`
  }
  return `cron:${trigger.expression} tz=${trigger.timezone}`
}

function scheduleUsage(): string {
  return "usage: /workflow schedule add [workflow-ref] <endpoint-ref> (--every 5m | --cron \"15 30 14 * * *\" --tz UTC) [--queue <queue-ref>] [--overlap skip|queue] [--max-runs <n|null>] [--prompt <text>] | list [workflow-ref] | enable <schedule-ref> | disable <schedule-ref> | remove <schedule-ref>"
}

function parseWorkflowScheduleOptions(args: readonly string[]): {
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
      const seconds = parseWatchdogIntervalSeconds(next)
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
      const parsed = parseWatchdogMaxWakeups(next)
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
