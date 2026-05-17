import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowWatchdogPayload = {
  watchdog: WorkflowWatchdogDefinition
  workflow?: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition
  session: RuntimeSession
}

export type WorkflowWatchdogCommandContext = {
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowWatchdogCommandDeps = {
  appendNotice: (message: string) => void
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
  return `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"} next=${new Date(watchdog.next_run_at_ms).toISOString()}${watchdog.pending_run ? " pending=true" : ""}`
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
