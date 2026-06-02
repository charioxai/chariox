import type { DaemonHealthProjection, RuntimeSession } from "./kernel-types.js"
import { deleteKernelRequest, exportDebugBundleRequest, getDaemonHealthRequest, getSessionStateRequest } from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatKernelHealth,
  kernelHealthIssueCount,
} from "./shell-kernel-health-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellKernelCommandDeps = {
  client: ShellKernelClient
}

export async function executeKernelCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellKernelCommandDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  if ((action === "health" || action === "status") && args.length === 0) {
    const response = await deps.client.send(getDaemonHealthRequest())
    const payload = expectVariant<{ projection: DaemonHealthProjection }>(response, "DaemonHealth")
    const runtimeContext = await kernelHealthRuntimeContext(context, deps)
    const issueCount = kernelHealthIssueCount(payload.projection)
    return {
      ok: issueCount === 0,
      message: [runtimeContext, formatKernelHealth(payload.projection)].filter(Boolean).join("\n"),
      data: payload.projection,
    }
  }
  if (action === "debug-bundle" && args.length <= 1) {
    if (!context.sessionId) {
      return { ok: false, message: "kernel debug-bundle requires an active session" }
    }
    const response = await deps.client.send(exportDebugBundleRequest(context.sessionId, { bundleLabel: args[0] ?? null }))
    const payload = expectVariant<{
      bundle_dir: string
      manifest_path: string
      logs_path: string
      record_count: number
      limit: number
    }>(response, "DebugBundleExported")
    return {
      ok: true,
      message: `kernel debug bundle exported on kernel machine: ${payload.bundle_dir} (${payload.record_count}/${payload.limit} records)`,
      data: payload,
    }
  }
  if (action !== "delete" || args.length > 0) {
    return { ok: false, message: "usage: kernel health|status|debug-bundle [label]|delete" }
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

async function kernelHealthRuntimeContext(
  context: ShellContext,
  deps: ShellKernelCommandDeps,
): Promise<string> {
  if (!context.sessionId) {
    return ""
  }
  try {
    const response = await deps.client.send(getSessionStateRequest(context.sessionId))
    const session = expectSessionState(response)
    return [
      "session runtime:",
      `  session: ${session.id}`,
      `  home kernel: ${formatHomeKernel(session)}`,
      `  owner: ${session.owner_user_id?.trim() || "-"}`,
      `  agent: ${context.agentId ?? "-"}`,
    ].join("\n")
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    return [
      "session runtime:",
      `  session: ${context.sessionId}`,
      "  home kernel: unknown",
      `  lookup: ${message || "failed"}`,
    ].join("\n")
  }
}

function expectSessionState(response: Record<string, unknown>): RuntimeSession {
  if ("SessionState" in response) {
    return (response.SessionState as { session: RuntimeSession }).session
  }
  return expectVariant<{ session: RuntimeSession }>(response, "SessionStateLoaded").session
}

function formatHomeKernel(session: RuntimeSession): string {
  const kernel = session.host_daemon_id?.trim() || ""
  const machine = session.host_machine_id?.trim() || ""
  if (kernel && machine) {
    return `${kernel}@${machine}`
  }
  return kernel || machine || "-"
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
