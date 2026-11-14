import { cancelActivePromptRequest, cancelAgentTaskRequest, cancelAgentWakeRequest } from "./ipc-requests.js"
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
  const [kind, id, rawRevision] = parsed.args
  const revision = kind === "task" && rawRevision !== undefined && /^\d+$/u.test(rawRevision)
    ? Number(rawRevision) : undefined
  if (parsed.args.length > 0 && !(
    kind === "task" && parsed.args.length === 3 && id && revision !== undefined && Number.isSafeInteger(revision)
    || kind === "timer" && parsed.args.length === 2 && id
  )) {
    return { ok: false, message: "usage: stop [task <task-id> <revision> | timer <wake-id>]" }
  }
  if (!context.sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const attachmentId = await resolveShellAttachmentId(context, deps)
  if (!attachmentId.ok) {
    return { ok: false, message: attachmentId.message }
  }
  const request = kind === "task"
    ? cancelAgentTaskRequest(context.sessionId, attachmentId.attachmentId, id!, revision!)
    : kind === "timer"
      ? cancelAgentWakeRequest(context.sessionId, attachmentId.attachmentId, id!)
      : cancelActivePromptRequest(context.sessionId, attachmentId.attachmentId, context.agentId)
  const response = await deps.client.send(request)
  if ("AgentTaskUpdated" in response) {
    const payload = response.AgentTaskUpdated as { session_id: string; task: { task_id: string } }
    return { ok: true, message: kind === "timer" ? `cancelled timer ${id}` : `cancelled task ${payload.task.task_id}`, data: payload }
  }
  if (kind === "task" || kind === "timer") {
    throw new Error("unexpected response variant: expected AgentTaskUpdated")
  }
  if ("NothingToStop" in response) {
    return { ok: true, message: "nothing to stop", data: response.NothingToStop }
  }
  const payload = expectVariant<{ cancellation: { prompt?: { id?: string | null } | null } }>(response, "PromptCancelled")
  return { ok: true, message: `cancellation requested${payload.cancellation.prompt?.id ? ` for prompt ${payload.cancellation.prompt.id}` : ""}`, data: payload }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
