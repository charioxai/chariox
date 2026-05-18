import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import type { RuntimeProviderRun } from "../cli-types.js"
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
): Promise<void> {
  let sawProviderRun = false
  while (true) {
    const run = await getProviderRunIfAvailable(client, providerRunId).catch((error) => {
      if (sawProviderRun) throw error
      return null
    })
    if (!run) {
      await sleep(500)
      continue
    }
    sawProviderRun = true
    if (run.state === "Ended") return
    await sleep(1_000)
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
