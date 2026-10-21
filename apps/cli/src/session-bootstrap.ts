import { requestInitialSessionView } from "./initial-session-view.js"
import { reconcileComposerCatalogs } from "./composer-catalog-reconciliation.js"
import { createComposerInventoryLoader } from "./composer-inventory-loader.js"
import { getRelayStatus, getKernelCloudRelayProfile } from "./relay-api.js"
import { bootstrapWaitingRoom } from "./waiting-room-bootstrap.js"
import type { CharioxLogger } from "./logging.js"
import type { CharioxPreferences } from "./preferences.js"
import type { TerminalCommandCatalog } from "@chariox/kernel-client/kernel-types"
import { extractPromptInputHistoryEntries } from "@chariox/kernel-client/prompt-history"
import { fallbackProviderCatalog, type ProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs, type ProviderCommandCatalogs } from "./provider-command-catalog.js"
import { selectAttachableSession, decideBootstrapAction } from "./sessions.js"
import { settleAttachProviderRun } from "./attach-provider-run.js"
import { sessionHistoryCursorForVisibleAgent } from "@chariox/kernel-client/session-history-outline"

import type {
  AttachLaunch,
  ComposerInventory,
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
import type { ProviderAccountProfile } from "@chariox/kernel-client"
import { selectedProviderAccountStartBlockReason } from "./waiting-room-provider-accounts.js"

export type BootstrapDeps = {
  getClientCatalogRevisions?: typeof import("./client-catalog-api.js").getClientCatalogRevisions
  deferProviderLaunch?: boolean
  listProviderAccountProfiles?: (client: LocalIpcClient) => Promise<ProviderAccountProfile[]>
  logger?: CharioxLogger | null
  getConfiguredProviderLaunchDefaults?: (client: LocalIpcClient) => Promise<{
    provider?: string
    model?: string
    effort?: string
  }>
  listSessions: (client: LocalIpcClient) => Promise<RuntimeSession[]>
  getProviderCatalog: (client: LocalIpcClient, logger?: CharioxLogger | null, options?: Parameters<typeof import("./provider-api.js").getProviderCatalog>[2], fallbackOnError?: boolean) => Promise<ProviderCatalog>
  getProviderCommandCatalogs: (client: LocalIpcClient, logger?: CharioxLogger | null, fallbackOnError?: boolean) => Promise<ProviderCommandCatalogs>
  getTerminalCommandCatalog: (client: LocalIpcClient, logger?: CharioxLogger | null) => Promise<TerminalCommandCatalog>
  createSession: (
    client: LocalIpcClient,
    workspace: string,
    worktree: string,
    alias?: string,
    agentDefaults?: RuntimeSession["agent_defaults"],
  ) => Promise<RuntimeSession>
  resolveSession: (client: LocalIpcClient, sessionRef: string, workspace: string) => Promise<RuntimeSession>
  attachToSession: (client: LocalIpcClient, sessionId: string, clientId: string) => Promise<RuntimeAttachment>
  attachSessionView?: (client: LocalIpcClient, sessionId: string, clientId: string, preferredAgentId: string | null) => Promise<{
    attachment: RuntimeAttachment; session: RuntimeSession; history: SessionHistoryOutline
  }>
  getSessionState: (client: LocalIpcClient, sessionId: string) => Promise<RuntimeSession>
  getCredentialVaultStatus?: (client: LocalIpcClient) => Promise<Record<string, unknown>>
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
    logger?: CharioxLogger | null,
  ) => Promise<RuntimeProviderRun | null>
  catchUpAttachedSession: (
    client: LocalIpcClient,
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
    logger?: CharioxLogger | null,
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
  resolveVisibleAgentId: (session: RuntimeSession, preferences: CharioxPreferences) => string | null
  prepareHistoryOutlineAgent: (agent: SessionHistoryOutlineAgent, session: RuntimeSession) => TranscriptEntry[]
}

export async function bootstrapSession(
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
  preferences: CharioxPreferences,
  deps: BootstrapDeps,
): Promise<BootstrapState> {
  if (decideBootstrapAction(options, [], workspace, worktree).action === "none") {
    return bootstrapWaitingRoom(client, options, preferences, deps)
  }
  let createdSession = false
  let session: RuntimeSession | null = null

  // MP-08 / MP-10 / MP-11: an explicit attach observes its existing
  // agent selection. Waiting-room inventory and new-session launch defaults
  // cannot affect its target and must not hold its first screen hostage.
  const explicitAttach = Boolean(options.sessionId && !options.createSession)
  const configuredPromise: Promise<{ provider?: string; model?: string; effort?: string }> = !explicitAttach && (!options.provider || options.model === "default" || !options.effort.trim())
    ? deps.getConfiguredProviderLaunchDefaults?.(client) ?? Promise.resolve({})
    : Promise.resolve({})
  const sessionsPromise = explicitAttach ? Promise.resolve([]) : deps.listSessions(client)
  const requestedView = explicitAttach ? requestInitialSessionView(client, options, preferences, deps.attachSessionView) : null
  const resolvedSessionPromise = explicitAttach
    ? requestedView ? requestedView.then(view => view?.session ?? deps.resolveSession(client, options.sessionId!, workspace))
      : deps.resolveSession(client, options.sessionId!, workspace)
    : Promise.resolve(null)
  const [configured, discoveredSessions, resolvedSession] = await Promise.all([
    configuredPromise, sessionsPromise, resolvedSessionPromise,
  ])
  const sessions = resolvedSession ? [resolvedSession] : discoveredSessions
  if (!options.provider && configured.provider) options.provider = configured.provider
  if (options.model === "default" && configured.model) options.model = configured.model
  if (!options.effort.trim() && configured.effort) options.effort = configured.effort

  const decision = decideBootstrapAction(options, sessions, workspace, worktree)
  const requestedAgentDefaults = options.provider
    ? {
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        account_profile: options.accountProfile,
      }
    : undefined
  const validateNewSessionAccount = async (): Promise<void> => {
    if (!deps.listProviderAccountProfiles || !options.provider) return
    const profiles = await deps.listProviderAccountProfiles(client)
    const reason = selectedProviderAccountStartBlockReason(profiles, options.provider, options.accountProfile, null, options.model)
    if (reason) throw new Error(reason)
  }
  switch (decision.action) {
    case "create":
      await validateNewSessionAccount()
      session = await deps.createSession(client, workspace, worktree, options.alias, requestedAgentDefaults)
      createdSession = true
      break
    case "resolve":
      session = resolvedSession ?? await deps.resolveSession(client, decision.sessionRef, workspace)
      break
    case "attach_existing": {
      const existing = selectAttachableSession(sessions, workspace, worktree)
      if (!existing) {
        await validateNewSessionAccount()
        session = await deps.createSession(client, workspace, worktree, options.alias, requestedAgentDefaults)
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

  const initialView = (requestedView ? await requestedView : null) ?? await deps.attachSessionView?.(client, session.id, options.clientId, deps.resolveVisibleAgentId(session, preferences))
  const attachment = initialView?.attachment ?? await deps.attachToSession(client, session.id, options.clientId)
  const attachedSession = initialView?.session ?? await deps.getSessionState(client, session.id)
  // MP-08 / MP-10 PANE-03: with a bundled initial view the transcript needs no
  // launch result, so the attach-time launch runs beside the first paint.
  const launching = settleAttachLaunch(client, session, attachedSession, attachment.id, createdSession, Boolean(initialView), options, deps)
  launching.catch(() => {})
  const settled = deps.deferProviderLaunch ? await launching : initialView ? null : await launching
  const hydratedSession = settled?.session ?? attachedSession
  const visibleAgentId = deps.resolveVisibleAgentId(hydratedSession, preferences)
  // MP-08 / MP-10: composer inventories are user requested, never attach work.
  const refreshComposer = async (cached?: ComposerInventory): Promise<ComposerInventory> => {
    const catalogOptions = { provider: options.provider ?? null, accountProfile: options.accountProfile }
    const catalogs = await reconcileComposerCatalogs(cached, {
      revisions: async () => deps.getClientCatalogRevisions?.(client, catalogOptions),
      provider: () => deps.getProviderCatalog(client, deps.logger, catalogOptions, false),
      providerCommands: () => deps.getProviderCommandCatalogs(client, deps.logger, false),
      terminalCommands: () => deps.getTerminalCommandCatalog(client, deps.logger),
    })
    return { sessionId: session.id, ...catalogs, promptHistoryEntries: [] }
  }
  const cachedComposer = createComposerInventoryLoader(async () => {
    const profile = options.targetDaemonId ? preferences.relay?.cloud : await getKernelCloudRelayProfile(client)
    const owner = profile ? JSON.stringify([profile.apiUrl, profile.userId, profile.accountId, profile.realmId]) : "local"
    const kernel = options.targetDaemonId ?? (await getRelayStatus(client)).daemon_id
    return { owner, key: JSON.stringify([owner, kernel, options.provider, options.accountProfile]) }
  }, refreshComposer, () => JSON.stringify([options.provider, options.accountProfile]))
  const loadComposer = () => cachedComposer().then(value => ({ ...value, sessionId: session.id }))
  const attachedHistoryPromise = hydrateAttachedHistory(
    client,
    session.id,
    visibleAgentId,
    hydratedSession,
    deps,
    initialView?.history,
    true,
  )

  return {
    client,
    binding: {
      session: hydratedSession,
      attachment,
      providerRun: settled?.providerRun ?? null,
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
      loadComposer,
      composerScopeKey: () => JSON.stringify([options.provider, options.accountProfile]),
      loadPromptInputHistory: (sessionId: string) => loadSessionPromptHistory(client, sessionId, deps).then(entries => ({ sessionId, entries })),
      attachedHistory: attachedHistoryPromise,
      ...(settled?.providerLaunch ? { providerLaunch: settled.providerLaunch } : {}),
      ...(initialView && !deps.deferProviderLaunch ? { attachLaunch: launching, historyIsBundled: true as const } : {}),
    },
  }
}

async function settleAttachLaunch(
  client: LocalIpcClient,
  session: RuntimeSession,
  attachedSession: RuntimeSession,
  attachmentId: string,
  createdSession: boolean,
  hasInitialView: boolean,
  options: CliOptions,
  deps: BootstrapDeps,
): Promise<AttachLaunch> {
  const providerSettlement = await settleAttachProviderRun(
    attachedSession,
    {
      provider: options.provider ?? "opencode",
      model: options.model,
      effort: options.effort,
    },
    options.accountProfile,
    createdSession,
    {
      ...(deps.getCredentialVaultStatus
        ? { getCredentialVaultStatus: () => deps.getCredentialVaultStatus!(client) }
        : {}),
      deferLaunch: deps.deferProviderLaunch === true,
      launchProviderRun: (sessionId, provider, accountProfile, model, effort, targetAgentId) =>
        deps.launchProviderRun(client, sessionId, provider, accountProfile, model, effort, targetAgentId),
      getSessionState: (sessionId) => deps.getSessionState(client, sessionId),
      tryGetProviderRun: (providerRunId) => deps.tryGetProviderRun(client, providerRunId, deps.logger),
    },
  )
  if (providerSettlement.action === "deferred") {
    return { sessionId: session.id, session: null, providerRun: null, issue: null,
      providerLaunch: { sessionId: session.id, agentId: providerSettlement.targetAgentId,
        isPending: providerSettlement.isPending, start: providerSettlement.start } }
  }
  const settledSession = providerSettlement.session
  if (providerSettlement.action === "skipped") {
    if (providerSettlement.recoveredRemotePlacement) {
      deps.logger?.info?.("recovered attach-time provider launch after agent moved remote", {
        session_id: session.id,
        agent_id: providerSettlement.targetAgent?.id ?? null,
        worker_kernel_id: providerSettlement.targetAgent?.remote_execution?.worker_kernel_id ?? null,
      })
    } else if (providerSettlement.reason === "no_visible_agents") {
      deps.logger?.warn("skipping provider launch because no agents are visible to this client", {
        session_id: session.id,
        focused_agent_id: settledSession.focused_agent_id,
      })
    } else if (providerSettlement.reason === "missing_focused_agent") {
      deps.logger?.warn("skipping provider launch because focused agent is not visible to this client", {
        session_id: session.id,
        focused_agent_id: settledSession.focused_agent_id,
      })
    } else if (providerSettlement.reason === "remote_backed_agent") {
      deps.logger?.info?.("skipping attach-time provider launch for remote-backed agent", {
        session_id: session.id,
        agent_id: providerSettlement.targetAgent?.id ?? null,
        worker_kernel_id: providerSettlement.targetAgent?.remote_execution?.worker_kernel_id ?? null,
      })
    } else if (providerSettlement.reason === "credential_vault_locked") {
      deps.logger?.warn("skipping attach-time provider launch because the credential vault is locked", {
        session_id: session.id,
        agent_id: providerSettlement.targetAgent?.id ?? null,
      })
    }
  }
  // The new attachment already buffers output after its history read. Let the
  // live runtime drain it; catch-up would discard rows absent from this view.
  if (!hasInitialView) await deps.catchUpAttachedSession(client, session.id, attachmentId, settledSession, deps.logger)
  return {
    sessionId: session.id,
    // Only a state read after the launch (or a recovered placement) may replace
    // the live session; the bundled view is already what the client shows.
    session: !hasInitialView || providerSettlement.action === "launched"
      ? await deps.getSessionState(client, session.id)
      : settledSession === attachedSession ? null : settledSession,
    providerRun: providerSettlement.providerRun,
    issue: providerSettlement.action === "skipped" && providerSettlement.reason === "credential_vault_locked"
      ? "credential_vault_locked" : null,
  }
}

async function hydrateAttachedHistory(
  client: LocalIpcClient,
  sessionId: string,
  visibleAgentId: string | null,
  session: RuntimeSession,
  deps: Pick<BootstrapDeps, "getSessionHistoryOutline" | "getPromptInputHistory" | "prepareHistoryOutlineAgent">,
  initialOutline?: SessionHistoryOutline,
  deferPromptHistory = false,
) {
  const agentIds = session.agents.map((agent) => agent.id)
  const outlinePromise = initialOutline
    ? Promise.resolve(initialOutline)
    : agentIds.length > 0
    ? deps.getSessionHistoryOutline(client, sessionId, agentIds)
    : Promise.resolve({ agents: [] })
  const [outline, promptHistoryEntries] = await Promise.all([
    outlinePromise,
    deferPromptHistory ? Promise.resolve([]) : loadSessionPromptHistory(client, sessionId, deps),
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
    includesPromptHistory: !deferPromptHistory,
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
