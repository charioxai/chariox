import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import type { AgentInstance, RuntimeProviderRun } from "../cli-types.js"
import type { AgentRuntimeActivity } from "../cli-types.js"
import { getSessionStateRequest } from "../ipc-requests.js"
import { LocalIpcClient } from "../ipc.js"
import {
  getNativeProviderRun,
  requestNativeProviderRunLaunch,
} from "./provider-run-control.js"

export async function launchClaudeNativeRun(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  model: string,
  effort: string,
): Promise<RuntimeProviderRun> {
  return requestNativeProviderRunLaunch(client, {
    sessionId,
    provider: "claude",
    model,
    effort,
    agentId,
    native: {
      structuredEndpoint: `native://claude/${process.pid}`,
    },
  })
}

export async function launchClaudeRemoteRenderedRun(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  model: string,
  effort: string,
): Promise<RuntimeProviderRun> {
  return requestNativeProviderRunLaunch(client, {
    sessionId,
    provider: "claude",
    model,
    effort,
    agentId,
  })
}

export async function waitForClaudeProviderRunReady(
  client: LocalIpcClient,
  providerRunId: string,
): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + 30_000
  let latest: RuntimeProviderRun | null = null
  let latestError: unknown = null
  while (Date.now() < deadline) {
    latest = await getProviderRunIfAvailable(client, providerRunId).catch((error) => {
      latestError = error
      return null
    })
    if (latest?.state === "Running") return latest
    if (latest?.state === "Ended") throw new Error(`Claude provider run ended before native TUI was ready: ${providerRunId}`)
    await sleep(250)
  }
  throw new Error(`timed out waiting for Claude provider run ${providerRunId}; latest state ${latest?.state ?? "unknown"}${latestError ? `; latest error ${formatError(latestError)}` : ""}`)
}

export async function waitForClaudeRemoteRenderedRunExit(
  client: LocalIpcClient,
  providerRunId: string,
  context?: { sessionId: string, agentId: string, debug?: (label: string, fields: unknown) => void },
  signal?: AbortSignal,
): Promise<void> {
  let sawProviderRun = false
  let endedAt: number | undefined
  while (true) {
    const readAt = Date.now()
    context?.debug?.("exit_run_read_start", { at: readAt })
    signal?.throwIfAborted()
    const run = await getProviderRunIfAvailable(client, providerRunId).catch((error) => {
      if (sawProviderRun || signal?.aborted) throw error
      return null
    })
    if (!run) {
      await sleep(500, undefined, { signal })
      continue
    }
    sawProviderRun = true
    context?.debug?.("exit_run_read_end", { at: Date.now(), elapsedMs: Date.now() - readAt, state: run.state })
    if (run.state === "Ended") {
      // MP-08 / MP-10 / MP-11: the kernel's exact turn record supplies the
      // error outcome. A native PTY exit alone cannot manufacture Cancelled.
      if (context) {
        const state = await client.send<{ SessionState?: { session?: { agents?: AgentInstance[] }, agent_activity?: Record<string, AgentRuntimeActivity> } }>(getSessionStateRequest(context.sessionId))
        const activity = state.SessionState?.agent_activity?.[context.agentId]
        context.debug?.("exit_settlement_read_end", { at: Date.now(), activeCount: activity?.active_prompt_count, settlement: activity?.last_completed_turn?.settlement_status })
        const turn = activity?.last_completed_turn
        const lease = state.SessionState?.session?.agents?.find(agent => agent.id === context.agentId)?.remote_execution?.leased_agent_id
        const sameRun = (id?: string | null) => id === providerRunId
          || Boolean(lease && id && providerRunId === `leased:${lease}:${id}`)
        if ((activity?.active_prompt_count ?? 0) > 0
          && (!activity?.active_turn?.provider_run_id || sameRun(activity.active_turn.provider_run_id))) {
          endedAt ??= Date.now()
          if (Date.now() - endedAt >= 20_000) throw new Error("Claude provider run ended before its active turn settled.")
          await sleep(25, undefined, { signal })
          continue
        }
        const exactRun = sameRun(turn?.provider_run_id)
        if (exactRun && turn?.settlement_status === "failed"
          && turn.provider_termination?.category === "runtime_failure") {
          throw new Error(turn.provider_termination?.reason ?? "Claude provider run failed.")
        }
      }
      return
    }
    await sleep(1_000, undefined, { signal })
  }
}

async function getProviderRunIfAvailable(client: LocalIpcClient, providerRunId: string): Promise<RuntimeProviderRun | null> {
  try {
    return await getNativeProviderRun(client, providerRunId)
  } catch (error) {
    if (isProviderRunNotFound(error)) return null
    throw error
  }
}

function isProviderRunNotFound(error: unknown): boolean {
  const message = formatError(error)
  return message.includes("provider run") && message.includes("not found")
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

// MP-08 / MP-10 / MP-11: close retires both transport lanes and their subscriptions.
export async function closeClaudeRemoteRenderedClient(client: LocalIpcClient): Promise<void> {
  await client.close()
}
