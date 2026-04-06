import type { ArrobaLogger } from "./logging.js"
import type { ArrobaPreferences } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import { selectAttachableSession, decideBootstrapAction } from "./sessions.js"

import type {
  CliOptions,
  BootstrapState,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryCursor,
  SessionHistoryPageEntry,
  TranscriptEntry,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"

type BootstrapDeps = {
  logger?: ArrobaLogger | null
  listSessions: (client: LocalIpcClient) => Promise<RuntimeSession[]>
  getProviderCatalog: (client: LocalIpcClient, logger?: ArrobaLogger | null) => Promise<ProviderCatalog>
  getProviderCommandCatalogs: (client: LocalIpcClient, logger?: ArrobaLogger | null) => Promise<ProviderCommandCatalogs>
  createSession: (client: LocalIpcClient, workspace: string, worktree: string, alias?: string) => Promise<RuntimeSession>
  resolveSession: (client: LocalIpcClient, sessionRef: string, workspace: string) => Promise<RuntimeSession>
  attachToSession: (client: LocalIpcClient, sessionId: string, clientId: string) => Promise<RuntimeAttachment>
  getSessionState: (client: LocalIpcClient, sessionId: string) => Promise<RuntimeSession>
  launchProviderRun: (
    client: LocalIpcClient,
    sessionId: string,
    provider: string,
    accountProfile: string,
    model: string,
    effort: string,
    agentId?: string | null,
  ) => Promise<RuntimeProviderRun>
  tryGetProviderRun: (
    client: LocalIpcClient,
    providerRunId: string,
    logger?: ArrobaLogger | null,
  ) => Promise<RuntimeProviderRun | null>
  catchUpAttachedSession: (
    client: LocalIpcClient,
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
    logger?: ArrobaLogger | null,
  ) => Promise<void>
  getSessionHistory: (
    client: LocalIpcClient,
    sessionId: string,
    cursor?: SessionHistoryCursor | null,
    agentId?: string | null,
  ) => Promise<{ entries: SessionHistoryPageEntry[]; next_cursor: SessionHistoryCursor | null }>
  resolveVisibleAgentId: (session: RuntimeSession, preferences: ArrobaPreferences) => string | null
  prepareHistoryEntries: (entries: SessionHistoryPageEntry[], session: RuntimeSession) => TranscriptEntry[]
}

export async function bootstrapSession(
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
  preferences: ArrobaPreferences,
  deps: BootstrapDeps,
): Promise<BootstrapState> {
  let createdSession = false
  let session: RuntimeSession | null = null

  const sessions = await deps.listSessions(client)
  let providerCatalog = await deps.getProviderCatalog(client, deps.logger)
  let providerCommandCatalogs = await deps.getProviderCommandCatalogs(client, deps.logger)
  const decision = decideBootstrapAction(options, sessions, workspace, worktree)
  switch (decision.action) {
    case "create":
      session = await deps.createSession(client, workspace, worktree, options.alias)
      createdSession = true
      break
    case "resolve":
      session = await deps.resolveSession(client, decision.sessionRef, workspace)
      break
    case "attach_existing": {
      const existing = selectAttachableSession(sessions, workspace, worktree)
      if (!existing) {
        session = await deps.createSession(client, workspace, worktree, options.alias)
        createdSession = true
        break
      }
      session = existing as RuntimeSession
      break
    }
    case "none":
      return {
        client,
        binding: null,
        sessions,
        providerCatalog,
        providerCommandCatalogs,
        options,
        preferences,
      }
  }

  if (!session) {
    return {
      client,
      binding: null,
      sessions,
      providerCatalog,
      providerCommandCatalogs,
      options,
      preferences,
    }
  }

  const attachment = await deps.attachToSession(client, session.id, options.clientId)
  const attachedSession = await deps.getSessionState(client, session.id)
  let providerRun: RuntimeProviderRun | null = null
  if (!attachedSession.active_provider_run_id) {
    const resolvedLaunch = resolveStoredAgentLaunch(attachedSession, {
      provider: options.provider ?? "opencode",
      model: options.model,
      effort: options.effort,
    }, createdSession)
    providerRun = await deps.launchProviderRun(
      client,
      session.id,
      resolvedLaunch.provider,
      options.accountProfile,
      resolvedLaunch.model,
      resolvedLaunch.effort,
      attachedSession.focused_agent_id,
    )
  } else {
    providerRun = await deps.tryGetProviderRun(client, attachedSession.active_provider_run_id, deps.logger)
  }
  providerCatalog = await deps.getProviderCatalog(client, deps.logger)
  providerCommandCatalogs = await deps.getProviderCommandCatalogs(client, deps.logger)
  await deps.catchUpAttachedSession(client, session.id, attachment.id, attachedSession, deps.logger)
  const hydratedSession = await deps.getSessionState(client, session.id)
  const visibleAgentId = deps.resolveVisibleAgentId(hydratedSession, preferences)
  const historyPage = visibleAgentId
    ? await deps.getSessionHistory(client, session.id, null, visibleAgentId)
    : { entries: [], next_cursor: null }

  return {
    client,
    binding: {
      session: hydratedSession,
      attachment,
      providerRun,
      createdSession,
      historyEntries: deps.prepareHistoryEntries(historyPage.entries, hydratedSession),
      nextHistoryCursor: historyPage.next_cursor,
    },
    sessions,
    providerCatalog,
    providerCommandCatalogs,
    options,
    preferences,
  }
}

function resolveStoredAgentLaunch(
  session: RuntimeSession,
  fallback: { provider: string; model: string; effort: string },
  createdSession: boolean,
) {
  if (createdSession) {
    return fallback
  }

  const focusedAgent = session.agents.find((agent) => agent.id === session.focused_agent_id) ?? session.agents[0]
  if (!focusedAgent) {
    return fallback
  }

  return {
    provider: focusedAgent.provider && focusedAgent.provider !== "default"
      ? focusedAgent.provider
      : fallback.provider,
    model: focusedAgent.model?.trim() || fallback.model,
    effort: focusedAgent.effort?.trim() || fallback.effort,
  }
}
