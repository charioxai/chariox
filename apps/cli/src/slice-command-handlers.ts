import type {
  SliceDisplayEndpoint,
  SliceRecord,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"

type FooterTone = "info" | "error"

type SliceCreateOptions = {
  name: string
  backend?: "local_docker" | "ssh_docker"
  os?: string
  displayMode?: "headless" | "headed"
  workspaceMount?: string | null
  workerKernelRef?: string | null
  displayUrl?: string | null
}

export type SliceCommandHandlerDeps = {
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
  getSliceDisplayEndpoint?: (sliceRef: string) => Promise<SliceDisplayEndpoint>
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
  deps.flashFooter("usage: /slice list | /slice create <name> [--headed|--headless] | /slice status [slice-ref] | /slice start [slice-ref] | /slice stop [slice-ref] | /slice delete <slice-ref> | /slice screen [slice-ref] | /slice auth import [slice-ref] <provider>", "error")
}

function formatSliceLabel(slice: SliceRecord): string {
  return slice.name || slice.id
}

function formatSlice(slice: SliceRecord): string {
  const display = slice.display_endpoint?.url ? ` screen=${slice.display_endpoint.url}` : ""
  const providers = (slice.providers ?? []).join(",") || "-"
  const worker = slice.worker_kernel_id ?? slice.worker_kernel_ref
  return `${formatSliceLabel(slice)} id=${slice.id} status=${slice.status} backend=${slice.backend} os=${slice.os} worker=${worker} providers=${providers}${slice.workspace_mount ? ` mount=${slice.workspace_mount}` : ""}${display}`
}

async function resolveFocusedSliceRef(deps: SliceCommandHandlerDeps): Promise<string> {
  const focusedId = deps.focusedAgentId()
  const resolved = deps.resolveSessionAgent(focusedId)
  const remote = resolved.agent?.remote_execution
  if (!remote) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  if (!deps.listSlices) {
    throw new Error("slice inventory is unavailable in this build")
  }
  const slices = await deps.listSlices()
  const match = slices.find((slice) =>
    slice.worker_kernel_id === remote.worker_kernel_id
    || slice.worker_kernel_ref === remote.worker_kernel_id
    || slice.worker_machine_id === remote.worker_machine_id
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

function parseSliceCreateOptions(
  deps: SliceCommandHandlerDeps,
  args: string[],
): {
  name?: string
  backend?: "local_docker" | "ssh_docker"
  workerKernelRef?: string | null
  displayUrl?: string | null
  workspaceMount?: string | null
  displayMode?: "headless" | "headed"
  error?: string
} {
  const name = args[0]
  let backend: "local_docker" | "ssh_docker" | undefined
  let workerKernelRef: string | null | undefined
  let displayUrl: string | null | undefined
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
  const slice = await handler(await explicitOrFocusedSliceRef(deps, sliceRef))
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
  const slice = await deps.deleteSlice(sliceRef)
  deps.flashFooter(`deleted slice ${formatSliceLabel(slice)}`, "info")
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
  deps.flashFooter(`slice auth import ${result.provider}: ${result.status}`, result.status === "imported" ? "info" : "error")
}
