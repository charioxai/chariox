import { externalProviderSessionPage } from "@arroba/kernel-client/external-provider-sessions"
import {
  normalizeRuntimeSession,
  type AgentInstance,
  type RuntimeProviderRun,
  type RuntimeSession,
  type ExternalProviderSessionRecord,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  importExternalProviderAgentRequest,
  importExternalProviderSessionRequest,
  listExternalProviderSessionsRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function listExternalProviderSessions(
  client: LocalIpcClient,
  options: {
    provider?: string | null
    cursor?: string | null
    limit?: number | null
  } = {},
): Promise<{
  sessions: ExternalProviderSessionRecord[]
  hasMore: boolean
  nextCursor: string | null
}> {
  const response = await client.send<Record<string, unknown>>(
    listExternalProviderSessionsRequest(options),
  )
  const payload = expectVariant<{
    page: {
      sessions?: ExternalProviderSessionRecord[]
      has_more?: boolean
      next_cursor?: string | null
    }
  }>(response, "ExternalProviderSessionsListed")
  return externalProviderSessionPage(payload.page)
}

export async function importExternalProviderSession(
  client: LocalIpcClient,
  externalSessionId: string,
): Promise<{ session: RuntimeSession; agent: AgentInstance; providerRun: RuntimeProviderRun | null }> {
  const response = await client.send<Record<string, unknown>>(
    importExternalProviderSessionRequest(externalSessionId),
  )
  const payload = expectVariant<{
    session: RuntimeSession
    agent: AgentInstance
    provider_run?: RuntimeProviderRun | null
  }>(response, "ExternalProviderSessionImported")
  return {
    session: normalizeRuntimeSession(payload.session),
    agent: payload.agent,
    providerRun: payload.provider_run ?? null,
  }
}

export async function importExternalProviderAgent(
  client: LocalIpcClient,
  sessionId: string,
  externalSessionId: string,
): Promise<{ session: RuntimeSession; agent: AgentInstance; providerRun: RuntimeProviderRun | null }> {
  const response = await client.send<Record<string, unknown>>(
    importExternalProviderAgentRequest(sessionId, externalSessionId, { focus: true }),
  )
  const payload = expectVariant<{
    session: RuntimeSession
    agent: AgentInstance
    provider_run?: RuntimeProviderRun | null
  }>(response, "ExternalProviderAgentImported")
  return {
    session: normalizeRuntimeSession(payload.session),
    agent: payload.agent,
    providerRun: payload.provider_run ?? null,
  }
}
