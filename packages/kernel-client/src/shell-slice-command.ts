import type {
  SliceDisplayEndpoint,
  SliceRecord,
} from "./kernel-types.js"
import {
  createSliceRequest,
  deleteSliceRequest,
  getSliceDisplayEndpointRequest,
  getSliceRequest,
  importSliceProviderAuthRequest,
  listSlicesRequest,
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
      if (first !== "import") {
        return { ok: false, message: "usage: slice auth import [slice-ref] <provider>" }
      }
      const sliceRef = rest.length > 1 ? rest[0]! : await focusedAgentSliceRef(context, deps)
      const provider = rest.length > 1 ? rest[1] : rest[0]
      if (!provider) {
        return { ok: false, message: "usage: slice auth import [slice-ref] <provider>" }
      }
      const response = await deps.client.send(importSliceProviderAuthRequest(sliceRef, provider))
      const payload = expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthImported")
      return { ok: true, message: `slice ${formatSliceLabel(payload.slice)} auth import ${payload.provider}: ${payload.status}`, data: payload }
    }
    default:
      return { ok: false, message: "usage: slice list|create|status|start|stop|delete|auth import|screen" }
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

function formatSlices(slices: SliceRecord[]): string {
  if (slices.length === 0) {
    return "no slices"
  }
  return slices.map(formatSlice).join("\n")
}

function formatSlice(slice: SliceRecord): string {
  const providers = (slice.providers ?? []).join(",") || "-"
  const display = slice.display_endpoint?.url ? ` display=${slice.display_endpoint.url}` : ""
  const auth = (slice.provider_auth ?? [])
    .map((entry) => `${entry.provider}:${entry.alias ?? entry.email ?? entry.account_id ?? entry.auth_type ?? entry.state}`)
    .join(",") || "-"
  const agents = slice.agent_ids?.length ?? 0
  const scope = slice.worktree_id ? ` worktree=${slice.worktree_id}` : ""
  return `${formatSliceLabel(slice)} status=${slice.status} backend=${slice.backend} os=${slice.os} display_mode=${slice.display_mode ?? "headless"} worker=${slice.worker_kernel_id ?? slice.worker_kernel_ref} agents=${agents}${scope} providers=${providers} auth=${auth}${display}`
}

function formatSliceLabel(slice: SliceRecord): string {
  return slice.name ? `${slice.name} id=${slice.id}` : slice.id
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
  const match = slices.find((slice) =>
    slice.worker_kernel_id === remote.worker_kernel_id
    || slice.worker_kernel_ref === remote.worker_kernel_id
    || slice.worker_machine_id === remote.worker_machine_id
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
