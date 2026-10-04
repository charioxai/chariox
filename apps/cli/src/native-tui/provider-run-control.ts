import { setTimeout as sleep } from "node:timers/promises"
import type { RuntimeProviderRun } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  getProviderRunRequest,
  launchProviderRunRequest,
} from "../ipc-requests.js"

export type NativeProviderRunBinding = {
  structuredEndpoint?: string | null
  providerSessionId?: string | null
  nativeTui?: boolean | null
}

export async function requestNativeProviderRunLaunch(
  client: LocalIpcClient,
  options: {
    sessionId: string
    provider: string
    accountProfile?: string
    model: string
    effort: string
    agentId: string
    native?: NativeProviderRunBinding
  },
): Promise<RuntimeProviderRun> {
  const nativeBinding = {
    ...(options.native?.structuredEndpoint !== undefined ? { structuredEndpoint: options.native.structuredEndpoint } : {}),
    ...(options.native?.providerSessionId !== undefined ? { providerSessionId: options.native.providerSessionId } : {}),
    nativeTui: true,
  }
  const response = await client.send<Record<string, unknown>>(
    launchProviderRunRequest(
      options.sessionId,
      options.provider,
      options.accountProfile ?? "default",
      options.model,
      options.effort,
      options.agentId,
      nativeBinding,
    ),
  )
  return "ProviderRunLaunched" in response
    ? expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched").provider_run
    : expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunchAccepted").provider_run
}

export async function getNativeProviderRun(
  client: LocalIpcClient,
  providerRunId: string,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId))
  return expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun").provider_run
}

// MP-08 / MP-10: LaunchAccepted contains planned metadata. Running is
// published only after the kernel finishes initializing the provider runtime.
export async function waitForNativeProviderRunReady(
  client: LocalIpcClient,
  providerRunId: string,
  options: { timeoutMs?: number, pollIntervalMs?: number } = {},
): Promise<RuntimeProviderRun> {
  const deadline = Date.now() + (options.timeoutMs ?? 60_000)
  let latest: RuntimeProviderRun | null = null
  while (Date.now() < deadline) {
    latest = await getNativeProviderRun(client, providerRunId)
    if (latest.state === "Ended" || latest.state === "Parked") {
      throw new Error(`Codex provider run ${latest.state.toLowerCase()} before attach was ready: ${providerRunId}`)
    }
    if (latest.state === "Running") return latest
    await sleep(Math.min(options.pollIntervalMs ?? 250, Math.max(0, deadline - Date.now())))
  }
  throw new Error(`timed out waiting for Codex provider run to become ready: ${providerRunId} (${latest?.state ?? "unknown"})`)
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
