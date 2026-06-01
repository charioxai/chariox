import type {
  SliceDisplayEndpoint,
  SliceLogEntry,
  SliceRecord,
} from "./kernel-types.js"
import {
  createSliceRequest,
  deleteSliceRequest,
  getSliceDisplayEndpointRequest,
  getSliceLogsRequest,
  getSliceRequest,
  importSliceProviderAuthRequest,
  listSlicesRequest,
  removeSliceProviderAuthRequest,
  setSliceProviderAuthAliasRequest,
  startSliceProviderLoginRequest,
  startSliceRequest,
  stopSliceRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellSliceCommandDeps = {
  client: ShellKernelClient
}

export async function executeSliceCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellSliceCommandDeps,
): Promise<ShellCommandResult> {
  const [action, first, ...rest] = parsed.args
  switch (action ?? "list") {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSlicesRequest())
      const slices = expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
      return { ok: true, message: formatSlices(slices), data: { slices } }
    }
    case "create": {
      if (!first) {
        return { ok: false, message: "usage: slice create <name> [--headed|--headless] [--kernel <worker-kernel-ref>] [--display-url <url>]" }
      }
      let workerKernelRef: string | undefined
      let displayUrl: string | undefined
      let displayMode: "headless" | "headed" | undefined
      for (let index = 0; index < rest.length; index += 1) {
        const arg = rest[index]
        const value = rest[index + 1]
        if (arg === "--kernel" && value && !value.startsWith("--")) {
          workerKernelRef = value
          index += 1
        } else if (arg === "--display-url" && value && !value.startsWith("--")) {
          displayUrl = value
          index += 1
        } else if (arg === "--headed" || arg === "--display") {
          displayMode = "headed"
        } else if (arg === "--headless" || arg === "--no-display") {
          displayMode = "headless"
        } else if (arg?.startsWith("--")) {
          return { ok: false, message: `unknown or incomplete slice create option: ${arg}` }
        } else if (arg) {
          return { ok: false, message: "usage: slice create <name> [--headed|--headless] [--kernel <worker-kernel-ref>] [--display-url <url>]" }
        }
      }
      const response = await deps.client.send(createSliceRequest({
        name: first,
        ...(displayMode ? { displayMode } : {}),
        workspaceId: context.workspace,
        worktreeId: context.worktree,
        workspaceMount: context.worktree,
        workerKernelRef: workerKernelRef ?? null,
        displayUrl: displayUrl ?? null,
      }))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceCreated").slice
      return resourceResult(`created slice ${formatSliceLabel(slice)}`, parsed.assignment, slice.id, {}, { slice })
    }
    case "status":
    case "show": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(getSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "Slice").slice
      return { ok: true, message: formatSlice(slice), data: { slice } }
    }
    case "doctor": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(getSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "Slice").slice
      return { ok: sliceDoctorHealthy(slice), message: formatSliceDoctor(slice), data: { slice } }
    }
    case "logs":
    case "log": {
      const { sliceRef, tailLines, error } = parseSliceLogsArgs(first, rest)
      if (error) {
        return { ok: false, message: error }
      }
      const resolvedSliceRef = sliceRef ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(getSliceLogsRequest(resolvedSliceRef, tailLines))
      const payload = expectVariant<{ slice: SliceRecord; entries: SliceLogEntry[] }>(response, "SliceLogs")
      return { ok: true, message: formatSliceLogs(payload.slice, payload.entries), data: payload }
    }
    case "start": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(startSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceStarted").slice
      return { ok: true, message: `started slice ${formatSliceLabel(slice)}`, data: { slice } }
    }
    case "stop": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(stopSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceStopped").slice
      return { ok: true, message: `stopped slice ${formatSliceLabel(slice)}`, data: { slice } }
    }
    case "delete":
    case "rm": {
      if (!first) {
        return { ok: false, message: "usage: slice delete <slice-ref>" }
      }
      const response = await deps.client.send(deleteSliceRequest(first))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceDeleted").slice
      return { ok: true, message: `deleted slice ${formatSliceLabel(slice)}`, data: { slice } }
    }
    case "screen": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(getSliceDisplayEndpointRequest(sliceRef))
      const endpoint = expectVariant<{ endpoint: SliceDisplayEndpoint }>(response, "SliceDisplayEndpoint").endpoint
      return { ok: true, message: endpoint.url, data: { endpoint } }
    }
    case "auth": {
      if (first === "import") {
        const sliceRef = rest.length > 1 ? rest[0]! : await focusedAgentSliceRef(context, deps)
        const provider = rest.length > 1 ? rest[1] : rest[0]
        if (!provider) {
          return { ok: false, message: "usage: slice auth import [slice-ref] <provider>" }
        }
        const response = await deps.client.send(importSliceProviderAuthRequest(sliceRef, provider))
        const payload = expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthImported")
        return { ok: true, message: `slice ${formatSliceLabel(payload.slice)} auth import ${payload.provider}: ${payload.status}`, data: payload }
      }
      if (first === "remove") {
        const sliceRef = rest.length > 1 ? rest[0]! : await focusedAgentSliceRef(context, deps)
        const provider = rest.length > 1 ? rest[1] : rest[0]
        if (!provider) {
          return { ok: false, message: "usage: slice auth remove [slice-ref] <provider>" }
        }
        const response = await deps.client.send(removeSliceProviderAuthRequest(sliceRef, provider))
        const payload = expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthRemoved")
        return { ok: true, message: `slice ${formatSliceLabel(payload.slice)} auth remove ${payload.provider}: ${payload.status}`, data: payload }
      }
      if (first === "login") {
        const sliceRef = rest.length > 1 ? rest[0]! : await focusedAgentSliceRef(context, deps)
        const provider = rest.length > 1 ? rest[1] : rest[0]
        if (!provider) {
          return { ok: false, message: "usage: slice auth login [slice-ref] <provider>" }
        }
        const response = await deps.client.send(startSliceProviderLoginRequest(sliceRef, provider))
        const payload = expectVariant<{
          slice: SliceRecord
          login: {
            provider: string
            login_kind: string
            verification_url?: string | null
            user_code?: string | null
            status: string
            message: string
          }
        }>(response, "SliceProviderLoginStarted")
        return {
          ok: true,
          message: formatSliceLoginMessage(payload.slice, payload.login),
          data: payload,
        }
      }
      if (first === "alias") {
        const hasExplicitSlice = rest.length >= 3
        const sliceRef = hasExplicitSlice ? rest[0]! : await focusedAgentSliceRef(context, deps)
        const provider = hasExplicitSlice ? rest[1] : rest[0]
        const aliasValue = hasExplicitSlice ? rest.slice(2).join(" ") : rest.slice(1).join(" ")
        if (!provider || !aliasValue) {
          return { ok: false, message: "usage: slice auth alias [slice-ref] <provider> <alias|clear>" }
        }
        const alias = aliasValue === "clear" ? null : aliasValue
        const response = await deps.client.send(setSliceProviderAuthAliasRequest(sliceRef, provider, alias))
        const payload = expectVariant<{ slice: SliceRecord; provider: string; alias: string | null }>(response, "SliceProviderAuthAliasSet")
        return {
          ok: true,
          message: payload.alias
            ? `slice ${formatSliceLabel(payload.slice)} auth alias ${payload.provider}: ${payload.alias}`
            : `slice ${formatSliceLabel(payload.slice)} auth alias ${payload.provider}: cleared`,
          data: payload,
        }
      }
      return { ok: false, message: "usage: slice auth import|remove [slice-ref] <provider> | slice auth login [slice-ref] <provider> | slice auth alias [slice-ref] <provider> <alias|clear>" }
    }
    default:
      return { ok: false, message: "usage: slice list|create|status|doctor|logs|start|stop|delete|auth import|auth remove|auth login|auth alias|screen" }
  }
}

function parseSliceLogsArgs(first: string | undefined, rest: string[]): {
  sliceRef?: string
  tailLines?: number
  error?: string
} {
  const args = [first, ...rest].filter((arg): arg is string => Boolean(arg))
  let sliceRef: string | undefined
  let tailLines: number | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const value = args[index + 1]
    if ((arg === "--tail" || arg === "-n") && value && !value.startsWith("--")) {
      const parsed = Number.parseInt(value, 10)
      if (!Number.isFinite(parsed) || parsed <= 0) {
        return { error: "usage: slice logs [slice-ref] [--tail <lines>]" }
      }
      tailLines = parsed
      index += 1
      continue
    }
    if (!sliceRef && arg && !arg.startsWith("--")) {
      sliceRef = arg
      continue
    }
    return { error: "usage: slice logs [slice-ref] [--tail <lines>]" }
  }
  return { ...(sliceRef ? { sliceRef } : {}), ...(tailLines ? { tailLines } : {}) }
}

function formatSliceLoginMessage(
  slice: SliceRecord,
  login: {
    provider: string
    verification_url?: string | null
    user_code?: string | null
    status: string
    message: string
  },
): string {
  return [
    `slice ${formatSliceLabel(slice)} auth login ${login.provider}: ${login.status}`,
    login.verification_url ? `url=${login.verification_url}` : "",
    login.user_code ? `code=${login.user_code}` : "",
    login.message,
  ].filter(Boolean).join("\n")
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

function formatSlices(slices: SliceRecord[]): string {
  if (slices.length === 0) {
    return "no slices"
  }
  return slices.map(formatSlice).join("\n")
}

function formatSlice(slice: SliceRecord): string {
  const providers = (slice.providers ?? []).join(",") || "-"
  const display = slice.display_endpoint?.url ? ` display=${slice.display_endpoint.url}` : ""
  const relay = formatSliceRelay(slice) || "none"
  const auth = (slice.provider_auth ?? [])
    .map(formatSliceProviderAuth)
    .join(",") || "-"
  const agents = slice.agent_ids?.length ?? 0
  const scope = slice.worktree_id ? ` worktree=${slice.worktree_id}` : ""
  const diagnostics = formatSliceDiagnostics(slice)
  const next = sliceNextAction(slice)
  return `${formatSliceLabel(slice)} status=${slice.status} backend=${slice.backend} os=${slice.os} display_mode=${slice.display_mode ?? "headless"} worker=${slice.worker_kernel_id ?? slice.worker_kernel_ref} relay=${relay} agents=${agents}${scope} providers=${providers} auth=${auth}${diagnostics}${display}${next ? ` next=${next}` : ""}`
}

function formatSliceDoctor(slice: SliceRecord): string {
  const scope = slice.worktree_id || slice.workspace_mount || slice.workspace_id || "missing"
  const relay = formatSliceRelay(slice) || "none"
  const providers = slice.providers ?? []
  const providerAuth = slice.provider_auth ?? []
  const providerAuthHealthy = providers.length > 0
    && providerAuth.length > 0
    && providerAuth.every((entry) => entry.state !== "unknown" && entry.state !== "not_configured")
  const checks = [
    doctorCheck("lifecycle", slice.status !== "unhealthy", slice.status),
    doctorCheck("worker", slice.status !== "running" || Boolean(slice.worker_kernel_id), slice.worker_kernel_id ?? "not discovered"),
    doctorCheck("relay", slice.status !== "running" || Boolean(slice.relay_endpoint?.url), relay),
    doctorCheck("scope", scope !== "missing", scope),
    doctorCheck("agents", true, `${slice.agent_ids?.length ?? 0} attached`),
    doctorCheck("sessions", true, `${slice.session_ids?.length ?? 0} attached`),
    doctorCheck("display", slice.display_mode !== "headed" || Boolean(slice.display_endpoint?.url), slice.display_endpoint?.url ?? slice.display_mode ?? "headless"),
    doctorCheck("last operation", slice.last_operation_status !== "failed", formatSliceOperation(slice) || "none"),
    doctorCheck("provider CLIs", providers.length > 0, providers.join(",") || "none"),
    doctorCheck("provider accounts", providerAuthHealthy, providerAuth.map(formatSliceProviderAuth).join(",") || "none"),
  ]
  return [`slice doctor ${formatSliceLabel(slice)}`, ...checks, ...sliceDoctorNextActions(slice)].join("\n")
}

function sliceDoctorHealthy(slice: SliceRecord): boolean {
  const scope = slice.worktree_id || slice.workspace_mount || slice.workspace_id || ""
  const providers = slice.providers ?? []
  const providerAuth = slice.provider_auth ?? []
  return slice.status !== "unhealthy"
    && (slice.status !== "running" || Boolean(slice.worker_kernel_id))
    && (slice.status !== "running" || Boolean(slice.relay_endpoint?.url))
    && Boolean(scope)
    && (slice.display_mode !== "headed" || Boolean(slice.display_endpoint?.url))
    && slice.last_operation_status !== "failed"
    && providers.length > 0
    && providerAuth.length > 0
    && providerAuth.every((entry) => entry.state !== "unknown" && entry.state !== "not_configured")
}

function sliceDoctorNextActions(slice: SliceRecord): string[] {
  const scope = slice.worktree_id || slice.workspace_mount || slice.workspace_id || "missing"
  const next = sliceNextAction(slice)
  const actions = [
    slice.status === "unhealthy" ? "inspect slice logs, then try slice start or recreate the slice" : null,
    slice.status === "running" && !slice.worker_kernel_id ? "wait for worker discovery; restart the slice if no worker appears" : null,
    slice.status === "running" && !slice.relay_endpoint?.url ? "check relay connectivity and restart the slice if the relay endpoint stays missing" : null,
    scope === "missing" ? "create or reuse a slice scoped to the selected worktree before spawning agents into it" : null,
    slice.display_mode === "headed" && !slice.display_endpoint?.url ? "start the slice screen service or recreate as headless if a display is not needed" : null,
    slice.last_operation_status === "failed" ? "inspect slice logs and retry the failed operation after fixing the reported error" : null,
    next,
  ].filter((action): action is string => Boolean(action))
  const unique = [...new Set(actions)]
  return unique.length > 0 ? [`next: ${unique.join("; ")}`] : []
}

function sliceNextAction(slice: SliceRecord): string | null {
  if (slice.status === "unhealthy") {
    return "inspect logs, then start or delete/recreate the slice"
  }
  if (slice.status === "running" && !slice.relay_endpoint?.url) {
    return "check relay connectivity or restart the slice"
  }
  if (slice.display_mode === "headed" && !slice.display_endpoint?.url) {
    return "start the slice screen service or recreate as headless if a display is not needed"
  }
  if (slice.last_operation_status === "failed") {
    return "open logs and retry the failed operation after fixing the error"
  }
  if ((slice.providers ?? []).length === 0) {
    return "configure provider CLIs inside the slice before spawning agents there"
  }
  const auth = slice.provider_auth ?? []
  if (auth.length === 0) {
    return "import or login provider accounts for this slice"
  }
  if (auth.some((entry) => entry.state === "unknown" || entry.state === "not_configured")) {
    return "refresh provider login for this slice"
  }
  return null
}

function formatSliceProviderAuth(entry: NonNullable<SliceRecord["provider_auth"]>[number]): string {
  const identity = entry.email || entry.account_id || entry.auth_type || entry.state
  const label = entry.alias && entry.alias !== identity
    ? `${entry.alias} (${identity})`
    : identity
  const org = entry.organization_name || entry.organization_id
  return [
    `${entry.provider}:${label}`,
    org ? `org=${org}` : "",
    entry.subscription_type ? `plan=${entry.subscription_type}` : "",
  ].filter(Boolean).join("/")
}

function formatSliceRelay(slice: SliceRecord): string {
  const endpoint = slice.relay_endpoint
  if (!endpoint?.url) {
    return ""
  }
  return `${endpoint.private ? "private" : "shared"}:${endpoint.url}`
}

function formatSliceLogs(slice: SliceRecord, entries: SliceLogEntry[]): string {
  if (!entries.length) {
    return `slice logs ${formatSliceLabel(slice)}\nno logs found`
  }
  return [
    `slice logs ${formatSliceLabel(slice)}`,
    ...entries.map((entry) => {
      const location = entry.path ? ` path=${entry.path}` : ""
      const truncated = entry.truncated ? " truncated" : ""
      const body = entry.text.trim() || "<empty>"
      return `== ${entry.source}${location}${truncated} ==\n${body}`
    }),
  ].join("\n")
}

function doctorCheck(name: string, ok: boolean, detail: string): string {
  return `${ok ? "ok" : "fail"} ${name}: ${detail}`
}

function formatSliceLabel(slice: SliceRecord): string {
  return slice.name ? `${slice.name} id=${slice.id}` : slice.id
}

function formatSliceOperation(slice: SliceRecord): string {
  if (!slice.last_operation && !slice.last_operation_status && !slice.last_error) {
    return ""
  }
  const status = slice.last_operation_status ? `:${slice.last_operation_status}` : ""
  const error = slice.last_error ? ` error=${slice.last_error}` : ""
  return `${slice.last_operation ?? "operation"}${status}${error}`
}

function formatSliceDiagnostics(slice: SliceRecord): string {
  const operation = formatSliceOperation(slice)
  return operation ? ` last_operation=${operation}` : ""
}

async function focusedAgentSliceRef(context: ShellContext, deps: ShellSliceCommandDeps): Promise<string> {
  const resolved = await resolveShellAgent(context, deps, undefined)
  if (!resolved.ok) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  const remote = resolved.agent.remote_execution
  if (!remote) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  const response = await deps.client.send(listSlicesRequest())
  const slices = expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
  const match = slices.find((slice) => slice.agent_ids?.includes(resolved.agent.id))
    ?? slices.find((slice) =>
      slice.worker_kernel_id === remote.worker_kernel_id
      || slice.worker_kernel_ref === remote.worker_kernel_id
      || slice.worker_machine_id === remote.worker_machine_id,
    )
  if (!match) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  return match.id
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
