import type { ArrobaLogger } from "./logging.js"
import type { ArrobaPreferences } from "./preferences.js"
import type { TerminalCommandCatalog } from "@arroba/kernel-client/kernel-types"
import { extractPromptInputHistoryEntries } from "@arroba/kernel-client/prompt-history"
import { fallbackProviderCatalog, type ProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs, type ProviderCommandCatalogs } from "./provider-command-catalog.js"
import { selectAttachableSession, decideBootstrapAction } from "./sessions.js"
import {
  resolveAttachTimeProviderLaunch,
} from "@arroba/kernel-client/session-lifecycle-state"
import { sessionHistoryCursorForVisibleAgent } from "@arroba/kernel-client/session-history-outline"

import type {
  CliOptions,
  BootstrapState,
  PromptInputHistoryEntry,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryOutline,
  SessionHistoryOutlineAgent,
  TranscriptEntry,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"

type BootstrapDeps = {
  logger?: ArrobaLogger | null
  listSessions: (client: LocalIpcClient) => Promise<RuntimeSession[]>
  getProviderCatalog: (client: LocalIpcClient, logger?: ArrobaLogger | null) => Promise<ProviderCatalog>
  getProviderCommandCatalogs: (client: LocalIpcClient, logger?: ArrobaLogger | null) => Promise<ProviderCommandCatalogs>
  getTerminalCommandCatalog: (client: LocalIpcClient, logger?: ArrobaLogger | null) => Promise<TerminalCommandCatalog>
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
  getSessionHistoryOutline: (
    client: LocalIpcClient,
    sessionId: string,
    agentIds: readonly string[],
  ) => Promise<SessionHistoryOutline>
  getPromptInputHistory?: (
    client: LocalIpcClient,
    sessionId: string,
  ) => Promise<{ entries: PromptInputHistoryEntry[] }>
  resolveVisibleAgentId: (session: RuntimeSession, preferences: ArrobaPreferences) => string | null
  prepareHistoryOutlineAgent: (agent: SessionHistoryOutlineAgent, session: RuntimeSession) => TranscriptEntry[]
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
    case "none": {
      const [providerCatalog, providerCommandCatalogs, terminalCommandCatalog] = await Promise.all([
        deps.getProviderCatalog(client, deps.logger),
        deps.getProviderCommandCatalogs(client, deps.logger),
        deps.getTerminalCommandCatalog(client, deps.logger),
      ])
      return {
        client,
        binding: null,
        sessions,
        providerCatalog,
        providerCommandCatalogs,
        terminalCommandCatalog,
        options,
        preferences,
      }
    }
  }

  if (!session) {
    const [providerCatalog, providerCommandCatalogs, terminalCommandCatalog] = await Promise.all([
      deps.getProviderCatalog(client, deps.logger),
      deps.getProviderCommandCatalogs(client, deps.logger),
      deps.getTerminalCommandCatalog(client, deps.logger),
    ])
    return {
      client,
      binding: null,
      sessions,
      providerCatalog,
      providerCommandCatalogs,
      terminalCommandCatalog,
      options,
      preferences,
    }
  }

  const attachment = await deps.attachToSession(client, session.id, options.clientId)
  const attachedSession = await deps.getSessionState(client, session.id)
  let providerRun: RuntimeProviderRun | null = null
  const launchDecision = resolveAttachTimeProviderLaunch(attachedSession, {
    provider: options.provider ?? "opencode",
    model: options.model,
    effort: options.effort,
  }, createdSession)
  switch (launchDecision.action) {
    case "launch_provider_run":
      providerRun = await deps.launchProviderRun(
        client,
        session.id,
        launchDecision.launch.provider,
        options.accountProfile,
        launchDecision.launch.model,
        launchDecision.launch.effort,
        launchDecision.targetAgentId,
      )
      break
    case "load_provider_run":
      providerRun = await deps.tryGetProviderRun(client, launchDecision.providerRunId, deps.logger)
      break
    case "skip_launch":
      if (launchDecision.reason === "no_visible_agents") {
        deps.logger?.warn("skipping provider launch because no agents are visible to this client", {
          session_id: session.id,
          focused_agent_id: attachedSession.focused_agent_id,
        })
      } else if (launchDecision.reason === "missing_focused_agent") {
        deps.logger?.warn("skipping provider launch because focused agent is not visible to this client", {
          session_id: session.id,
          focused_agent_id: attachedSession.focused_agent_id,
        })
      } else if (launchDecision.reason === "remote_backed_agent") {
        deps.logger?.info?.("skipping attach-time provider launch for remote-backed agent", {
          session_id: session.id,
          agent_id: launchDecision.targetAgent?.id ?? null,
          worker_kernel_id: launchDecision.targetAgent?.remote_execution?.worker_kernel_id ?? null,
        })
      }
      break
    default: {
      const exhaustive: never = launchDecision
      throw new Error(`unhandled attach provider launch decision ${String(exhaustive)}`)
    }
  }
  await deps.catchUpAttachedSession(client, session.id, attachment.id, attachedSession, deps.logger)
  const hydratedSession = await deps.getSessionState(client, session.id)
  const visibleAgentId = deps.resolveVisibleAgentId(hydratedSession, preferences)
  const providerCatalogPromise = deps.getProviderCatalog(client, deps.logger)
  const providerCommandCatalogsPromise = deps.getProviderCommandCatalogs(client, deps.logger)
  const terminalCommandCatalogPromise = deps.getTerminalCommandCatalog(client, deps.logger)
  const attachedHistoryPromise = hydrateAttachedHistory(
    client,
    session.id,
    visibleAgentId,
    hydratedSession,
    deps,
  )

  return {
    client,
    binding: {
      session: hydratedSession,
      attachment,
      providerRun,
      createdSession,
      historyEntries: [],
      promptHistoryEntries: [],
      nextHistoryCursor: null,
    },
    sessions,
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    providerCommandCatalogs: fallbackProviderCommandCatalogs({ catalogSource: "local_fallback" }),
    terminalCommandCatalog: null,
    options,
    preferences,
    deferred: {
      providerCatalog: providerCatalogPromise,
      providerCommandCatalogs: providerCommandCatalogsPromise,
      terminalCommandCatalog: terminalCommandCatalogPromise,
      attachedHistory: attachedHistoryPromise,
    },
  }
}

async function hydrateAttachedHistory(
  client: LocalIpcClient,
  sessionId: string,
  visibleAgentId: string | null,
  session: RuntimeSession,
  deps: Pick<BootstrapDeps, "getSessionHistoryOutline" | "getPromptInputHistory" | "prepareHistoryOutlineAgent">,
) {
  const agentIds = session.agents.map((agent) => agent.id)
  const outlinePromise = agentIds.length > 0
    ? deps.getSessionHistoryOutline(client, sessionId, agentIds)
    : Promise.resolve({ agents: [] })
  const [outline, promptHistoryEntries] = await Promise.all([
    outlinePromise,
    loadSessionPromptHistory(client, sessionId, deps),
  ])
  const agentEntries = Object.fromEntries(outline.agents.map((agent) => [
    agent.agent_id,
    deps.prepareHistoryOutlineAgent(agent, session),
  ]))
  const historyEntries = visibleAgentId ? agentEntries[visibleAgentId] ?? [] : []
  return {
    sessionId,
    visibleAgentId,
    agentEntries,
    historyEntries,
    promptHistoryEntries,
    nextHistoryCursor: sessionHistoryCursorForVisibleAgent(outline, visibleAgentId),
  }
}

async function loadSessionPromptHistory(
  client: LocalIpcClient,
  sessionId: string,
  deps: Pick<BootstrapDeps, "getPromptInputHistory">,
) {
  if (deps.getPromptInputHistory) {
    const history = await deps.getPromptInputHistory(client, sessionId)
    return extractPromptInputHistoryEntries(history.entries)
  }
  return []
}
