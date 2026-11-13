import type { MachineAllocationCapability } from "./machine-allocation-api.js"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
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
import type { ContextDevelopmentSetup } from "@chariox/kernel-client/context-transfer-types"
import { waitingRoomProjectDevelopmentSetup } from "./waiting-room-context-selection.js"
import {
  waitingRoomExecutionMode,
  waitingRoomPermissionLevel,
} from "./waiting-room-state.js"
import { selectedProviderAccountStartBlockReason } from "./waiting-room-provider-accounts.js"

export type WaitingRoomCreateSessionLaunch = WaitingRoomLaunchConfig & {
  account_profile: string | null
  execution_mode: "build" | "plan"
  permission_level: "required" | "yolo"
  projectSelection?: WaitingRoomLaunchConfig["projectSelection"]
}

export type WaitingRoomActivationControllerDeps = {
  onLaunchRefused?: (message: string, retry: () => void) => void
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
  ) => Promise<RuntimeSession>
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
    developmentSetup?: ContextDevelopmentSetup | null
    workerKernelRef?: string | null
  }) => Promise<SliceRecord>
  startSlice?: (sliceRef: string) => Promise<SliceRecord>
  deleteSlice?: (sliceRef: string) => Promise<unknown>
  updateSlices?: (slice: SliceRecord) => void
  prepareSessionOwnerClient?: (launch: WaitingRoomLaunchConfig) => Promise<void>
  acquireMachine?: (capability: MachineAllocationCapability) => Promise<void>
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
      const detachedControl = deriveControlDecision({
        state: deps.getWaitingRoomState(),
        workspacePath: deps.getWorkspaceTarget(), worktreePath: deps.getWorktreeTarget(), remote: deps.getRemoteState(),
      })
      if (["cloud", "browse-kernel"].includes(detachedControl.action) && await applyControlDecision(detachedControl)) return
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
        count > 0 ? `loaded ${count} older saved conversation${count === 1 ? "" : "s"}` : "no older saved conversations available",
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
    if (decision.action === "acquire-machine") {
      await acquireMachine(decision.capability)
      return
    }
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
    if (decision.action === "acquire-machine") return await acquireMachine(decision.capability)
    return await createAndAttachSession(decision.launch)
  }

  const acquireMachine = async (capability: MachineAllocationCapability) => {
    if (!deps.acquireMachine) throw new Error("Machine acquisition is unavailable in this client.")
    await deps.acquireMachine(capability)
  }

  const createAndAttachSession = async (launch: WaitingRoomLaunchConfig) => {
    const inventoryStatus = deps.getRemoteState().inventoryStatus
    if (inventoryStatus === "loading" || inventoryStatus === "error") {
      throw new Error("Waiting for selected kernel inventory. Try again after it connects.")
    }
    let session: RuntimeSession | null = null
    let createdSliceRef: string | null = null
    let workspacePath = ""
    let projectBindingChecked = false
    let attachAttempted = false
    try {
      await deps.prepareSessionOwnerClient?.(launch)
      const accountBlock = launch.sliceRef ? null : selectedProviderAccountStartBlockReason(
        deps.getRemoteState().providerAccounts, launch.provider,
        launch.accountProfile ?? deps.getAccountProfile() ?? undefined,
        null, launch.model,
      )
      if (accountBlock) throw new Error(accountBlock)
      const sliceRef = await prepareSliceForLaunch(launch, (sliceRef) => {
        createdSliceRef = sliceRef
      })
      workspacePath = deps.getWorkspaceTarget()
      const worktreePath = deps.getWorktreeTarget()
      session = await deps.createSession(
        workspacePath,
        worktreePath,
        {
          provider: launch.provider,
          model: launch.model,
          effort: launch.effort,
          account_profile: launch.accountProfile ?? deps.getAccountProfile() ?? "default",
          execution_mode: waitingRoomExecutionMode(deps.getWaitingRoomState()),
          permission_level: waitingRoomPermissionLevel(deps.getWaitingRoomState()),
          workspaceLiveSyncMode: launch.workspaceLiveSyncMode ?? "off",
          ...(launch.projectSelection ? { projectSelection: launch.projectSelection } : {}),
          ...(sliceRef ? { sliceRef } : {}),
        },
      )
      if (launch.projectSelection) {
        projectBindingChecked = true
        if (launch.projectSelection.kind === "existing"
          && session.project_id !== launch.projectSelection.project_id) {
          throw new Error("The created session is attached to a different Project than the Waiting Room selection.")
        }
      }
      attachAttempted = true
      await deps.attachBinding(session, true, launch)
      deps.flashFooter(createdSessionFooter(session, sliceRef), "info")
      return session
    } catch (error) {
      const cleanupErrors: string[] = []
      if (session && projectBindingChecked) {
        if (attachAttempted) {
          try {
            await deps.rollbackAttachedSession(session.id)
          } catch (cleanupError) {
            cleanupErrors.push(
              `failed to undo the cancelled session attachment ${session.id}: ${deps.formatError(cleanupError)}`,
            )
          }
        }
        try {
          await deps.deleteCreatedSession(session.id, workspacePath)
        } catch (cleanupError) {
          cleanupErrors.push(
            `failed to remove unprepared session ${session.id}: ${deps.formatError(cleanupError)}`,
          )
        }
      }
      if (!session && createdSliceRef && deps.deleteSlice) {
        try {
          await deps.deleteSlice(createdSliceRef)
        } catch (cleanupError) {
          cleanupErrors.push(
            `failed to remove orphaned slice ${createdSliceRef}: ${deps.formatError(cleanupError)}`,
          )
        }
      }
      if (!session && error instanceof LocalIpcError && error.code === "local_transport_error"
        && cleanupErrors.length === 0) {
        deps.onLaunchRefused?.(deps.formatError(error), () => {
          void startSessionFromWaitingRoomDefaults().catch(() => {})
        })
      }
      if (cleanupErrors.length > 0) {
        throw new Error(`${deps.formatError(error)}; ${cleanupErrors.join("; ")}`)
      }
      throw error
    }
  }

  const prepareSliceForLaunch = async (
    launch: WaitingRoomLaunchConfig,
    created: (sliceRef: string) => void,
  ): Promise<string | null> => {
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
    created(slice.id)
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
  return `created session ${label}${worktree}${slice}`
}

function createdSessionLiveSyncMode(session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>): string {
  return formatWorkspaceLiveSyncModeLabel(session.workspace_live_sync_mode)
}
