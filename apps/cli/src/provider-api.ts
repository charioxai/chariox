import {
  normalizeRuntimeSession,
  type ProviderAuthStatus,
  type ProviderAccountProfile,
  type ProviderLoginStart,
  type ProviderLoginStatus,
  type ProviderLogoutResult,
  type ProviderLogoutOutcome,
  type ProviderProcessInfo,
  type RuntimeProviderRun,
  type RuntimeSession,
  type SessionConfigState,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { CharioxLogger } from "./logging.js"
import {
  getProviderAuthStatusRequest,
  getProviderLoginStatusRequest,
  sendProviderLoginInputRequest,
  cancelProviderLoginRequest,
  listProviderAccountProfilesRequest,
  createProviderAccountProfileRequest,
  linkProviderAccountProfileRequest,
  importNativeProviderAccountProfileRequest,
  renameProviderAccountProfileRequest,
  setDefaultProviderAccountProfileRequest,
  refreshProviderAccountProfileRequest,
  removeProviderAccountProfileRequest,
  deleteProviderAccountProfileDataRequest,
  getProviderCatalogRequest,
  getProviderCommandCatalogsRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  launchProviderRunsRequest,
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

export async function getProviderCatalog(
  client: LocalIpcClient,
  logger?: CharioxLogger | null,
  options: Parameters<typeof getProviderCatalogRequest>[0] = {},
  fallbackOnError = true,
): Promise<ProviderCatalog> {
  try {
    const response = await client.send<Record<string, unknown>>(getProviderCatalogRequest(options))
    const payload = expectVariant<{ catalog: ProviderCatalog }>(response, "ProviderCatalog")
    logger?.info("Received provider catalog from daemon", {
      provider_count: payload.catalog.all.length,
      providers: payload.catalog.all.map((p) => ({ id: p.id, model_count: Object.keys(p.models).length })),
      connected: payload.catalog.connected,
    })
    return { ...payload.catalog, source: "daemon" }
  } catch (error) {
    if (!fallbackOnError) throw error
    const message = describeCliError(error)
    logger?.warn("provider catalog lookup failed; using fallback catalog", {
      error: message,
    })
    return fallbackProviderCatalog({ source: "local_fallback", unavailableReason: message })
  }
}

export async function getProviderCommandCatalogs(
  client: LocalIpcClient,
  logger?: CharioxLogger | null,
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
    return Object.fromEntries(
      Object.entries(payload.catalogs).map(([provider, catalog]) => [
        provider,
        { ...catalog, catalog_source: "daemon" as const },
      ]),
    ) as ProviderCommandCatalogs
  } catch (error) {
    const message = describeCliError(error)
    logger?.warn("provider command catalog lookup failed; using fallback command catalogs", {
      error: message,
    })
    return fallbackProviderCommandCatalogs({ catalogSource: "local_fallback", unavailableReason: message })
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
  logger?: CharioxLogger | null,
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

export async function launchProviderRuns(
  client: LocalIpcClient,
  launches: Array<{
    sessionId: string
    provider: string
    accountProfile: string
    model: string
    effort: string
    agentId?: string | null
  }>,
  maxConcurrency?: number | null,
): Promise<{
  providerRuns: RuntimeProviderRun[]
  failures: Array<{ index: number; agent_id?: string | null; message: string }>
}> {
  const response = await client.send<Record<string, unknown>>(launchProviderRunsRequest(launches, maxConcurrency))
  const payload = expectVariant<{
    provider_runs?: Array<{ provider_run: RuntimeProviderRun }>
    failures?: Array<{ index: number; agent_id?: string | null; message: string }>
  }>(response, "ProviderRunsLaunchAccepted")
  return {
    providerRuns: (payload.provider_runs ?? []).map((result) => result.provider_run),
    failures: payload.failures ?? [],
  }
}

export async function getProviderAuthStatus(
  client: LocalIpcClient,
  provider: string,
  accountProfile = "default",
): Promise<ProviderAuthStatus> {
  const response = await client.send<Record<string, unknown>>(getProviderAuthStatusRequest(provider, accountProfile))
  const payload = expectVariant<{ status: ProviderAuthStatus }>(response, "ProviderAuthStatus")
  return payload.status
}

export async function startProviderLogin(
  client: LocalIpcClient,
  provider: string,
  accountProfile = "default",
  method?: string,
): Promise<ProviderLoginStart> {
  const response = await client.send<Record<string, unknown>>(
    startProviderLoginRequest(provider, accountProfile, method),
  )
  const payload = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted")
  return payload.login
}

export async function getProviderLoginStatus(
  client: LocalIpcClient,
  loginId: string,
): Promise<ProviderLoginStatus> {
  const response = await client.send<Record<string, unknown>>(getProviderLoginStatusRequest(loginId))
  return expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginStatus").login
}

export async function sendProviderLoginInput(
  client: LocalIpcClient,
  loginId: string,
  dataBase64: string,
): Promise<{ login_id: string; byte_count: number }> {
  const response = await client.send<Record<string, unknown>>(sendProviderLoginInputRequest(loginId, dataBase64))
  return expectVariant<{ login_id: string; byte_count: number }>(response, "ProviderLoginInputSent")
}

export async function cancelProviderLogin(
  client: LocalIpcClient,
  loginId: string,
): Promise<ProviderLoginStatus> {
  const response = await client.send<Record<string, unknown>>(cancelProviderLoginRequest(loginId))
  return expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginCancelled").login
}

export async function logoutProvider(
  client: LocalIpcClient,
  provider: string,
  accountProfile = "default",
): Promise<ProviderLogoutOutcome> {
  const response = await client.send<Record<string, unknown>>(logoutProviderRequest(provider, accountProfile))
  if ("ProviderLoggedOut" in response) {
    return { kind: "logged_out", result: response.ProviderLoggedOut as ProviderLogoutResult }
  }
  return {
    kind: "interaction_required",
    workflow: expectVariant<{ logout: ProviderLoginStart }>(response, "ProviderLogoutStarted").logout,
  }
}

export async function listProviderAccountProfiles(client: LocalIpcClient, provider?: string | null): Promise<ProviderAccountProfile[]> {
  const response = await client.send<Record<string, unknown>>(listProviderAccountProfilesRequest(provider))
  return expectVariant<{ profiles: ProviderAccountProfile[] }>(response, "ProviderAccountProfilesListed").profiles
}

export async function createProviderAccountProfile(client: LocalIpcClient, provider: string, label: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(createProviderAccountProfileRequest(provider, label))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfile").profile
}

export async function linkProviderAccountProfile(client: LocalIpcClient, provider: string, label: string, path: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(linkProviderAccountProfileRequest(provider, label, path))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfile").profile
}

export async function importNativeProviderAccountProfile(client: LocalIpcClient, provider: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(importNativeProviderAccountProfileRequest(provider))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfile").profile
}

export async function renameProviderAccountProfile(client: LocalIpcClient, provider: string, profile: string, label: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(renameProviderAccountProfileRequest(provider, profile, label))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfile").profile
}

export async function setDefaultProviderAccountProfile(client: LocalIpcClient, provider: string, profile: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(setDefaultProviderAccountProfileRequest(provider, profile))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfile").profile
}

export async function refreshProviderAccountProfile(client: LocalIpcClient, provider: string, profile: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(refreshProviderAccountProfileRequest(provider, profile))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfile").profile
}

export async function removeProviderAccountProfile(client: LocalIpcClient, provider: string, profile: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(removeProviderAccountProfileRequest(provider, profile))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfileRemoved").profile
}

export async function deleteProviderAccountProfileData(client: LocalIpcClient, provider: string, profile: string): Promise<ProviderAccountProfile> {
  const response = await client.send<Record<string, unknown>>(deleteProviderAccountProfileDataRequest(provider, profile, profile))
  return expectVariant<{ profile: ProviderAccountProfile }>(response, "ProviderAccountProfileDataDeleted").profile
}

export { sameProviderRun } from "@chariox/kernel-client/session-runtime-lookup"

export function providerRunUsesNativeTui(run: RuntimeProviderRun | null | undefined): boolean {
  return run?.client_interface === "native_tui"
}
