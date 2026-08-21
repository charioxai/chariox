import type { RuntimeSession, SliceRecord } from "./cli-types.js"
import type {
  BackendProviderId,
  ProviderCatalog,
} from "./provider-catalog.js"
import { normalizeBackendProviderId } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomControlActivationDecision,
  deriveWaitingRoomCreateSessionDecision,
  type WaitingRoomActivationDecision,
  type WaitingRoomControlActivationDecision,
  type WaitingRoomCreateSessionDecision,
  type WaitingRoomLaunchConfig,
} from "./waiting-room-controller.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"
import { formatWorkspaceLiveSyncModeLabel } from "@chariox/kernel-client/workspace-live-sync-mode"
import type { ManagedEnvironmentDevelopmentSetup } from "@chariox/kernel-client/ipc-managed-environment-requests"
import { waitingRoomProjectDevelopmentSetup } from "./waiting-room-managed-environments.js"
import {
  waitingRoomExecutionMode,
  waitingRoomPermissionLevel,
} from "./waiting-room-state.js"

export type WaitingRoomCreateSessionLaunch = WaitingRoomLaunchConfig & {
  account_profile: string | null
  execution_mode: "build" | "plan"
  permission_level: "required" | "yolo"
  projectSelection?: WaitingRoomLaunchConfig["projectSelection"]
}

export type WaitingRoomPreparedManagedLaunch = {
  launch: WaitingRoomLaunchConfig
  assertActive: () => void
  commit: () => Promise<void>
  rollback: () => Promise<void>
}

export type WaitingRoomActivationControllerDeps = {
  isKernelConnected: () => boolean
  connectKernel: () => Promise<void>
  getWaitingRoomState: () => WaitingRoomState
  getRemoteState: () => WaitingRoomRemoteState
  getWorkspaceTarget: () => string
  getWorktreeTarget: () => string
  getAvailableSessions: () => SessionListEntry[]
  getProviderCatalog: () => ProviderCatalog
  getCurrentProvider: () => BackendProviderId
  getCurrentModel: () => string
  getAccountProfile: () => string | null | undefined
  handleCloudCommand: () => Promise<void>
  setPromptText: (text: string) => void
  focusPrompt: () => void
  syncCommandCenter: (text: string) => void
  openTerminalPairingDialog: () => Promise<void>
  openSessionBrowserDialog: () => void
  createSession: (
    workspacePath: string,
    worktreePath: string,
    launch: WaitingRoomCreateSessionLaunch,
  ) => Promise<Pick<RuntimeSession, "id"> & Partial<RuntimeSession>>
  deleteCreatedSession: (sessionId: string, workspacePath: string) => Promise<void>
  importExternalProviderSession?: (
    externalSessionId: string,
  ) => Promise<{ session: RuntimeSession; agent?: unknown; providerRun?: unknown }>
  loadOlderExternalProviderSessions?: () => Promise<number>
  browseKernelInventory?: (kernelId: string, machineId: string) => Promise<number>
  createSlice?: (options: {
    name: string
    displayMode: "headless" | "headed"
    workspaceId: string
    worktreeId: string
    workspaceMount: string
    developmentSetup?: ManagedEnvironmentDevelopmentSetup | null
    workerKernelRef?: string | null
  }) => Promise<SliceRecord>
  startSlice?: (sliceRef: string) => Promise<SliceRecord>
  updateSlices?: (slice: SliceRecord) => void
  prepareSessionOwnerClient?: (launch: WaitingRoomLaunchConfig) => Promise<void>
  prepareManagedSessionLaunch?: (
    launch: WaitingRoomLaunchConfig,
  ) => Promise<WaitingRoomPreparedManagedLaunch>
  prepareExistingSessionClient?: (session: SessionListEntry) => Promise<void>
  attachBinding: (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
    launch: WaitingRoomLaunchConfig,
  ) => Promise<void>
  rollbackAttachedSession: (sessionId: string) => Promise<unknown>
  flashFooter: (message: string, tone: "info" | "error") => void
  warn: (message: string, fields: Record<string, unknown>) => void
  formatError: (error: unknown) => string
  deriveControlDecision?: typeof deriveWaitingRoomControlActivationDecision
  deriveActivationDecision?: typeof deriveWaitingRoomActivationDecision
  deriveCreateSessionDecision?: typeof deriveWaitingRoomCreateSessionDecision
}

export function createWaitingRoomActivationController(
  deps: WaitingRoomActivationControllerDeps,
) {
  const deriveControlDecision = deps.deriveControlDecision ?? deriveWaitingRoomControlActivationDecision
  const deriveActivationDecision = deps.deriveActivationDecision ?? deriveWaitingRoomActivationDecision
  const deriveCreateSessionDecision = deps.deriveCreateSessionDecision ?? deriveWaitingRoomCreateSessionDecision

  const activate = async () => {
    try {
      if (!deps.isKernelConnected()) {
        await deps.connectKernel()
      }

      const remote = deps.getRemoteState()
      const controlDecision = deriveControlDecision({
        state: deps.getWaitingRoomState(),
        workspacePath: deps.getWorkspaceTarget(),
        worktreePath: deps.getWorktreeTarget(),
        remote,
      })
      if (await applyControlDecision(controlDecision)) {
        return
      }

      const decision = deriveActivationDecision({
        state: deps.getWaitingRoomState(),
        sessions: deps.getAvailableSessions(),
        catalog: deps.getProviderCatalog(),
        currentProvider: deps.getCurrentProvider(),
        currentModel: deps.getCurrentModel(),
        remote,
      })
      await applyActivationDecision(decision)
    } catch (error) {
      deps.warn("waiting room activation failed", {
        error: deps.formatError(error),
      })
      deps.flashFooter(deps.formatError(error), "error")
    }
  }

  const startSessionFromWaitingRoomDefaults = async () => {
    try {
      if (!deps.isKernelConnected()) {
        await deps.connectKernel()
      }

      const decision = deriveCreateSessionDecision({
        state: deps.getWaitingRoomState(),
        catalog: deps.getProviderCatalog(),
        currentProvider: deps.getCurrentProvider(),
        currentModel: deps.getCurrentModel(),
        remote: deps.getRemoteState(),
      })
      return await applyCreateSessionDecision(decision)
    } catch (error) {
      deps.warn("waiting room prompt bootstrap failed", {
        error: deps.formatError(error),
      })
      deps.flashFooter(deps.formatError(error), "error")
      throw error
    }
  }

  const applyControlDecision = async (
    decision: WaitingRoomControlActivationDecision,
  ) => {
    if (decision.action === "none") {
      return false
    }
    if (decision.action === "cloud") {
      await deps.handleCloudCommand()
      return true
    }
    if (decision.action === "browse-kernel") {
      if (!deps.browseKernelInventory) {
        deps.flashFooter("kernel inventory browsing is unavailable", "error")
        return true
      }
      const count = await deps.browseKernelInventory(decision.kernelId, decision.machineId)
      deps.flashFooter(
        `loaded ${count} session${count === 1 ? "" : "s"} from ${decision.label}`,
        "info",
      )
      return true
    }
    if (decision.action === "stage-command") {
      deps.setPromptText(decision.command)
      deps.focusPrompt()
      deps.syncCommandCenter(decision.command)
      deps.flashFooter(decision.message, "info")
      return true
    }
    if (decision.action === "open-terminal-pairing") {
      await deps.openTerminalPairingDialog()
      return true
    }
    if (decision.action === "open-session-browser") {
      deps.openSessionBrowserDialog()
      return true
    }
    if (decision.action === "load-older-external-sessions") {
      if (!deps.loadOlderExternalProviderSessions) {
        deps.flashFooter("unattached agent pagination is unavailable", "error")
        return true
      }
      const count = await deps.loadOlderExternalProviderSessions()
      deps.flashFooter(
        count > 0 ? `loaded ${count} older unattached agent${count === 1 ? "" : "s"}` : "no older unattached agents available",
        "info",
      )
      return true
    }
    deps.flashFooter(decision.message, decision.action === "error" ? "error" : "info")
    return true
  }

  const applyActivationDecision = async (
    decision: WaitingRoomActivationDecision,
  ) => {
    if (decision.action === "create") {
      await createAndAttachSession(decision.launch)
      return
    }
    if (decision.action === "join") {
      await deps.prepareExistingSessionClient?.(decision.session)
      await deps.attachBinding(decision.session, false, decision.launch)
      deps.flashFooter(`attached to session ${decision.session.alias ?? decision.session.id}`, "info")
      return
    }
    if (decision.action === "import-external-session") {
      if (!deps.importExternalProviderSession) {
        deps.flashFooter("unattached agent import is unavailable", "error")
        return
      }
      const imported = await deps.importExternalProviderSession(decision.externalSessionId)
      await deps.attachBinding(imported.session, true, {
        provider: imported.session.agent_defaults?.provider
          ? normalizeBackendProviderId(imported.session.agent_defaults.provider)
          : deps.getCurrentProvider(),
        model: imported.session.agent_defaults?.model ?? deps.getCurrentModel(),
        effort: imported.session.agent_defaults?.effort ?? "",
      })
      deps.flashFooter(`opened unattached agent ${decision.externalSessionId}`, "info")
      return
    }
    if (decision.action === "error") {
      deps.flashFooter(decision.message, "error")
    }
  }

  const applyCreateSessionDecision = async (
    decision: WaitingRoomCreateSessionDecision,
  ) => {
    if (decision.action === "error") {
      deps.flashFooter(decision.message, "error")
      throw new Error(decision.message)
    }
    return await createAndAttachSession(decision.launch)
  }

  const createAndAttachSession = async (launch: WaitingRoomLaunchConfig) => {
    const managedLaunch = Boolean(launch.managedEnvironment)
    const prepared = launch.managedEnvironment
      ? await deps.prepareManagedSessionLaunch?.(launch)
      : {
          launch,
          assertActive: () => {},
          commit: async () => {},
          rollback: async () => {},
        }
    if (!prepared) {
      throw new Error("managed session launch orchestration is unavailable in this build")
    }
    let session: (Pick<RuntimeSession, "id"> & Partial<RuntimeSession>) | null = null
    let workspacePath = ""
    try {
      const preparedLaunch = prepared.launch
      prepared.assertActive()
      if (!managedLaunch) {
        await deps.prepareSessionOwnerClient?.(preparedLaunch)
        prepared.assertActive()
      }
      const sliceRef = await prepareSliceForLaunch(preparedLaunch)
      prepared.assertActive()
      workspacePath = deps.getWorkspaceTarget()
      const worktreePath = deps.getWorktreeTarget()
      session = await deps.createSession(
        workspacePath,
        worktreePath,
        {
          provider: preparedLaunch.provider,
          model: preparedLaunch.model,
          effort: preparedLaunch.effort,
          account_profile: preparedLaunch.accountProfile ?? deps.getAccountProfile() ?? "default",
          execution_mode: waitingRoomExecutionMode(deps.getWaitingRoomState()),
          permission_level: waitingRoomPermissionLevel(deps.getWaitingRoomState()),
          workspaceLiveSyncMode: preparedLaunch.workspaceLiveSyncMode ?? "off",
          ...(preparedLaunch.projectSelection ? { projectSelection: preparedLaunch.projectSelection } : {}),
          ...(sliceRef ? { sliceRef } : {}),
        },
      )
      prepared.assertActive()
      await deps.attachBinding(session, true, preparedLaunch)
      prepared.assertActive()
      await prepared.commit()
      deps.flashFooter(createdSessionFooter(session, sliceRef), "info")
      return session
    } catch (error) {
      const cleanupErrors: string[] = []
      if (managedLaunch && session) {
        try {
          await deps.rollbackAttachedSession(session.id)
        } catch (cleanupError) {
          cleanupErrors.push(
            `failed to undo the cancelled session attachment ${session.id}: ${deps.formatError(cleanupError)}`,
          )
        }
        try {
          await deps.deleteCreatedSession(session.id, workspacePath)
        } catch (cleanupError) {
          cleanupErrors.push(
            `failed to remove cancelled session ${session.id}: ${deps.formatError(cleanupError)}`,
          )
        }
      }
      try {
        await prepared.rollback()
      } catch (rollbackError) {
        cleanupErrors.push(`failed to restore the source kernel connection: ${deps.formatError(rollbackError)}`)
      }
      if (cleanupErrors.length > 0) {
        throw new Error(`${deps.formatError(error)}; ${cleanupErrors.join("; ")}`)
      }
      throw error
    }
  }

  const prepareSliceForLaunch = async (launch: WaitingRoomLaunchConfig): Promise<string | null> => {
    if (launch.sliceRef) {
      if (deps.startSlice) {
        const slice = await deps.startSlice(launch.sliceRef)
        deps.updateSlices?.(slice)
      }
      return launch.sliceRef
    }
    if (!launch.sliceCreate) {
      return null
    }
    if (!deps.createSlice || !deps.startSlice) {
      throw new Error("slice creation is unavailable in this build")
    }
    const worktreePath = deps.getWorktreeTarget()
    const workspacePath = deps.getWorkspaceTarget()
    const waitingRoomState = deps.getWaitingRoomState()
    const developmentSetup = waitingRoomProjectDevelopmentSetup(
      waitingRoomState,
      {
        ...deps.getRemoteState(),
        workspaceId: workspacePath,
        worktreeId: worktreePath,
      },
    )
    if (waitingRoomState.managedDevelopmentMode === "current_project" && !developmentSetup) {
      throw new Error("Choose an existing Project and primary Workspace before using Current Project in a slice.")
    }
    const slice = await deps.createSlice({
      name: defaultSliceName(worktreePath),
      displayMode: launch.sliceCreate.displayMode,
      workspaceId: workspacePath,
      worktreeId: worktreePath,
      workspaceMount: worktreePath,
      ...(developmentSetup ? { developmentSetup } : {}),
      ...(launch.workerKernelRef && launch.workerKernelRef !== "local" ? { workerKernelRef: launch.workerKernelRef } : {}),
    })
    deps.updateSlices?.(slice)
    const started = await deps.startSlice(slice.id)
    deps.updateSlices?.(started)
    return started.id
  }

  return {
    activate,
    startSessionFromWaitingRoomDefaults,
  }
}

function defaultSliceName(worktreePath: string): string {
  const leaf = worktreePath.split("/").filter(Boolean).pop() || "workspace"
  const suffix = Date.now().toString(36).slice(-5)
  return `${leaf}-slice-${suffix}`.replace(/[^a-zA-Z0-9_.-]/g, "-")
}

function createdSessionFooter(
  session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>,
  sliceRef: string | null,
): string {
  const label = session.alias ?? session.id
  const worktree = session.worktree_id ? ` in ${session.worktree_id}` : ""
  const slice = sliceRef ? ` · slice ${sliceRef}` : ""
  return `created session ${label}${worktree}${slice} · workspace live sync ${createdSessionLiveSyncMode(session)}`
}

function createdSessionLiveSyncMode(session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>): string {
  return formatWorkspaceLiveSyncModeLabel(session.workspace_live_sync_mode)
}
