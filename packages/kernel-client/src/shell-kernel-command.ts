import type { RuntimeSession } from "./kernel-types.js"
import { deleteKernelRequest } from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"

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
  if (action !== "delete" || args.length > 0) {
    return { ok: false, message: "usage: kernel delete" }
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

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
