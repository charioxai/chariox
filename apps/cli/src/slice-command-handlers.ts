import type {
  SliceDisplayEndpoint,
  SliceLogEntry,
  SliceRecord,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
import {
  formatSliceDiagnostics,
  formatSliceOperation,
  formatSliceProviderAuth,
  formatSliceProviderAuthReadiness,
  formatSliceProviderList,
  formatSliceRelayLabel,
  formatSliceScope,
  sliceProviderAuthCoverage,
} from "./slice-format.js"

type FooterTone = "info" | "error"

type SliceCreateOptions = {
  name: string
  backend?: "local_docker" | "ssh_docker"
  os?: string
  displayMode?: "headless" | "headed"
  workspaceId?: string | null
  worktreeId?: string | null
  workspaceMount?: string | null
  workerKernelRef?: string | null
  displayUrl?: string | null
}

export type SliceCommandHandlerDeps = {
  currentWorkspaceTarget: () => string
  currentWorktreeTarget: () => string
  focusedAgentId: () => string | null
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  openExternalUrl?: (url: string) => Promise<boolean>
  listSlices?: () => Promise<SliceRecord[]>
  createSlice?: (options: SliceCreateOptions) => Promise<SliceRecord>
  getSlice?: (sliceRef: string) => Promise<SliceRecord>
  startSlice?: (sliceRef: string) => Promise<SliceRecord>
  stopSlice?: (sliceRef: string) => Promise<SliceRecord>
  deleteSlice?: (sliceRef: string) => Promise<SliceRecord>
  importSliceProviderAuth?: (sliceRef: string, provider: string) => Promise<{ slice: SliceRecord; provider: string; status: string }>
  removeSliceProviderAuth?: (sliceRef: string, provider: string) => Promise<{ slice: SliceRecord; provider: string; status: string }>
  startSliceProviderLogin?: (sliceRef: string, provider: string) => Promise<{ slice: SliceRecord; login: SliceProviderLogin }>
  setSliceProviderAuthAlias?: (sliceRef: string, provider: string, alias: string | null) => Promise<{ slice: SliceRecord; provider: string; alias: string | null }>
  getSliceDisplayEndpoint?: (sliceRef: string) => Promise<SliceDisplayEndpoint>
  getSliceLogs?: (sliceRef: string, tailLines?: number | null) => Promise<{ slice: SliceRecord; entries: SliceLogEntry[] }>
  listSliceAudit?: (sliceRef: string, limit?: number | null) => Promise<Record<string, unknown>[]>
}

type SliceProviderLogin = {
  provider: string
  login_kind: string
  auth_url?: string | null
  verification_url?: string | null
  user_code?: string | null
  status: string
  message: string
}

export async function handleSliceSlashCommand(
  deps: SliceCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "slice" }>,
): Promise<void> {
  const [subcommand, ...args] = command.args
  if (!subcommand || subcommand === "list" || subcommand === "ls") {
    await listSlices(deps)
    return
  }
  if (subcommand === "create") {
    await createSlice(deps, args)
    return
  }
  if (subcommand === "status" || subcommand === "show") {
    await showSlice(deps, args[0])
    return
  }
  if (subcommand === "doctor") {
    await doctorSlice(deps, args[0])
    return
  }
  if (subcommand === "logs" || subcommand === "log") {
    await showSliceLogs(deps, args)
    return
  }
  if (subcommand === "audit") {
    await showSliceAudit(deps, args)
    return
  }
  if (subcommand === "start" || subcommand === "stop") {
    await setSliceRunning(deps, subcommand, args[0])
    return
  }
  if (subcommand === "delete" || subcommand === "rm") {
    await deleteSlice(deps, args[0])
    return
  }
  if (subcommand === "screen") {
    await openSliceScreen(deps, args[0])
    return
  }
  if (subcommand === "auth" && args[0] === "import") {
    await importSliceAuth(deps, args)
    return
  }
  if (subcommand === "auth" && args[0] === "remove") {
    await removeSliceAuth(deps, args)
    return
  }
  if (subcommand === "auth" && args[0] === "login") {
    await startSliceAuthLogin(deps, args)
    return
  }
  if (subcommand === "auth" && args[0] === "alias") {
    await setSliceAuthAlias(deps, args)
    return
  }
  deps.flashFooter("usage: /slice list | /slice create <name> [--headed|--headless] | /slice status [slice-ref] | /slice doctor [slice-ref] | /slice logs [slice-ref] [--tail <lines>] | /slice audit [slice-ref] [--limit <count>] | /slice start [slice-ref] | /slice stop [slice-ref] | /slice delete <slice-ref> | /slice screen [slice-ref] | /slice auth import [slice-ref] <provider> | /slice auth remove [slice-ref] <provider> | /slice auth login [slice-ref] <provider> | /slice auth alias [slice-ref] <provider> <alias|clear>", "error")
}

function formatSliceLabel(slice: SliceRecord): string {
  return slice.name || slice.id
}

function formatSlice(slice: SliceRecord): string {
  const display = slice.display_endpoint?.url ? ` screen=${slice.display_endpoint.url}` : ""
  const providers = (slice.providers ?? []).join(",") || "-"
  const auth = (slice.provider_auth ?? []).map((entry) => formatSliceProviderAuth(entry)).join(",") || "-"
  const authStatus = formatSliceProviderAuthReadiness(slice)
  const worker = slice.worker_kernel_id ?? slice.worker_kernel_ref
  const worktree = formatSliceScope(slice)
  const relay = formatSliceRelayLabel(slice, { includeUrl: true, emptyLabel: "none" })
  const diagnostics = formatSliceDiagnostics(slice)
  const next = sliceProviderAuthNextAction(slice)
  return `${formatSliceLabel(slice)} id=${slice.id} status=${slice.status} display=${slice.display_mode ?? "headless"} backend=${slice.backend} os=${slice.os} worktree=${worktree} agents=${slice.agent_ids?.length ?? 0} sessions=${slice.session_ids?.length ?? 0} worker=${worker} relay=${relay} providers=${providers} auth_status=${authStatus} auth=${auth}${diagnostics}${next ? ` next=${next}` : ""}${display}`
}

function formatSliceLogs(slice: SliceRecord, entries: SliceLogEntry[]): string {
  if (!entries.length) {
    return `slice logs ${formatSliceLabel(slice)} (${slice.id})\nno logs found`
  }
  return [
    `slice logs ${formatSliceLabel(slice)} (${slice.id})`,
    ...entries.map((entry) => {
      const path = entry.path ? ` path=${entry.path}` : ""
      const truncated = entry.truncated ? " truncated" : ""
      const text = entry.text.trim() || "<empty>"
      return `== ${entry.source}${path}${truncated} ==\n${text}`
    }),
  ].join("\n")
}

function formatSliceAuditEvents(events: readonly Record<string, unknown>[]): string {
  if (!events.length) {
    return "no slice audit events"
  }
  return events.map((event) => {
    const payload = typeof event.payload === "object" && event.payload ? event.payload as Record<string, unknown> : {}
    const at = typeof event.timestamp_ms === "number" ? new Date(event.timestamp_ms).toISOString() : "unknown-time"
    const action = typeof payload.action === "string" ? payload.action : String(event.kind ?? "slice.audit")
    const outcome = typeof payload.outcome === "string" ? payload.outcome : typeof payload.result === "string" ? payload.result : ""
    const label = typeof payload.slice_name === "string" && payload.slice_name ? payload.slice_name : payload.slice_id
    const provider = typeof payload.provider === "string" && payload.provider ? ` provider=${payload.provider}` : ""
    const message = typeof payload.message === "string" && payload.message ? ` message=${payload.message}` : ""
    const details = [
      auditField("status", payload.status),
      auditField("display", payload.display_mode),
      auditField("worktree", payload.worktree_id ?? payload.workspace_mount),
      auditField("agents", Array.isArray(payload.agent_ids) ? payload.agent_ids.length : undefined),
      auditField("worker", payload.worker_kernel_id ?? payload.worker_kernel_ref),
    ].filter(Boolean)
    const detailLine = details.length > 0 ? `\n  ${details.join(" ")}` : ""
    return `${at} ${action}${outcome ? ` ${outcome}` : ""} slice=${String(label ?? "-")}${provider}${message}${detailLine}`
  }).join("\n")
}

function auditField(label: string, value: unknown): string | null {
  if (value === null || value === undefined || value === "") {
    return null
  }
  return `${label}=${String(value)}`
}

function parseSliceLogsArgs(args: string[]): {
  sliceRef?: string
  tailLines?: number
  error?: string
} {
  let sliceRef: string | undefined
  let tailLines: number | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const value = args[index + 1]
    if ((arg === "--tail" || arg === "-n") && value && !value.startsWith("--")) {
      const parsed = Number.parseInt(value, 10)
      if (!Number.isFinite(parsed) || parsed <= 0) {
        return { error: "usage: /slice logs [slice-ref] [--tail <lines>]" }
      }
      tailLines = parsed
      index += 1
      continue
    }
    if (!sliceRef && arg && !arg.startsWith("--")) {
      sliceRef = arg
      continue
    }
    return { error: "usage: /slice logs [slice-ref] [--tail <lines>]" }
  }
  return { ...(sliceRef ? { sliceRef } : {}), ...(tailLines ? { tailLines } : {}) }
}

function parseSliceAuditArgs(args: string[]): {
  sliceRef?: string
  limit?: number
  error?: string
} {
  let sliceRef: string | undefined
  let limit: number | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const value = args[index + 1]
    if (arg === "--limit" && value && !value.startsWith("--")) {
      const parsed = Number.parseInt(value, 10)
      if (!Number.isFinite(parsed) || parsed <= 0) {
        return { error: "usage: /slice audit [slice-ref] [--limit <count>]" }
      }
      limit = parsed
      index += 1
      continue
    }
    if (!sliceRef && arg && !arg.startsWith("--")) {
      sliceRef = arg
      continue
    }
    return { error: "usage: /slice audit [slice-ref] [--limit <count>]" }
  }
  return { ...(sliceRef ? { sliceRef } : {}), ...(limit ? { limit } : {}) }
}

async function resolveFocusedSliceRef(deps: SliceCommandHandlerDeps): Promise<string> {
  const focusedId = deps.focusedAgentId()
  const resolved = deps.resolveSessionAgent(focusedId)
  const agent = resolved.agent
  const remote = agent?.remote_execution
  if (!remote) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  if (!deps.listSlices) {
    throw new Error("slice inventory is unavailable in this build")
  }
  const slices = await deps.listSlices()
  const match = slices.find((slice) => slice.agent_ids?.includes(agent.id))
    ?? slices.find((slice) =>
      slice.worker_kernel_id === remote.worker_kernel_id
      || slice.worker_kernel_ref === remote.worker_kernel_id
      || slice.worker_machine_id === remote.worker_machine_id,
    )
  if (!match) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  return match.name || match.id
}

async function explicitOrFocusedSliceRef(
  deps: SliceCommandHandlerDeps,
  value: string | undefined,
): Promise<string> {
  return value ?? resolveFocusedSliceRef(deps)
}

async function showSliceLogs(deps: SliceCommandHandlerDeps, args: string[]): Promise<void> {
  if (!deps.getSliceLogs) {
    deps.flashFooter("slice logs are unavailable in this build", "error")
    return
  }
  const parsed = parseSliceLogsArgs(args)
  if (parsed.error) {
    deps.flashFooter(parsed.error, "error")
    return
  }
  try {
    const sliceRef = await explicitOrFocusedSliceRef(deps, parsed.sliceRef)
    const payload = await deps.getSliceLogs(sliceRef, parsed.tailLines ?? null)
    deps.appendNotice(formatSliceLogs(payload.slice, payload.entries))
    deps.flashFooter(`slice logs ${formatSliceLabel(payload.slice)}`, "info")
  } catch (error) {
    deps.flashFooter(error instanceof Error ? error.message : "slice logs failed", "error")
  }
}

async function showSliceAudit(deps: SliceCommandHandlerDeps, args: string[]): Promise<void> {
  if (!deps.listSliceAudit) {
    deps.flashFooter("slice audit is unavailable in this build", "error")
    return
  }
  const parsed = parseSliceAuditArgs(args)
  if (parsed.error) {
    deps.flashFooter(parsed.error, "error")
    return
  }
  try {
    const sliceRef = await explicitOrFocusedSliceRef(deps, parsed.sliceRef)
    const events = await deps.listSliceAudit(sliceRef, parsed.limit ?? null)
    deps.appendNotice(formatSliceAuditEvents(events))
    deps.flashFooter(`slice audit ${sliceRef}`, "info")
  } catch (error) {
    deps.flashFooter(error instanceof Error ? error.message : "slice audit failed", "error")
  }
}

function parseSliceCreateOptions(
  deps: SliceCommandHandlerDeps,
  args: string[],
): {
  name?: string
  backend?: "local_docker" | "ssh_docker"
  workerKernelRef?: string | null
  displayUrl?: string | null
  workspaceMount?: string | null
  workspaceId?: string | null
  worktreeId?: string | null
  displayMode?: "headless" | "headed"
  error?: string
} {
  const name = args[0]
  let backend: "local_docker" | "ssh_docker" | undefined
  let workerKernelRef: string | null | undefined
  let displayUrl: string | null | undefined
  let workspaceId: string | null | undefined = deps.currentWorkspaceTarget()
  let worktreeId: string | null | undefined = deps.currentWorktreeTarget()
  let workspaceMount: string | null | undefined = deps.currentWorktreeTarget()
  let displayMode: "headless" | "headed" | undefined
  let error: string | undefined
  for (let index = 1; index < args.length; index += 1) {
    const arg = args[index]
    const value = args[index + 1]
    if (arg === "--backend") {
      if (value !== "local_docker" && value !== "ssh_docker") {
        error = "usage: /slice create <name> [--headed|--headless] [--backend local_docker|ssh_docker] [--kernel <worker-kernel-ref>] [--display-url <url>] [--mount <path|none>]"
        break
      }
      backend = value
      index += 1
      continue
    }
    if (arg === "--headed" || arg === "--display") {
      displayMode = "headed"
      continue
    }
    if (arg === "--headless" || arg === "--no-display") {
      displayMode = "headless"
      continue
    }
    if (arg === "--kernel") {
      if (!value || value.startsWith("--")) {
        error = "usage: /slice create <name> --kernel <worker-kernel-ref>"
        break
      }
      workerKernelRef = value
      index += 1
      continue
    }
    if (arg === "--display-url") {
      if (!value || value.startsWith("--")) {
        error = "usage: /slice create <name> --display-url <url>"
        break
      }
      displayUrl = value
      index += 1
      continue
    }
    if (arg === "--mount") {
      if (!value || value.startsWith("--")) {
        error = "usage: /slice create <name> --mount <path|none>"
        break
      }
      workspaceMount = value === "none" ? null : value
      worktreeId = value === "none" ? null : value
      index += 1
      continue
    }
    error = `unknown /slice create option ${arg}`
    break
  }
  return {
    ...(name !== undefined ? { name } : {}),
    ...(backend !== undefined ? { backend } : {}),
    ...(workerKernelRef !== undefined ? { workerKernelRef } : {}),
    ...(displayUrl !== undefined ? { displayUrl } : {}),
    ...(workspaceId !== undefined ? { workspaceId } : {}),
    ...(worktreeId !== undefined ? { worktreeId } : {}),
    ...(workspaceMount !== undefined ? { workspaceMount } : {}),
    ...(displayMode !== undefined ? { displayMode } : {}),
    ...(error !== undefined ? { error } : {}),
  }
}

async function listSlices(deps: SliceCommandHandlerDeps): Promise<void> {
  if (!deps.listSlices) {
    deps.flashFooter("slice inventory is unavailable in this build", "error")
    return
  }
  const slices = await deps.listSlices()
  deps.appendNotice(slices.length === 0 ? "No slices owned by this kernel." : slices.map(formatSlice).join("\n"))
  deps.flashFooter(`listed ${slices.length} slice${slices.length === 1 ? "" : "s"}`, "info")
}

async function createSlice(
  deps: SliceCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.createSlice) {
    deps.flashFooter("slice creation is unavailable in this build", "error")
    return
  }
  const parsed = parseSliceCreateOptions(deps, args)
  if (!parsed.name || parsed.error) {
    deps.flashFooter(parsed.error ?? "usage: /slice create <name> [--headed|--headless] [--kernel <worker-kernel-ref>] [--display-url <url>] [--mount <path|none>]", "error")
    return
  }
  const createOptions = {
    name: parsed.name,
    ...(parsed.backend !== undefined ? { backend: parsed.backend } : {}),
    ...(parsed.displayMode !== undefined ? { displayMode: parsed.displayMode } : {}),
    ...(parsed.workspaceId !== undefined ? { workspaceId: parsed.workspaceId } : {}),
    ...(parsed.worktreeId !== undefined ? { worktreeId: parsed.worktreeId } : {}),
    ...(parsed.workspaceMount !== undefined ? { workspaceMount: parsed.workspaceMount } : {}),
    workerKernelRef: parsed.workerKernelRef ?? null,
    displayUrl: parsed.displayUrl ?? null,
  }
  const slice = await deps.createSlice(createOptions)
  deps.flashFooter(`created slice ${formatSliceLabel(slice)}`, "info")
}

async function showSlice(
  deps: SliceCommandHandlerDeps,
  sliceRef: string | undefined,
): Promise<void> {
  if (!deps.getSlice) {
    deps.flashFooter("slice inventory is unavailable in this build", "error")
    return
  }
  const slice = await deps.getSlice(await explicitOrFocusedSliceRef(deps, sliceRef))
  deps.appendNotice(formatSlice(slice))
  deps.flashFooter(`showing slice ${formatSliceLabel(slice)}`, "info")
}

async function doctorSlice(
  deps: SliceCommandHandlerDeps,
  sliceRef: string | undefined,
): Promise<void> {
  if (!deps.getSlice) {
    deps.flashFooter("slice doctor is unavailable in this build", "error")
    return
  }
  const slice = await deps.getSlice(await explicitOrFocusedSliceRef(deps, sliceRef))
  deps.appendNotice(formatSliceDoctor(slice))
  deps.flashFooter(
    `slice doctor ${formatSliceLabel(slice)}: ${slice.status}`,
    sliceDoctorHasFailures(slice) ? "error" : "info",
  )
}

function sliceDoctorHasFailures(slice: SliceRecord): boolean {
  const scope = formatSliceScope(slice)
  return slice.status === "unhealthy"
    || (slice.status === "running" && !slice.worker_kernel_id)
    || (slice.status === "running" && !slice.relay_endpoint?.url)
    || scope === "-"
    || (slice.display_mode === "headed" && !slice.display_endpoint?.url)
    || slice.last_operation_status === "failed"
    || !sliceProviderAuthHealthy(slice)
}

function formatSliceDoctor(slice: SliceRecord): string {
  const scope = formatSliceScope(slice)
  const relay = formatSliceRelayLabel(slice, { includeUrl: true, emptyLabel: "none" })
  const providers = slice.providers ?? []
  const providerAuth = slice.provider_auth ?? []
  const checks = [
    doctorCheck("lifecycle", slice.status !== "unhealthy", slice.status),
    doctorCheck("worker", slice.status !== "running" || Boolean(slice.worker_kernel_id), slice.worker_kernel_id ?? "not discovered"),
    doctorCheck("relay", slice.status !== "running" || Boolean(slice.relay_endpoint?.url), relay),
    doctorCheck("scope", scope !== "-", scope === "-" ? "missing" : scope),
    doctorCheck("agents", true, `${slice.agent_ids?.length ?? 0} attached`),
    doctorCheck("sessions", true, `${slice.session_ids?.length ?? 0} attached`),
    doctorCheck("display", slice.display_mode !== "headed" || Boolean(slice.display_endpoint?.url), slice.display_endpoint?.url ?? slice.display_mode ?? "headless"),
    doctorCheck("last operation", slice.last_operation_status !== "failed", formatSliceOperation(slice) || "none"),
    doctorCheck("provider CLIs", providers.length > 0, providers.join(",") || "none"),
    doctorCheck("provider accounts", sliceProviderAuthHealthy(slice), providerAuth.map((entry) => formatSliceProviderAuth(entry)).join(",") || "none"),
  ]
  return [`slice doctor ${formatSliceLabel(slice)} (${slice.id})`, ...checks, ...sliceDoctorNextActions(slice)].join("\n")
}

function doctorCheck(name: string, ok: boolean, detail: string): string {
  return `${ok ? "ok" : "fail"} ${name}: ${detail}`
}

function sliceDoctorNextActions(slice: SliceRecord): string[] {
  const scope = formatSliceScope(slice)
  const actions = [
    slice.status === "unhealthy" ? "inspect slice logs and /slice audit, then try /slice start or recreate the slice" : null,
    slice.status === "running" && !slice.worker_kernel_id ? "wait for worker discovery; restart the slice if no worker appears" : null,
    slice.status === "running" && !slice.relay_endpoint?.url ? "check relay connectivity and restart the slice if the relay endpoint stays missing" : null,
    scope === "-" ? "create or reuse a slice scoped to the selected worktree before spawning agents into it" : null,
    slice.display_mode === "headed" && !slice.display_endpoint?.url ? "start the slice screen service or recreate as headless if a display is not needed" : null,
    slice.last_operation_status === "failed" ? "inspect slice logs and /slice audit, then retry the failed operation after fixing the reported error" : null,
    sliceProviderAuthNextAction(slice),
  ].filter((action): action is string => Boolean(action))
  return actions.length > 0 ? [`next: ${actions.join("; ")}`] : []
}

function sliceProviderAuthNextAction(slice: SliceRecord): string {
  if ((slice.providers ?? []).length === 0) {
    return "configure provider CLIs inside the slice before spawning agents there"
  }
  const coverage = sliceProviderAuthCoverage(slice)
  const actions = [
    coverage.missingProviders.length > 0
      ? `import or login provider accounts${formatSliceProviderActionTarget(coverage.missingProviders)} with ${formatSliceAuthRecoveryCommands(slice, coverage.missingProviders)}`
      : null,
    coverage.staleProviders.length > 0
      ? `refresh provider login${formatSliceProviderActionTarget(coverage.staleProviders)} with ${formatSliceAuthLoginRecoveryCommands(slice, coverage.staleProviders)}`
      : null,
  ].filter((action): action is string => Boolean(action))
  if (actions.length > 0) {
    return actions.join("; ")
  }
  return ""
}

function sliceProviderAuthHealthy(slice: SliceRecord): boolean {
  if ((slice.providers ?? []).length === 0) {
    return false
  }
  return sliceProviderAuthCoverage(slice).hasHealthyCoverage
}

function formatSliceProviderActionTarget(providers: readonly string[]): string {
  const list = formatSliceProviderList(providers)
  return list ? ` for ${list.replaceAll(", ", ",")}` : ""
}

function sliceProviderNames(providers: readonly string[]): string[] {
  return [...new Set(providers.map((provider) => provider.trim()).filter(Boolean))]
}

function formatSliceAuthRecoveryCommands(slice: SliceRecord, providers: readonly string[]): string {
  return formatProviderRecoveryCommands(providers, (provider) =>
    `/slice auth import ${formatSliceLabel(slice)} ${provider} or /slice auth login ${formatSliceLabel(slice)} ${provider}`)
}

function formatSliceAuthLoginRecoveryCommands(slice: SliceRecord, providers: readonly string[]): string {
  return formatProviderRecoveryCommands(providers, (provider) => `/slice auth login ${formatSliceLabel(slice)} ${provider}`)
}

function formatProviderRecoveryCommands(providers: readonly string[], commandForProvider: (provider: string) => string): string {
  const unique = sliceProviderNames(providers)
  if (unique.length === 0) {
    return "the provider-specific command shown by /slice doctor"
  }
  return unique
    .map((provider, index) => index === 0 ? commandForProvider(provider) : `for ${provider} use ${commandForProvider(provider)}`)
    .join("; ")
}

async function setSliceRunning(
  deps: SliceCommandHandlerDeps,
  action: "start" | "stop",
  sliceRef: string | undefined,
): Promise<void> {
  const handler = action === "start" ? deps.startSlice : deps.stopSlice
  if (!handler) {
    deps.flashFooter(`slice ${action} is unavailable in this build`, "error")
    return
  }
  const resolvedRef = await explicitOrFocusedSliceRef(deps, sliceRef)
  if (action === "stop" && await rejectSliceLifecycleWithAttachedAgents(deps, action, resolvedRef)) {
    return
  }
  const slice = await handler(resolvedRef)
  deps.flashFooter(`${action === "start" ? "started" : "stopped"} slice ${formatSliceLabel(slice)}`, "info")
}

async function deleteSlice(
  deps: SliceCommandHandlerDeps,
  sliceRef: string | undefined,
): Promise<void> {
  if (!deps.deleteSlice) {
    deps.flashFooter("slice delete is unavailable in this build", "error")
    return
  }
  if (!sliceRef) {
    deps.flashFooter("usage: /slice delete <slice-ref>", "error")
    return
  }
  if (await rejectSliceLifecycleWithAttachedAgents(deps, "delete", sliceRef)) {
    return
  }
  const slice = await deps.deleteSlice(sliceRef)
  deps.flashFooter(`deleted slice ${formatSliceLabel(slice)}`, "info")
}

async function rejectSliceLifecycleWithAttachedAgents(
  deps: SliceCommandHandlerDeps,
  action: "stop" | "delete",
  sliceRef: string,
): Promise<boolean> {
  const slice = await loadSliceForLifecycleGuard(deps, sliceRef)
  const attachedAgents = attachedAgentRefs(slice)
  if (attachedAgents.length === 0) {
    return false
  }
  const label = slice ? formatSliceLabel(slice) : sliceRef
  deps.flashFooter(
    `cannot ${action} slice ${label}; move or end attached agents first: ${attachedAgents.join(",")}`,
    "error",
  )
  return true
}

async function loadSliceForLifecycleGuard(
  deps: SliceCommandHandlerDeps,
  sliceRef: string,
): Promise<SliceRecord | null> {
  if (deps.getSlice) {
    return deps.getSlice(sliceRef)
  }
  if (!deps.listSlices) {
    return null
  }
  const slices = await deps.listSlices()
  return slices.find((slice) => slice.id === sliceRef || slice.name === sliceRef) ?? null
}

function attachedAgentRefs(slice: SliceRecord | null): string[] {
  return [...(slice?.agent_ids ?? [])].map((agent) => agent.trim()).filter(Boolean)
}

async function openSliceScreen(
  deps: SliceCommandHandlerDeps,
  sliceRef: string | undefined,
): Promise<void> {
  if (!deps.getSliceDisplayEndpoint) {
    deps.flashFooter("slice screen is unavailable in this build", "error")
    return
  }
  const endpoint = await deps.getSliceDisplayEndpoint(await explicitOrFocusedSliceRef(deps, sliceRef))
  deps.appendNotice(endpoint.url)
  const opened = await deps.openExternalUrl?.(endpoint.url)
  deps.flashFooter(`${opened ? "opened" : "screen"} ${endpoint.url}`, "info")
}

async function importSliceAuth(
  deps: SliceCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.importSliceProviderAuth) {
    deps.flashFooter("slice auth import is unavailable in this build", "error")
    return
  }
  const provider = args.length >= 3 ? args[2] : args[1]
  const sliceRef = args.length >= 3 ? args[1]! : await resolveFocusedSliceRef(deps)
  if (!provider) {
    deps.flashFooter("usage: /slice auth import [slice-ref] <provider>", "error")
    return
  }
  const result = await deps.importSliceProviderAuth(sliceRef, provider)
  deps.flashFooter(
    formatSliceProviderAuthActionResult("import", result.slice, result.provider, result.status),
    result.status === "imported" ? "info" : "error",
  )
}

async function removeSliceAuth(
  deps: SliceCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.removeSliceProviderAuth) {
    deps.flashFooter("slice auth remove is unavailable in this build", "error")
    return
  }
  const provider = args.length >= 3 ? args[2] : args[1]
  const sliceRef = args.length >= 3 ? args[1]! : await resolveFocusedSliceRef(deps)
  if (!provider) {
    deps.flashFooter("usage: /slice auth remove [slice-ref] <provider>", "error")
    return
  }
  const result = await deps.removeSliceProviderAuth(sliceRef, provider)
  deps.flashFooter(
    formatSliceProviderAuthActionResult("remove", result.slice, result.provider, result.status),
    result.status === "removed" ? "info" : "error",
  )
}

async function startSliceAuthLogin(
  deps: SliceCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.startSliceProviderLogin) {
    deps.flashFooter("slice auth login is unavailable in this build", "error")
    return
  }
  const provider = args.length >= 3 ? args[2] : args[1]
  const sliceRef = args.length >= 3 ? args[1]! : await resolveFocusedSliceRef(deps)
  if (!provider) {
    deps.flashFooter("usage: /slice auth login [slice-ref] <provider>", "error")
    return
  }
  const result = await deps.startSliceProviderLogin(sliceRef, provider)
  deps.appendNotice(formatSliceProviderLogin(result.login))
  deps.flashFooter(`slice auth login ${result.login.provider}: ${result.login.status}`, "info")
}

function formatSliceProviderLogin(login: SliceProviderLogin): string {
  return [
    `slice auth login ${login.provider}: ${login.status}`,
    login.verification_url ? `url=${login.verification_url}` : "",
    login.user_code ? `code=${login.user_code}` : "",
    login.message,
  ].filter(Boolean).join("\n")
}

function formatSliceProviderAuthActionResult(
  action: "import" | "remove",
  slice: SliceRecord,
  provider: string,
  status: string,
): string {
  const sliceRef = formatSliceLabel(slice)
  if ((action === "import" && status === "imported") || (action === "remove" && status === "removed")) {
    return `slice auth ${action} ${provider}: ${status}`
  }
  if (status === "not_implemented") {
    const fallback = action === "import"
      ? `use /slice auth login ${sliceRef} ${provider}, open /slice screen ${sliceRef} to configure the account inside the slice, or update/restart the worker kernel if auth import should be available`
      : `open /slice screen ${sliceRef} to remove the provider account inside the slice, or update/restart the worker kernel if auth removal should be available`
    return `slice auth ${action} ${provider} is unavailable on this kernel. Next action: ${fallback}.`
  }
  return `slice auth ${action} ${provider} failed${status ? ` with status ${status}` : ""}. Next action: run /slice doctor ${sliceRef}, then retry or use /slice auth login ${sliceRef} ${provider}.`
}

async function setSliceAuthAlias(
  deps: SliceCommandHandlerDeps,
  args: string[],
): Promise<void> {
  if (!deps.setSliceProviderAuthAlias) {
    deps.flashFooter("slice auth alias is unavailable in this build", "error")
    return
  }
  const hasExplicitSlice = args.length >= 4
  const sliceRef = hasExplicitSlice ? args[1]! : await resolveFocusedSliceRef(deps)
  const provider = hasExplicitSlice ? args[2] : args[1]
  const aliasValue = hasExplicitSlice ? args.slice(3).join(" ") : args.slice(2).join(" ")
  if (!provider || !aliasValue) {
    deps.flashFooter("usage: /slice auth alias [slice-ref] <provider> <alias|clear>", "error")
    return
  }
  const alias = aliasValue === "clear" ? null : aliasValue
  const result = await deps.setSliceProviderAuthAlias(sliceRef, provider, alias)
  deps.flashFooter(
    result.alias
      ? `slice auth alias ${result.provider}: ${result.alias}`
      : `slice auth alias ${result.provider}: cleared`,
    "info",
  )
}
