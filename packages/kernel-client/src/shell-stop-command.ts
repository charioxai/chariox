import { cancelActivePromptRequest } from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAttachmentId } from "./shell-session-attachment.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellStopCommandDeps = {
  client: ShellKernelClient
}

export async function executeStopCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellStopCommandDeps,
): Promise<ShellCommandResult> {
  if (parsed.args.length > 0) {
    return { ok: false, message: "usage: stop" }
  }
  if (!context.sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const attachmentId = await resolveShellAttachmentId(context, deps)
  if (!attachmentId.ok) {
    return { ok: false, message: attachmentId.message }
  }
  const response = await deps.client.send(cancelActivePromptRequest(context.sessionId, attachmentId.attachmentId))
  const payload = expectVariant<{ cancellation: { prompt?: { id?: string | null } | null } }>(response, "PromptCancelled")
  return { ok: true, message: `cancellation requested${payload.cancellation.prompt?.id ? ` for prompt ${payload.cancellation.prompt.id}` : ""}`, data: payload }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
