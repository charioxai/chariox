import {
  normalizeRuntimeSession,
  type ProviderAuthStatus,
  type ProviderLoginStart,
  type ProviderLogoutResult,
  type ProviderProcessInfo,
  type RuntimeProviderRun,
  type RuntimeSession,
  type SessionConfigState,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaLogger } from "./logging.js"
import {
  getProviderAuthStatusRequest,
  getProviderCatalogRequest,
  getProviderCommandCatalogsRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  listProviderProcessesRequest,
  logoutProviderRequest,
  startProviderLoginRequest,
  teardownProviderProcessesRequest,
  updateSessionConfigRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import {
  fallbackProviderCatalog,
  type ProviderCatalog,
} from "./provider-catalog.js"
import {
  fallbackProviderCommandCatalogs,
  type ProviderCommandCatalogs,
} from "./provider-command-catalog.js"
import { describeCliError } from "./runtime.js"

export async function getProviderCatalog(client: LocalIpcClient, logger?: ArrobaLogger | null): Promise<ProviderCatalog> {
  try {
    const response = await client.send<Record<string, unknown>>(getProviderCatalogRequest())
    const payload = expectVariant<{ catalog: ProviderCatalog }>(response, "ProviderCatalog")
    logger?.info("Received provider catalog from daemon", {
      provider_count: payload.catalog.all.length,
      providers: payload.catalog.all.map((p) => ({ id: p.id, model_count: Object.keys(p.models).length })),
      connected: payload.catalog.connected,
    })
    return payload.catalog
  } catch (error) {
    logger?.warn("provider catalog lookup failed; using fallback catalog", {
      error: describeCliError(error),
    })
    return fallbackProviderCatalog()
  }
}

export async function getProviderCommandCatalogs(
  client: LocalIpcClient,
  logger?: ArrobaLogger | null,
): Promise<ProviderCommandCatalogs> {
  try {
    const response = await client.send<Record<string, unknown>>(getProviderCommandCatalogsRequest())
    const payload = expectVariant<{ catalogs: ProviderCommandCatalogs }>(response, "ProviderCommandCatalogs")
    logger?.info("Received provider command catalogs from daemon", {
      providers: Object.values(payload.catalogs).map((catalog) => ({
        provider: catalog.provider,
        command_count: catalog.commands.length,
        source: catalog.source,
        discovery: catalog.discovery,
      })),
    })
    return payload.catalogs
  } catch (error) {
    logger?.warn("provider command catalog lookup failed; using fallback command catalogs", {
      error: describeCliError(error),
    })
    return fallbackProviderCommandCatalogs()
  }
}

export async function listProviderProcesses(
  client: LocalIpcClient,
  provider?: string | null,
): Promise<ProviderProcessInfo[]> {
  const response = await client.send<Record<string, unknown>>(
    listProviderProcessesRequest(provider),
  )
  return expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesListed").processes
}

export async function teardownProviderProcesses(
  client: LocalIpcClient,
  provider?: string | null,
): Promise<ProviderProcessInfo[]> {
  const response = await client.send<Record<string, unknown>>(
    teardownProviderProcessesRequest(provider),
  )
  return expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesTornDown").processes
}

export async function getProviderRun(client: LocalIpcClient, providerRunId: string): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(getProviderRunRequest(providerRunId))
  const payload = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun")
  return payload.provider_run
}

export async function tryGetProviderRun(
  client: LocalIpcClient,
  providerRunId: string,
  logger?: ArrobaLogger | null,
): Promise<RuntimeProviderRun | null> {
  try {
    return await getProviderRun(client, providerRunId)
  } catch (error) {
    const message = describeCliError(error)
    if (!/unknown variant `GetProviderRun`/i.test(message)) {
      throw error
    }
    logger?.warn("daemon does not support provider run lookup", {
      provider_run_id: providerRunId,
    })
    return null
  }
}

export async function updateSessionConfig(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  values: Record<string, string>,
  requiresIdle: boolean,
): Promise<{ session: RuntimeSession, config: SessionConfigState }> {
  const response = await client.send<Record<string, unknown>>(
    updateSessionConfigRequest(sessionId, attachmentId, values, requiresIdle),
  )
  const payload = expectVariant<{ session: RuntimeSession, config: SessionConfigState }>(response, "SessionConfigUpdated")
  return {
    ...payload,
    session: normalizeRuntimeSession(payload.session),
  }
}

export async function launchProviderRun(
  client: LocalIpcClient,
  sessionId: string,
  provider: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId?: string | null,
): Promise<RuntimeProviderRun> {
  const response = await client.send<Record<string, unknown>>(launchProviderRunRequest(sessionId, provider, accountProfile, model, effort, agentId))
  const payload = "ProviderRunLaunched" in response
    ? expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunched")
    : expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRunLaunchAccepted")
  return payload.provider_run
}

export async function getProviderAuthStatus(
  client: LocalIpcClient,
  provider: string,
): Promise<ProviderAuthStatus> {
  const response = await client.send<Record<string, unknown>>(getProviderAuthStatusRequest(provider))
  const payload = expectVariant<{ status: ProviderAuthStatus }>(response, "ProviderAuthStatus")
  return payload.status
}

export async function startProviderLogin(
  client: LocalIpcClient,
  provider: string,
): Promise<ProviderLoginStart> {
  const response = await client.send<Record<string, unknown>>(startProviderLoginRequest(provider))
  const payload = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted")
  return payload.login
}

export async function logoutProvider(
  client: LocalIpcClient,
  provider: string,
): Promise<ProviderLogoutResult> {
  const response = await client.send<Record<string, unknown>>(logoutProviderRequest(provider))
  return expectVariant<ProviderLogoutResult>(response, "ProviderLoggedOut")
}

export function sameProviderRun(left: RuntimeProviderRun, right: RuntimeProviderRun) {
  return left.id === right.id
    && left.session_id === right.session_id
    && left.agent_instance_id === right.agent_instance_id
    && left.adapter_key === right.adapter_key
    && left.provider === right.provider
    && left.account_profile === right.account_profile
    && left.model === right.model
    && left.variant === right.variant
    && left.client_interface === right.client_interface
    && left.usage_tokens_total === right.usage_tokens_total
    && left.state === right.state
}

export function providerRunUsesNativeTui(run: RuntimeProviderRun | null | undefined): boolean {
  return run?.client_interface === "native_tui"
}
