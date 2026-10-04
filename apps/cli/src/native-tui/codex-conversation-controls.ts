import { setTimeout as sleep } from "node:timers/promises"

import type { LocalIpcClient } from "../ipc.js"
import type { CodexJsonRpcMessage } from "./codex-json-rpc.js"
import type { CodexNativeBindingState } from "./codex-turn-submission.js"
import { getNativeProviderRun } from "./provider-run-control.js"

// MP-08 / MP-10: A display thread is not the managed conversation. Resolve
// the kernel-owned run on each control so a stale launch snapshot cannot
// compact an empty display thread or a previous execution conversation.
export async function mapCodexNativeCompaction(
  message: CodexJsonRpcMessage,
  options: {
    client: LocalIpcClient
    sessionId: string
    agentId: string
    bindState: CodexNativeBindingState
  },
  timeoutMs = 15_000,
): Promise<CodexJsonRpcMessage> {
  const boundRun = options.bindState.run
  if (!boundRun) throw new Error("Codex conversation is not bound to Chariox yet")
  const deadline = Date.now() + timeoutMs
  const timeoutError = () => new Error("timed out waiting for the managed Codex conversation")
  while (Date.now() < deadline) {
    let timer: ReturnType<typeof setTimeout> | undefined
    const run = await Promise.race([
      getNativeProviderRun(options.client, boundRun.id),
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(timeoutError()), Math.max(0, deadline - Date.now()))
      }),
    ]).finally(() => clearTimeout(timer))
    if (run.id !== boundRun.id || run.session_id !== options.sessionId
      || run.agent_instance_id !== options.agentId || run.provider !== "codex"
      || run.state !== "Running" || run.structured_endpoint !== boundRun.structured_endpoint) {
      throw new Error("managed Codex conversation changed or ended before compaction")
    }
    if (run.provider_session_id) {
      return { ...message, params: { ...message.params, threadId: run.provider_session_id } }
    }
    await sleep(Math.min(100, Math.max(0, deadline - Date.now())))
  }
  throw timeoutError()
}
