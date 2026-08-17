import type { LocalIpcClient } from "./ipc.js"
import { LocalIpcClient as DefaultLocalIpcClient } from "./ipc.js"
import type {
  BootstrapState,
  CliOptions,
} from "./cli-types.js"
import {
  applyProviderPreferenceDefaults,
  defaultKernelEndpoint,
  parseArgs,
} from "./cli-options.js"
import {
  attachToSession,
  createSession,
  deleteSessionByRef,
  getSessionState,
  listSessions,
  resolveSession,
} from "./session-api.js"
import {
  catchUpAttachedSession,
  resizeSessionTerminal,
} from "./session-runtime-api.js"
import {
  getPromptInputHistory,
  getSessionHistoryOutline,
} from "./session-history-api.js"
import {
  getProviderCatalog,
  getProviderCommandCatalogs,
  launchProviderRun,
  tryGetProviderRun,
} from "./provider-api.js"
import { getTerminalCommandCatalog } from "./terminal-command-catalog-api.js"
import {
  fallbackProviderCatalog,
} from "./provider-catalog.js"
import {
  fallbackProviderCommandCatalogs,
} from "./provider-command-catalog.js"
import {
  loadPreferences,
  resolveMaxAgentsPerScreen,
  type CharioxPreferences,
} from "./preferences.js"
import { getUserConfig } from "./config-api.js"
import {
  bootstrapSession,
} from "./session-bootstrap.js"
import {
  sessionResponseLayout,
} from "@chariox/kernel-client/session-config-projection"
import {
  sessionFocusedAgentId,
} from "@chariox/kernel-client/session-runtime-transition"
import {
  selectResponsePaneAgents,
} from "@chariox/kernel-client/response-pane-selection"
import { hydrateSessionHistoryOutlineAgentEntries } from "@chariox/kernel-client/session-history-transcript"
import { reindexTranscriptEntries } from "@chariox/kernel-client/transcript-entry-state"
import {
  inferWorkspaceTargetsFromLaunchDirectory,
} from "./workspace-launch-targets.js"
import {
  isKernelEndpointReachable,
  isKernelEndpointUnavailableError,
  isNoArgDefaultKernelLaunch,
} from "./kernel-endpoint.js"
import {
  loadThemeRegistry,
  type ThemeLoadWarning,
  type ThemeRegistry,
} from "./theme-registry.js"
import {
  primeWaitingRoomWorktreeInventory,
} from "./waiting-room-worktrees.js"
import {
  describeCliError,
} from "./runtime.js"
import type { CharioxLogger } from "./logging.js"

type RelayClientOptions = {
  relayAuthToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
}

type BootstrapAttachedSession = (
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
  preferences: CharioxPreferences,
  logger?: CharioxLogger | null,
) => Promise<BootstrapState>

export type CliRuntimeBootstrapDeps = {
  parseArgs: (argv: string[]) => CliOptions
  loadPreferences: () => Promise<CharioxPreferences>
  applyProviderPreferenceDefaults: (options: CliOptions, preferences: CharioxPreferences) => CliOptions
  defaultKernelEndpoint: () => string
  createClient: (endpoint: string, relayOptions?: RelayClientOptions) => LocalIpcClient
  inferWorkspaceTargetsFromLaunchDirectory: (cwd: string) => Promise<{ workspace: string; worktree: string }>
  primeWaitingRoomWorktreeInventory: (options: {
    cwd: string
    workspacePath: string
    currentWorktreePath: string
  }) => Promise<void>
  loadThemeRegistry: (options: {
    workspace?: string
    onWarning?: (warning: ThemeLoadWarning) => void
  }) => Promise<ThemeRegistry>
  deleteSessionByRef: (client: LocalIpcClient, sessionRef: string, workspace: string) => Promise<unknown>
  isNoArgDefaultKernelLaunch: (argv: string[]) => boolean
  isKernelEndpointReachable: (endpoint: string) => Promise<boolean>
  isKernelEndpointUnavailableError: (error: unknown) => boolean
  bootstrapAttachedSession: BootstrapAttachedSession
  maybeResize: (client: LocalIpcClient, sessionId: string) => Promise<void>
}

export type CliRuntimeBootstrapOptions = {
  argv: string[]
  cwd: string
  logger?: CharioxLogger | null
}

export type CliRuntimeBootstrapResult =
  | {
    kind: "deleted_session"
    client: LocalIpcClient
    options: CliOptions
    kernelEndpoint: string
    workspace: string
    worktree: string
  }
  | {
    kind: "ready"
    bootstrap: BootstrapState
    kernelEndpoint: string
    workspace: string
    worktree: string
  }

export const defaultCliRuntimeBootstrapDeps: CliRuntimeBootstrapDeps = {
  parseArgs,
  loadPreferences,
  applyProviderPreferenceDefaults,
  defaultKernelEndpoint,
  createClient(endpoint, relayOptions) {
    return new DefaultLocalIpcClient(endpoint, relayOptions)
  },
  inferWorkspaceTargetsFromLaunchDirectory,
  primeWaitingRoomWorktreeInventory,
  loadThemeRegistry,
  deleteSessionByRef,
  isNoArgDefaultKernelLaunch,
  isKernelEndpointReachable,
  isKernelEndpointUnavailableError,
  bootstrapAttachedSession: bootstrapAttachedSessionWithRuntimeDeps,
  maybeResize: resizeSessionTerminal,
}

export async function bootstrapCliRuntime(
  options: CliRuntimeBootstrapOptions,
  deps: CliRuntimeBootstrapDeps = defaultCliRuntimeBootstrapDeps,
): Promise<CliRuntimeBootstrapResult> {
  const cliOptions = deps.parseArgs(options.argv)
  const preferences = await deps.loadPreferences()
  deps.applyProviderPreferenceDefaults(cliOptions, preferences)
  const kernelEndpoint = cliOptions.relayUrl
    ?? cliOptions.kernelUrl
    ?? cliOptions.socketPath
    ?? deps.defaultKernelEndpoint()
  const client = deps.createClient(kernelEndpoint, relayClientOptions(cliOptions))
  const inferredTargets = await deps.inferWorkspaceTargetsFromLaunchDirectory(options.cwd)
  const workspace = cliOptions.workspace ?? inferredTargets.workspace
  const worktree = cliOptions.worktree ?? inferredTargets.worktree
  await deps.primeWaitingRoomWorktreeInventory({
    cwd: options.cwd,
    workspacePath: workspace,
    currentWorktreePath: worktree,
  })
  const themeRegistry = await deps.loadThemeRegistry({
    workspace,
    onWarning: (warning) => {
      options.logger?.warn("skipping custom theme", warning)
    },
  })
  if (cliOptions.deleteSessionRef) {
    try {
      await deps.deleteSessionByRef(client, cliOptions.deleteSessionRef, workspace)
    } finally {
      await client.close()
    }
    return {
      kind: "deleted_session",
      client,
      options: cliOptions,
      kernelEndpoint,
      workspace,
      worktree,
    }
  }

  options.logger?.info("bootstrapping cli session", {
    kernel_endpoint: kernelEndpoint,
    workspace_id: workspace,
    worktree_id: worktree,
    client_id: cliOptions.clientId,
  })
  if (
    !cliOptions.detached
    && deps.isNoArgDefaultKernelLaunch(options.argv)
    && !(await deps.isKernelEndpointReachable(kernelEndpoint))
  ) {
    options.logger?.warn("default local kernel unavailable; launching detached waiting room", {
      kernel_endpoint: kernelEndpoint,
    })
    cliOptions.detached = true
  }

  let bootstrap: BootstrapState
  if (cliOptions.detached) {
    bootstrap = buildDetachedBootstrap(client, cliOptions, preferences)
  } else {
    try {
      bootstrap = await deps.bootstrapAttachedSession(
        client,
        cliOptions,
        workspace,
        worktree,
        preferences,
        options.logger,
      )
    } catch (error) {
      if (
        !deps.isNoArgDefaultKernelLaunch(options.argv)
        || !deps.isKernelEndpointUnavailableError(error)
      ) {
        throw error
      }
      options.logger?.warn("default local kernel unavailable; launching detached waiting room", {
        kernel_endpoint: kernelEndpoint,
        error: describeCliError(error),
      })
      cliOptions.detached = true
      bootstrap = buildDetachedBootstrap(client, cliOptions, preferences)
    }
  }
  bootstrap.themeRegistry = themeRegistry
  if (bootstrap.binding) {
    options.logger?.info("bootstrapped cli session", {
      session_id: bootstrap.binding.session.id,
      attachment_id: bootstrap.binding.attachment.id,
      created_session: bootstrap.binding.createdSession,
    })
    await deps.maybeResize(client, bootstrap.binding.session.id)
  } else {
    options.logger?.info("starting cli without attached session")
  }
  return {
    kind: "ready",
    bootstrap,
    kernelEndpoint,
    workspace,
    worktree,
  }
}

export function buildDetachedBootstrap(
  client: LocalIpcClient,
  options: CliOptions,
  preferences: CharioxPreferences,
): BootstrapState {
  return {
    client,
    binding: null,
    sessions: [],
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    providerCommandCatalogs: fallbackProviderCommandCatalogs({ catalogSource: "local_fallback" }),
    terminalCommandCatalog: null,
    options,
    preferences,
  }
}

async function bootstrapAttachedSessionWithRuntimeDeps(
  client: LocalIpcClient,
  options: CliOptions,
  workspace: string,
  worktree: string,
  preferences: CharioxPreferences,
  logger?: CharioxLogger | null,
): Promise<BootstrapState> {
  return bootstrapSession(client, options, workspace, worktree, preferences, {
    logger: logger ?? null,
    getConfiguredProviderLaunchDefaults: async (ipcClient) => {
      const { config } = await getUserConfig(ipcClient)
      const result: { provider?: string, model?: string, effort?: string } = {}
      if (config.providers?.default) {
        result.provider = config.providers.default
      }
      if (config.providers?.model) {
        result.model = config.providers.model
      }
      if (config.providers?.effort) {
        result.effort = config.providers.effort
      }
      return result
    },
    listSessions,
    getProviderCatalog,
    getProviderCommandCatalogs,
    getTerminalCommandCatalog,
    createSession,
    resolveSession,
    attachToSession,
    getSessionState,
    launchProviderRun,
    tryGetProviderRun,
    catchUpAttachedSession,
    getSessionHistoryOutline,
    getPromptInputHistory,
    resolveVisibleAgentId: (session, nextPreferences) => {
      const focusedAgentId = sessionFocusedAgentId(session)
      return selectResponsePaneAgents(
        session.agents,
        focusedAgentId,
        sessionResponseLayout(session, nextPreferences.ui?.multiAgentResponseLayout) === "split",
        resolveMaxAgentsPerScreen(nextPreferences.ui?.maxAgentsPerScreen),
      ).visibleTranscriptAgentId
    },
    prepareHistoryOutlineAgent: (agent, _session) => {
      return reindexTranscriptEntries(
        hydrateSessionHistoryOutlineAgentEntries(agent),
        0,
      )
    },
  })
}

function relayClientOptions(options: CliOptions): RelayClientOptions | undefined {
  if (!options.relayUrl) {
    return undefined
  }
  const relayOptions: RelayClientOptions = {}
  if (options.relayToken !== undefined) {
    relayOptions.relayAuthToken = options.relayToken
  }
  if (options.targetDaemonId !== undefined) {
    relayOptions.targetDaemonId = options.targetDaemonId
  }
  if (options.targetDaemonAlias !== undefined) {
    relayOptions.targetDaemonAlias = options.targetDaemonAlias
  }
  return relayOptions
}
