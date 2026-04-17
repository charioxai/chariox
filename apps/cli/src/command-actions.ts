import type {
  AgentInstance,
  ProviderAuthStatus,
  ProviderLoginStart,
  RuntimeAttachment,
  ProviderProcessInfo,
  QueuedWorkflowLaunch,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
  WorkflowEdgeDefinition,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { MultiAgentResponseLayout, UiPreferences } from "./preferences.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { SessionListEntry } from "./sessions.js"
import { readFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

const WORKFLOW_MAX_TURNS_CONFIG_KEY = "workflow.max_turns"
const WORKFLOW_LAUNCH_POLICY_CONFIG_KEY = "workflow.launch_policy"

type FooterTone = "info" | "error"

type CreateSessionResult = Pick<RuntimeSession, "id" | "alias">
type ResolveSessionResult = Pick<RuntimeSession, "id" | "alias">
type DeleteSessionResult = Pick<RuntimeSession, "id" | "alias">

type ResolvedAgentReference = {
  agent: AgentInstance | null
  error?: string
}

type AgentCyclePayload = {
  agent: AgentInstance | null
  session: RuntimeSession
}

type AgentFocusPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type AgentSpawnPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type WorkflowCreatePayload = {
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

type WorkflowEndpointPayload = {
  endpoint: WorkflowEndpointDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowNodePayload = {
  node: WorkflowNodeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowEdgePayload = {
  edge: WorkflowEdgeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowRunInvokePayload = {
  workflow: WorkflowDefinition
  endpoint: WorkflowEndpointDefinition
  session: RuntimeSession
} & ({ workflow_run: WorkflowRun } | { queued_launch: QueuedWorkflowLaunch })

type QueuedWorkflowLaunchPayload = {
  queued_launch: QueuedWorkflowLaunch
  session: RuntimeSession
}

type WorkflowRunCancelPayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

type WorkflowRunResumePayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

type WorkflowWatchdogPayload = {
  watchdog: WorkflowWatchdogDefinition
  workflow?: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition
  session: RuntimeSession
}

type CommandActionDeps = {
  workspace: string
  worktree: string
  accountProfile?: string | null
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  attachmentState: () => RuntimeAttachment | null
  providerRunState: () => RuntimeProviderRun | null
  currentModelId: () => string
  currentVariantId: () => string
  currentProviderId: () => string
  focusedAgentId: () => string | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  maxAgentsPerScreen: () => number
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  formatError: (error: unknown) => string
  createSession: (workspace: string, worktree: string, alias?: string) => Promise<CreateSessionResult>
  attachBinding: (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
  ) => Promise<void>
  resolveSession: (reference: string, workspace: string) => Promise<ResolveSessionResult>
  listSessions: () => Promise<RuntimeSession[]>
  deleteSessionByRef: (reference: string, workspace: string) => Promise<DeleteSessionResult>
  assignSessionAlias?: (sessionId: string, alias: string) => Promise<RuntimeSession>
  transitionToNoSession: (message: string) => void
  applyModelSelection: (value: string) => Promise<void>
  applyVariantSelection: (value: string) => Promise<void>
  applyProviderSelection?: (value: string) => Promise<void>
  getProviderAuthStatus?: (provider: string) => Promise<ProviderAuthStatus>
  startProviderLogin?: (provider: string) => Promise<ProviderLoginStart>
  logoutProvider?: (provider: string) => Promise<{ provider: string }>
  getRelayStatus?: () => Promise<{
    configured: boolean
    connected: boolean
    relay_url?: string | null
    relay_token_configured: boolean
    daemon_id: string
    machine_id: string
    machine_alias?: string | null
  }>
  configureRelay?: (relayUrl: string | null, relayToken: string | null) => Promise<{
    configured: boolean
    connected: boolean
    relay_url?: string | null
    relay_token_configured: boolean
    daemon_id: string
    machine_id: string
    machine_alias?: string | null
  }>
  refreshWaitingRoomData?: () => Promise<void>
  listRemoteMachines?: () => Promise<Array<{
    machine_id: string
    machine_alias?: string | null
    registry_alias?: string | null
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
    pending?: boolean
    kernel_count: number
    available_providers?: string[]
  }>>
  approveRemoteMachine?: (machineRef: string) => Promise<{
    machine_id: string
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
  }>
  forgetRemoteMachine?: (machineRef: string) => Promise<{
    machine_id: string
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
  }>
  renameRemoteMachine?: (machineRef: string, alias: string) => Promise<{
    machine_id: string
    display_name?: string
    trust_status?: "approved" | "pending" | "forgotten"
    online?: boolean
  }>
  listRemoteMachineKernels?: (machineRef: string) => Promise<Array<{
    kernel_id: string
    machine_id: string
    machine_alias?: string | null
    relay_alias?: string | null
    kernel_alias?: string | null
    available_providers?: string[]
    capabilities?: string[]
    accepting_remote_leases?: boolean
    leased_agent_count?: number
    local_session_count?: number
  }>>
  listProviderProcesses?: (provider?: string | null) => Promise<ProviderProcessInfo[]>
  teardownProviderProcesses?: (provider?: string | null) => Promise<ProviderProcessInfo[]>
  logViewCommand?: (fields: Record<string, unknown>) => void
  setMultiAgentResponseLayout: (layout: MultiAgentResponseLayout) => void
  applyResponseLayout: () => void
  updateSessionResponseLayout: (
    sessionId: string,
    attachmentId: string,
    layout: MultiAgentResponseLayout,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  updateSessionConfig: (
    sessionId: string,
    attachmentId: string,
    values: Record<string, string>,
    requiresIdle: boolean,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  saveUiPreferences: (prefs: UiPreferences) => Promise<void>
  rebuildTranscript: () => void
  requestRender: () => void
  afterViewRender?: (layout: MultiAgentResponseLayout) => void
  cycleAgentFocus: () => Promise<AgentCyclePayload>
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  spawnAgent: (provider: string, alias?: string, model?: string, effort?: string) => Promise<AgentSpawnPayload>
  destroyAgent: (agentId: string) => Promise<RuntimeSession>
  focusAgent: (agentId: string) => Promise<AgentFocusPayload>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  workflowScreenActive: () => boolean
  showWorkflowScreen: () => void
  selectedWorkflowId?: () => string | null
  selectWorkflowCanvas: (workflowId: string | null) => void
  replaceWorkflowDefinitions: (workflows: WorkflowDefinition[]) => void
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  createWorkflow: (alias?: string | null) => Promise<WorkflowCreatePayload>
  listWorkflows: () => Promise<WorkflowDefinition[]>
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  assignWorkflowAlias: (workflowId: string, alias: string) => Promise<WorkflowDefinition | null>
  createWorkflowEndpoint: (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => Promise<WorkflowEndpointPayload>
  assignWorkflowEndpointAlias: (
    workflowRef: string,
    endpointRef: string,
    alias: string,
  ) => Promise<WorkflowEndpointPayload>
  bindWorkflowEndpoint: (
    workflowRef: string,
    endpointRef: string,
    entryNodeId: string,
  ) => Promise<WorkflowEndpointPayload>
  addWorkflowNode: (workflowRef: string, agentId: string) => Promise<WorkflowNodePayload>
  removeWorkflowNode: (workflowRef: string, nodeId: string) => Promise<WorkflowNodePayload>
  addWorkflowEdge: (
    workflowRef: string,
    fromNodeId: string,
    toNodeId: string,
  ) => Promise<WorkflowEdgePayload>
  removeWorkflowEdge: (workflowRef: string, edgeId: string) => Promise<WorkflowEdgePayload>
  invokeWorkflowEndpoint?: (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
  ) => Promise<WorkflowRunInvokePayload>
  createWorkflowWatchdog?: (
    workflowRef: string,
    endpointRef: string,
    intervalSeconds: number,
    invocationPrompt: string,
    policy: "skip" | "queue",
    maxWakeups?: number | null,
  ) => Promise<WorkflowWatchdogPayload>
  listWorkflowWatchdogs?: (workflowRef?: string | null) => Promise<{ watchdogs: WorkflowWatchdogDefinition[] }>
  setWorkflowWatchdogEnabled?: (watchdogRef: string, enabled: boolean) => Promise<WorkflowWatchdogPayload>
  removeWorkflowWatchdog?: (watchdogRef: string) => Promise<WorkflowWatchdogPayload>
  setWorkflowFlushContext?: (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowLaunchPolicy?: (policy: "reject" | "queue") => Promise<{ session: RuntimeSession }>
  listQueuedWorkflowLaunches?: () => Promise<QueuedWorkflowLaunch[]>
  removeQueuedWorkflowLaunch?: (queueItemRef: string) => Promise<QueuedWorkflowLaunchPayload>
  clearQueuedWorkflowLaunches?: () => Promise<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>
  listWorkflowRuns?: (workflowRef?: string | null) => Promise<WorkflowRun[]>
  cancelWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunCancelPayload>
  resumeWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunResumePayload>
  updateWorkflowNodeInstructions?: (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeCanCompleteRun?: (
    workflowRef: string,
    nodeId: string,
    canCompleteWorkflowRun: boolean,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeCanEmitIntermediateOutput?: (
    workflowRef: string,
    nodeId: string,
    canEmitIntermediateWorkflowRunOutput: boolean,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeIntermediateOutputSchema?: (
    workflowRef: string,
    nodeId: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowNodeMaxTurns?: (
    workflowRef: string,
    nodeId: string,
    maxTurns: number | null,
  ) => Promise<WorkflowNodePayload>
  setWorkflowRunOutputSchema?: (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowIntermediateOutputSchema?: (
    workflowRef: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  openWorkflowNodeInstructionsEditor?: (workflowId: string, nodeId: string, draft: string) => void
  closeWorkflowNodeInstructionsEditor?: () => void
  getWorkflowNodeInstructionsDraft?: () => string
  getWorkflowNodeInstructionsContext?: () => { workflowId: string; nodeId: string } | null
  openWorkflowTerminalPanel?: (workflowId: string) => void
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
  formatSessionList: (sessions: SessionListEntry[], currentSessionId?: string) => string
}

export function createCommandActionHandlers(deps: CommandActionDeps) {
  const currentWorkflowLaunchPolicy = (): "reject" | "queue" => {
    const policy =
      deps.sessionState().workflow_launch_policy ??
      deps.sessionState().config_state?.values?.[WORKFLOW_LAUNCH_POLICY_CONFIG_KEY] ??
      "reject"
    return policy === "queue" ? "queue" : "reject"
  }

  const parseWatchdogIntervalSeconds = (value: string | undefined): number | null => {
    if (!value) return null
    const match = value.trim().toLowerCase().match(/^(\d+)(s|m|h|d)$/)
    if (!match) return null
    const amount = Number(match[1])
    const unit = match[2]
    if (!Number.isFinite(amount) || amount <= 0) return null
    const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
    return amount * multiplier
  }

  const formatQueuedWorkflowLaunch = (queuedLaunch: QueuedWorkflowLaunch): string =>
    [
      queuedLaunch.id,
      `[${queuedLaunch.source}]`,
      `workflow=${queuedLaunch.workflow_id}`,
      `endpoint=${queuedLaunch.endpoint_id}`,
      queuedLaunch.watchdog_id ? `watchdog=${queuedLaunch.watchdog_id}` : null,
      `queued_at=${queuedLaunch.queued_at_ms}`,
      queuedLaunch.invocation_prompt && queuedLaunch.invocation_prompt.trim() !== ""
        ? `prompt=${JSON.stringify(
            queuedLaunch.invocation_prompt.length > 50
              ? `${queuedLaunch.invocation_prompt.slice(0, 50)}...`
              : queuedLaunch.invocation_prompt,
          )}`
        : null,
    ]
      .filter((value): value is string => Boolean(value))
      .join(" ")
  const parseWatchdogMaxWakeups = (value: string | undefined): number | null | undefined => {
    if (value == null) return undefined
    const normalized = value.trim().toLowerCase()
    if (!normalized) return undefined
    if (normalized === "null" || normalized === "unbounded") return null
    const numeric = Number(normalized)
    if (!Number.isFinite(numeric) || numeric <= 0 || !Number.isInteger(numeric)) {
      return undefined
    }
    return numeric
  }
  const formatProviderProcessNotice = (process: ProviderProcessInfo): string => {
    const lines = [
      `${process.process_id} provider=${process.provider} pid=${process.pid ?? "-"} status=${process.status} mode=${process.endpoint_mode} safe=${String(process.teardown_safe)}`,
      `  provider sessions: ${process.provider_session_ids.join(",") || "-"}`,
      `  owner runs: ${process.owner_provider_run_ids.join(",") || "-"}`,
      `  owner sessions: ${process.owner_session_ids.join(",") || "-"}`,
      `  attached sessions: ${process.attached_session_ids.join(",") || "-"}`,
      `  active workflow runs: ${process.active_workflow_run_ids.join(",") || "-"}`,
    ]
    if (process.teardown_blockers.length > 0) {
      lines.push(`  blockers: ${process.teardown_blockers.join("; ")}`)
    }
    return lines.join("\n")
  }
  const hasDuplicateWorkflowEdge = (
    workflow: WorkflowDefinition,
    fromNodeId: string,
    toNodeId: string,
  ) => {
    return (workflow.edges ?? []).some((edge) => (
      edge.from_node_id === fromNodeId && edge.to_node_id === toNodeId
    ))
  }
  const workflowEdgeAddUsage = "usage: /workflow edge add [workflow-ref] <from-node-id|from-agent-ref> <to-node-id|to-agent-ref>"
  const selectedWorkflowRef = () => deps.selectedWorkflowId?.() ?? null
  const workflowRefOrSelected = (workflowRef: string | null | undefined) => workflowRef ?? selectedWorkflowRef()
  const firstWorkflowArgIsExplicit = (workflowRef: string | undefined) => (
    !selectedWorkflowRef() || isKnownWorkflowReference(workflowRef)
  )
  const isKnownWorkflowReference = (reference: string | undefined) => {
    if (!reference) {
      return false
    }
    if (reference === selectedWorkflowRef()) {
      return true
    }
    return (deps.sessionState().workflows ?? []).some((workflow) => (
      workflow.id === reference || workflow.alias === reference
    ))
  }

  const resolveWorkflowNodeReference = (
    workflow: WorkflowDefinition,
    workflowRef: string,
    reference: string,
  ): { nodeId: string } | { error: string } => {
    const nodes = workflow.nodes ?? []
    const nodeMatch = nodes.find((node) => node.id === reference)
    if (nodeMatch) {
      return { nodeId: nodeMatch.id }
    }

    const resolvedAgent = deps.resolveSessionAgent(reference)
    if (!resolvedAgent.agent) {
      if (resolvedAgent.error?.startsWith("multiple agents match")) {
        return { error: resolvedAgent.error }
      }
      return { nodeId: reference }
    }

    const matches = nodes.filter((node) => node.agent_id === resolvedAgent.agent?.id)
    if (matches.length === 1) {
      const [node] = matches
      return { nodeId: node?.id ?? reference }
    }
    if (matches.length > 1) {
      return {
        error: `agent '${reference}' maps to multiple nodes in workflow '${workflow.id}'; use explicit node ids`,
      }
    }
    return {
      error: `agent '${reference}' is not a node in workflow '${workflowRef}'; add it first with /workflow node add <workflow-ref> <agent-id>`,
    }
  }

  const addAllRemainingWorkflowNodes = async (workflowRef: string) => {
    const resolved = await deps.resolveWorkflow(workflowRef)
    deps.upsertWorkflowDefinition(resolved.workflow)

    const existingAgentIds = new Set((resolved.workflow.nodes ?? []).map((node) => node.agent_id))
    const agentsToAdd = deps.sessionState().agents.filter((agent) => !existingAgentIds.has(agent.id))
    if (agentsToAdd.length === 0) {
      deps.selectWorkflowCanvas(resolved.workflow.id)
      deps.flashFooter(`workflow ${resolved.workflow.id} already has nodes for all session agents`, "info")
      return
    }

    let latestWorkflow = resolved.workflow
    for (const agent of agentsToAdd) {
      const payload = await deps.addWorkflowNode(latestWorkflow.id, agent.id)
      latestWorkflow = payload.workflow
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
    }

    deps.selectWorkflowCanvas(latestWorkflow.id)
    deps.flashFooter(
      `added ${agentsToAdd.length} workflow node${agentsToAdd.length === 1 ? "" : "s"} for ${agentsToAdd.map((agent) => deps.formatAgentLabel(agent)).join(", ")}`,
      "info",
    )
  }

  const formatWorkflowRunSummary = (workflowRun: WorkflowRun) => {
    const failureSummary = (workflowRun.failure_events?.length ?? 0) > 0
      ? `, failures ${workflowRun.failure_events?.length ?? 0}`
      : ""
    return `${workflowRun.id} [${String(workflowRun.status).toLowerCase()}${failureSummary}]`
  }

  const parseSpawnCount = (value: string | undefined): number | null => {
    if (!value || !/^\d+$/.test(value)) {
      return null
    }
    const count = Number(value)
    return Number.isInteger(count) && count > 0 ? count : null
  }

  const spawnAndLaunchAgent = async (options: {
    provider: string
    alias?: string | undefined
    model: string
    effort: string
  }): Promise<AgentSpawnPayload> => {
    const payload = await deps.spawnAgent(options.provider, options.alias, options.model, options.effort)
    deps.applySessionState(payload.session)
    await deps.refreshAgentPanes(payload.session)
    const run = await deps.launchAgentProviderRun(
      options.provider,
      options.model,
      options.effort,
      payload.agent.id,
    )
    deps.setProviderRunState(run)
    const refreshedSession = await deps.refreshSessionState(payload.session.id)
    deps.applySessionState(refreshedSession)
    await deps.refreshAgentPanes(refreshedSession)
    deps.rebuildTranscript()
    deps.refreshSplitPaneFocusRepaint()
    return {
      agent: payload.agent,
      session: refreshedSession,
    }
  }

  const handleSessionCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "session" }>,
  ): Promise<boolean> => {
    const { action, value, args } = command

    switch (action) {
      case "create":
      case "new": {
        const session = await deps.createSession(deps.workspace, deps.worktree, value || undefined)
        await deps.attachBinding(session, true)
        deps.flashFooter(`attached to session ${session.alias ?? session.id}`, "info")
        return true
      }
      case "attach": {
        if (!value) {
          deps.flashFooter("usage: /session attach <ref>", "error")
          return true
        }
        const session = await deps.resolveSession(value, deps.workspace)
        await deps.attachBinding(session, false)
        deps.flashFooter(`attached to session ${session.alias ?? session.id}`, "info")
        return true
      }
      case "list":
      case "ls": {
        const sessions = await deps.listSessions()
        deps.appendNotice(deps.formatSessionList(sessions, deps.sessionState().id))
        deps.flashFooter(`listed ${sessions.length} session${sessions.length === 1 ? "" : "s"}`, "info")
        return true
      }
      case "delete": {
        const sessionRef = value || (deps.isAttached() ? deps.sessionState().id : "")
        if (!sessionRef) {
          deps.flashFooter("usage: /session delete <ref>", "error")
          return true
        }
        const deleted = await deps.deleteSessionByRef(sessionRef, deps.workspace)
        if (deps.isAttached() && deleted.id === deps.sessionState().id) {
          deps.transitionToNoSession(`Session ${deleted.alias ?? deleted.id} was deleted.`)
        } else {
          deps.flashFooter(`deleted session ${deleted.alias ?? deleted.id}`, "info")
        }
        return true
      }
      default: {
        if (!action) {
          return false
        }
        if (args.length !== 0) {
          deps.flashFooter("usage: /session <alias>", "error")
          return true
        }
        const alias = action
        if (!deps.isAttached()) {
          deps.flashFooter("attach to a session before setting an alias", "error")
          return true
        }
        if (!deps.assignSessionAlias) {
          deps.flashFooter("session aliases are unavailable in this build", "error")
          return true
        }
        const session = await deps.assignSessionAlias(deps.sessionState().id, alias)
        deps.applySessionState(session)
        deps.flashFooter(`session ${session.id} aliased as ${session.alias}`, "info")
        return true
      }
    }
  }

  const handleProviderCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "provider" }>,
  ): Promise<void> => {
    const { value } = command
    if (!value) {
      deps.flashFooter("usage: /provider <opencode|codex|status|login|logout|reauth|processes>", "error")
      return
    }
    const parts = value.split(/\s+/).filter(Boolean)
    const [action, maybeProvider] = parts
    if (action === "status") {
      const provider = maybeProvider ?? deps.currentProviderId()
      if (!deps.getProviderAuthStatus) {
        deps.flashFooter("provider status is not available in this daemon", "error")
        return
      }
      const status = await deps.getProviderAuthStatus(provider)
      const details = status.account_profile
        ? `${status.provider}: ${status.auth_state} as ${status.account_profile}`
        : `${status.provider}: ${status.auth_state}`
      deps.appendNotice(
        [
          details,
          status.detected_version ? `version ${status.detected_version}` : null,
          status.login_hint ?? null,
        ].filter(Boolean).join(" • "),
      )
      deps.flashFooter(details, "info")
      return
    }
    if (action === "login") {
      const provider = maybeProvider ?? deps.currentProviderId()
      if (!deps.startProviderLogin) {
        deps.flashFooter("provider login is not available in this daemon", "error")
        return
      }
      const login = await deps.startProviderLogin(provider)
      const message = [
        `${login.provider} login started`,
        login.user_code ? `code ${login.user_code}` : null,
        login.verification_url ?? login.auth_url ?? null,
      ].filter(Boolean).join(" • ")
      deps.appendNotice(message)
      deps.flashFooter(message, "info")
      return
    }
    if (action === "logout") {
      const provider = maybeProvider ?? deps.currentProviderId()
      if (!deps.logoutProvider) {
        deps.flashFooter("provider logout is not available in this daemon", "error")
        return
      }
      const loggedOut = await deps.logoutProvider(provider)
      const message = `${loggedOut.provider} logged out`
      deps.appendNotice(message)
      deps.flashFooter(message, "info")
      return
    }
    if (action === "reauth") {
      const provider = maybeProvider ?? deps.currentProviderId()
      if (!deps.logoutProvider || !deps.startProviderLogin) {
        deps.flashFooter("provider reauth is not available in this daemon", "error")
        return
      }
      await deps.logoutProvider(provider)
      const login = await deps.startProviderLogin(provider)
      const message = [
        `${login.provider} reauth started`,
        login.user_code ? `code ${login.user_code}` : null,
        login.verification_url ?? login.auth_url ?? null,
      ].filter(Boolean).join(" • ")
      deps.appendNotice(message)
      deps.flashFooter(message, "info")
      return
    }
    if (action === "processes") {
      if (parts[1] === "teardown") {
        if (!deps.teardownProviderProcesses) {
          deps.flashFooter("provider process teardown is not available in this daemon", "error")
          return
        }
        const provider = parts[2] ?? null
        const blocked = deps.listProviderProcesses
          ? (await deps.listProviderProcesses(provider)).filter((process) => !process.teardown_safe)
          : []
        const tornDown = await deps.teardownProviderProcesses(provider)
        if (tornDown.length === 0) {
          if (blocked.length > 0) {
            deps.appendNotice(`blocked provider processes:\n${blocked.map((process) => formatProviderProcessNotice(process)).join("\n")}`)
          }
          deps.flashFooter("no safe provider processes to tear down", "info")
          return
        }
        deps.appendNotice(
          tornDown.map((process) => formatProviderProcessNotice(process)).join("\n"),
        )
        if (blocked.length > 0) {
          deps.appendNotice(`skipped blocked provider processes:\n${blocked.map((process) => formatProviderProcessNotice(process)).join("\n")}`)
        }
        deps.flashFooter(`tore down ${tornDown.length} provider process(es)`, "info")
        return
      }
      if (!deps.listProviderProcesses) {
        deps.flashFooter("provider process inspection is not available in this daemon", "error")
        return
      }
      const provider = maybeProvider ?? null
      const processes = await deps.listProviderProcesses(provider)
      if (processes.length === 0) {
        deps.flashFooter("no daemon-tracked provider processes", "info")
        return
      }
      deps.appendNotice(
        processes.map((process) => formatProviderProcessNotice(process)).join("\n"),
      )
      deps.flashFooter(`listed ${processes.length} provider process(es)`, "info")
      return
    }
    if (value !== "opencode" && value !== "codex") {
      deps.flashFooter(`unknown provider: ${value}`, "error")
      return
    }
    if (deps.applyProviderSelection) {
      await deps.applyProviderSelection(value)
    } else {
      deps.flashFooter(`${value} selected`, "info")
    }
  }

  const handleModelCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "model" }>,
  ): Promise<void> => {
    const { value } = command
    if (!value) {
      deps.flashFooter("usage: /model <provider/model>", "error")
      return
    }
    await deps.applyModelSelection(value)
  }

  const handleVariantCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "variant" }>,
  ): Promise<void> => {
    const { value } = command
    if (!value) {
      deps.flashFooter("usage: /variant <name>", "error")
      return
    }
    await deps.applyVariantSelection(value)
  }

  const handleViewCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "view" }>,
  ): Promise<void> => {
    const selection = parseRequestedViewLayout(command.value, deps.multiAgentResponseLayout())
    if (selection.kind === "summary") {
      deps.flashFooter(
        `view: ${deps.multiAgentResponseLayout()} • agents: ${deps.sessionState().agents.length}`,
        "info",
      )
      return
    }
    if (selection.kind === "invalid") {
      deps.flashFooter("usage: /view <split|individual>", "error")
      return
    }
    const nextLayout = selection.layout
    deps.logViewCommand?.({
      requested_layout: nextLayout,
      previous_layout: deps.multiAgentResponseLayout(),
      attached: deps.isAttached(),
      agent_count: deps.sessionState().agents.length,
      focused_agent_id: deps.focusedAgentId(),
    })
    deps.setMultiAgentResponseLayout(nextLayout)
    deps.applyResponseLayout()
    if (deps.isAttached() && deps.attachmentState()) {
      const updated = await deps.updateSessionResponseLayout(
        deps.sessionState().id,
        deps.attachmentState()!.id,
        nextLayout,
      )
      deps.applySessionState(updated.session)
      await deps.refreshAgentPanes(updated.session)
    }
    await deps.saveUiPreferences({ multiAgentResponseLayout: nextLayout })
    deps.rebuildTranscript()
    deps.requestRender()
    deps.afterViewRender?.(nextLayout)
    deps.flashFooter(`view set to ${nextLayout} • ${deps.sessionState().agents.length} agents`, "info")
  }

  const handleCycleAgentFocus = async (): Promise<void> => {
    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to cycle agents", "error")
      return
    }
    try {
      const previousSession = deps.sessionState()
      const previousSelection = selectResponsePaneAgents(
        previousSession.agents,
        previousSession.focused_agent_id,
        deps.multiAgentResponseLayout() === "split",
        deps.maxAgentsPerScreen(),
      )
      const payload = await deps.cycleAgentFocus()
      const nextSession = payload.session
      const nextSelection = selectResponsePaneAgents(
        nextSession.agents,
        nextSession.focused_agent_id,
        deps.multiAgentResponseLayout() === "split",
        deps.maxAgentsPerScreen(),
      )
      const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
        || !responsePaneBindingsMatch(previousSelection, nextSelection)
      deps.applySessionState(nextSession)
      if (shouldRefreshPaneContents) {
        await deps.refreshAgentPanes(nextSession)
      }
      if (!nextSession.active_provider_run_id && payload.agent) {
        const run = await deps.launchAgentProviderRun(
          payload.agent.provider,
          payload.agent.model ?? deps.currentModelId(),
          deps.currentVariantId(),
          payload.agent.id,
        )
        deps.setProviderRunState(run)
        deps.applySessionState(await deps.refreshSessionState(nextSession.id))
      }
      if (payload.agent) {
        deps.flashFooter(
          `cycled to agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
          "info",
        )
      } else {
        deps.flashFooter("no agents to cycle", "info")
      }
    } catch (error) {
      deps.flashFooter(deps.formatError(error), "error")
    }
  }

  const handleAgentCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "agent" }>,
  ): Promise<void> => {
    const args = command.args
    const subcommand = args[0]

    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to manage agents", "error")
      return
    }

    switch (subcommand) {
      case "spawn": {
        const spawnArgs = args.slice(1)
        try {
          const count = parseSpawnCount(spawnArgs[0])
          if (count !== null && spawnArgs.length === 1) {
            const session = deps.sessionState()
            const sourceAgent = session.agents.find((agent) => agent.id === session.focused_agent_id)
              ?? session.agents[0]
              ?? null
            if (!sourceAgent) {
              deps.flashFooter("no focused agent to clone", "error")
              return
            }

            const provider = sourceAgent.provider ?? deps.currentProviderId()
            const model = sourceAgent.model ?? deps.currentModelId()
            const effort = deps.currentVariantId()
            for (let index = 0; index < count; index += 1) {
              await spawnAndLaunchAgent({
                provider,
                model,
                effort,
              })
            }
            deps.flashFooter(
              `spawned ${count} agent${count === 1 ? "" : "s"} from ${deps.formatAgentLabel(sourceAgent)}`,
              "info",
            )
            return
          }

          const alias = spawnArgs[0]
          const model = spawnArgs[1]
          const provider = deps.currentProviderId()
          const effort = deps.currentVariantId()
          const payload = await spawnAndLaunchAgent({
            provider,
            alias,
            model: model ?? deps.currentModelId(),
            effort,
          })
          deps.flashFooter(`spawned agent ${payload.agent.agent_ref}${alias ? ` (${alias})` : ""}`, "info")
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "delete":
      case "destroy": {
        const reference = args[1]
        const resolved = deps.resolveSessionAgent(reference)
        if (resolved.error || !resolved.agent) {
          deps.flashFooter(resolved.error ?? "usage: /agent delete <agent-name|agent-alias>", "error")
          return
        }
        try {
          const nextSession = await deps.destroyAgent(resolved.agent.id)
          deps.applySessionState(nextSession)
          await deps.refreshAgentPanes(nextSession)
          deps.rebuildTranscript()
          deps.refreshSplitPaneFocusRepaint()
          deps.flashFooter(`deleted agent ${deps.formatAgentLabel(resolved.agent)}`, "info")
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "focus": {
        const agentId = args[1]
        if (!agentId) {
          deps.flashFooter("usage: /agent focus <agent-id>", "error")
          return
        }
        try {
          const payload = await deps.focusAgent(agentId)
          const nextSession = payload.session
          const previousSession = deps.sessionState()
          const previousSelection = selectResponsePaneAgents(
            previousSession.agents,
            previousSession.focused_agent_id,
            deps.multiAgentResponseLayout() === "split",
            deps.maxAgentsPerScreen(),
          )
          const nextSelection = selectResponsePaneAgents(
            nextSession.agents,
            nextSession.focused_agent_id,
            deps.multiAgentResponseLayout() === "split",
            deps.maxAgentsPerScreen(),
          )
          const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
            || !responsePaneBindingsMatch(previousSelection, nextSelection)
          deps.applySessionState(nextSession)
          if (shouldRefreshPaneContents) {
            await deps.refreshAgentPanes(nextSession)
          }
          if (!nextSession.active_provider_run_id) {
            const run = await deps.launchAgentProviderRun(
              payload.agent.provider,
              payload.agent.model ?? deps.currentModelId(),
              deps.currentVariantId(),
              payload.agent.id,
            )
            deps.setProviderRunState(run)
            deps.applySessionState(await deps.refreshSessionState(nextSession.id))
          }
          deps.flashFooter(
            `focused on agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
            "info",
          )
        } catch (error) {
          deps.flashFooter(deps.formatError(error), "error")
        }
        return
      }
      case "list":
      case "ls": {
        deps.flashFooter(formatAgentListSummary(deps.sessionState().agents), "info")
        return
      }
      case "cycle": {
        await handleCycleAgentFocus()
        return
      }
      default:
        deps.flashFooter(
          "usage: /agent spawn [alias] [model] | /agent spawn <count> | delete [agent-name|agent-alias] | focus <agent-id> | list | cycle",
          "error",
        )
    }
  }


  const handleRelayCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "relay" }>,
  ): Promise<void> => {
    const [subcommand, ...args] = command.args
    if (!subcommand || subcommand === "status") {
      if (!deps.getRelayStatus) {
        deps.flashFooter("relay status is unavailable in this build", "error")
        return
      }
      const status = await deps.getRelayStatus()
      const state = !status.configured ? "not configured" : status.connected ? "connected" : "configured, disconnected"
      deps.appendNotice(
        `relay ${state}\nurl=${status.relay_url ?? "-"}\ntoken_configured=${String(status.relay_token_configured)}\ndaemon=${status.daemon_id}\nmachine=${status.machine_alias ?? status.machine_id}`,
      )
      deps.flashFooter(`relay ${state}`, "info")
      return
    }
    if (subcommand === "use" || subcommand === "configure") {
      if (!deps.configureRelay) {
        deps.flashFooter("relay configuration is unavailable in this build", "error")
        return
      }
      const relayUrl = args[0]
      const relayToken = args[1] ?? process.env.ARROBA_RELAY_TOKEN
      if (!relayUrl) {
        deps.flashFooter("usage: /relay use <ws-url> [token]", "error")
        return
      }
      if (!relayToken) {
        deps.flashFooter("relay token missing; pass it or set ARROBA_RELAY_TOKEN", "error")
        return
      }
      const status = await deps.configureRelay(relayUrl, relayToken)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(
        `relay configured: ${status.relay_url ?? relayUrl} (${status.connected ? "connected" : "connecting"})`,
        "info",
      )
      return
    }
    if (subcommand === "disable" || subcommand === "reset") {
      if (!deps.configureRelay) {
        deps.flashFooter("relay configuration is unavailable in this build", "error")
        return
      }
      await deps.configureRelay(null, null)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter("relay disabled", "info")
      return
    }
    deps.flashFooter("usage: /relay status | /relay use <ws-url> [token] | /relay disable", "error")
  }

  const handleMachineCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "machine" }>,
  ): Promise<void> => {
    const args = command.args
    const subcommand = args[0]
    if (subcommand === "list") {
      if (!deps.listRemoteMachines) {
        deps.flashFooter("remote machine discovery is unavailable in this build", "error")
        return
      }
      const machines = await deps.listRemoteMachines()
      if (machines.length === 0) {
        deps.flashFooter("no live remote machines available through relay", "info")
        return
      }
      deps.appendNotice(
        machines
          .map((machine) =>
            `${machine.display_name ?? machine.machine_alias ?? "-"} id=${machine.machine_id} status=${machine.trust_status ?? "pending"}${machine.online === false ? ",offline" : ""} kernels=${machine.kernel_count} providers=${(machine.available_providers ?? []).join(",") || "-"}`
          )
          .join("\n"),
      )
      deps.flashFooter(`listed ${machines.length} live remote machine(s)`, "info")
      return
    }
    if (subcommand === "kernels") {
      if (!deps.listRemoteMachineKernels) {
        deps.flashFooter("remote machine discovery is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      if (!machineRef) {
        deps.flashFooter("usage: /machine kernels <machine-ref>", "error")
        return
      }
      const kernels = await deps.listRemoteMachineKernels(machineRef)
      if (kernels.length === 0) {
        deps.flashFooter(`no live kernels found for machine ${machineRef}`, "info")
        return
      }
      deps.appendNotice(
        kernels
          .map((kernel) => {
            const displayName = kernel.relay_alias ?? kernel.kernel_alias ?? "-"
            const kernelAlias =
              kernel.kernel_alias && kernel.kernel_alias !== displayName
                ? ` kernel_alias=${kernel.kernel_alias}`
                : ""
            return `${displayName} id=${kernel.kernel_id}${kernelAlias} machine=${kernel.machine_alias ?? kernel.machine_id} providers=${(kernel.available_providers ?? []).join(",") || "-"} accepting_remote_leases=${String(kernel.accepting_remote_leases ?? false)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}`
          })
          .join("\n"),
      )
      deps.flashFooter(`listed ${kernels.length} live kernel(s) for ${machineRef}`, "info")
      return
    }
    if (subcommand === "approve") {
      if (!deps.approveRemoteMachine) {
        deps.flashFooter("remote machine registration is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      if (!machineRef) {
        deps.flashFooter("usage: /machine approve <machine-ref>", "error")
        return
      }
      const machine = await deps.approveRemoteMachine(machineRef)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(`approved remote machine ${machine.display_name ?? machine.machine_id}`, "info")
      return
    }
    if (subcommand === "forget") {
      if (!deps.forgetRemoteMachine) {
        deps.flashFooter("remote machine registration is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      if (!machineRef) {
        deps.flashFooter("usage: /machine forget <machine-ref>", "error")
        return
      }
      const machine = await deps.forgetRemoteMachine(machineRef)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(`forgot remote machine ${machine.display_name ?? machine.machine_id}`, "info")
      return
    }
    if (subcommand === "rename") {
      if (!deps.renameRemoteMachine) {
        deps.flashFooter("remote machine registration is unavailable in this build", "error")
        return
      }
      const machineRef = args[1]
      const alias = args.slice(2).join(" ").trim()
      if (!machineRef || !alias) {
        deps.flashFooter("usage: /machine rename <machine-ref> <alias>", "error")
        return
      }
      const machine = await deps.renameRemoteMachine(machineRef, alias)
      await deps.refreshWaitingRoomData?.()
      deps.flashFooter(`renamed remote machine ${machine.machine_id} to ${machine.display_name ?? alias}`, "info")
      return
    }
    deps.flashFooter("usage: /machine list | /machine kernels <machine-ref> | /machine approve <machine-ref> | /machine forget <machine-ref> | /machine rename <machine-ref> <alias>", "error")
  }

  const handleMcpCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "mcp" }>,
  ): Promise<void> => {
    const [action] = command.args
    if (!action || action === "list" || action === "ls") {
      deps.appendNotice("MCP registry commands are planned for M7. Use /mcp install|import|list|show|grant|revoke once the registry API is wired.")
      deps.flashFooter("MCP management is not wired yet", "info")
      return
    }
    deps.flashFooter("MCP management is not wired yet", "error")
  }

  const handleSkillCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "skill" }>,
  ): Promise<void> => {
    const [action] = command.args
    if (!action || action === "list" || action === "ls") {
      deps.appendNotice("Skill registry commands are planned for M7. Use /skill install|import|list|show|grant|revoke once the registry API is wired.")
      deps.flashFooter("skill management is not wired yet", "info")
      return
    }
    deps.flashFooter("skill management is not wired yet", "error")
  }

  const handleWorkflowCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "workflow" }>,
  ): Promise<void> => {
    if (!deps.isAttached()) {
      deps.flashFooter("must be attached to a session to manage workflows", "error")
      return
    }

    const args = command.args
    const subcommand = args[0]

    if (!subcommand) {
      const knownWorkflows = deps.sessionState().workflows ?? []
      if (knownWorkflows.length > 0) {
        if (!deps.workflowScreenActive()) {
          deps.selectWorkflowCanvas(knownWorkflows[0]?.id ?? null)
          deps.showWorkflowScreen()
        }
        return
      }

      if (!deps.workflowScreenActive()) {
        deps.showWorkflowScreen()
        return
      }

      const workflows = await deps.listWorkflows()
      if (workflows.length > 0) {
        deps.replaceWorkflowDefinitions(workflows)
        deps.selectWorkflowCanvas(workflows[0]?.id ?? null)
      } else {
        // If already on workflow screen but no workflows exist, create one
        const payload = await deps.createWorkflow(null)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.applySessionState(payload.session)
        deps.flashFooter(`created workflow ${payload.workflow.id}`, "info")
      }
      return
    }

    if (subcommand === "list") {
      const workflows = await deps.listWorkflows()
      deps.replaceWorkflowDefinitions(workflows)
      deps.flashFooter(
        workflows.length === 0
          ? "no workflows in workspace"
          : `workflows: ${workflows.map((workflow) => workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id).join(", ")}`,
        "info",
      )
      return
    }

    if (subcommand === "show") {
      const workflowRef = workflowRefOrSelected(args[1])
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow show [workflow-ref]", "error")
        return
      }
      const payload = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.flashFooter(
        `workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
        "info",
      )
      return
    }

    if (subcommand === "new") {
      const payload = await deps.createWorkflow(args[1] ?? null)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `created workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
        "info",
      )
      return
    }

    if (subcommand === "run" || subcommand === "start") {
      const firstArg = args[1]
      const explicitWorkflowRef = firstWorkflowArgIsExplicit(firstArg) ? firstArg : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const endpointRef = explicitWorkflowRef ? args[2] : firstArg
      const prompt = args.slice(explicitWorkflowRef ? 3 : 2).join(" ").trim()
      if (!workflowRef || !endpointRef) {
        deps.flashFooter("usage: /workflow run|start [workflow-ref] <endpoint-ref> [prompt]", "error")
        return
      }
      if (!deps.invokeWorkflowEndpoint) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.invokeWorkflowEndpoint(workflowRef, endpointRef, prompt || null)
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      if ("workflow_run" in payload) {
        deps.flashFooter(
          `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
          "info",
        )
      } else {
        deps.flashFooter(
          `queued workflow launch ${payload.queued_launch.id}; active workflow run in session`,
          "info",
        )
      }
      return
    }

    if (subcommand === "launch-policy") {
      const value = args[1]?.trim().toLowerCase()
      if (!value) {
        deps.flashFooter(`workflow launch policy: ${currentWorkflowLaunchPolicy()}`, "info")
        return
      }
      if (value !== "reject" && value !== "queue") {
        deps.flashFooter("usage: /workflow launch-policy <reject|queue>", "error")
        return
      }
      if (!deps.setWorkflowLaunchPolicy) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.setWorkflowLaunchPolicy(value)
      deps.applySessionState(payload.session)
      deps.flashFooter(`workflow launch policy set to ${value}`, "info")
      return
    }

    if (subcommand === "flush-context") {
      const firstArg = args[1]?.trim().toLowerCase()
      const selectedRef = selectedWorkflowRef()
      const firstArgIsValue = firstArg === "true" || firstArg === "false"
      const workflowRef = workflowRefOrSelected(firstArgIsValue ? null : args[1])
      const value = (firstArgIsValue ? args[1] : args[2])?.trim().toLowerCase()
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
        return
      }
      if (firstArgIsValue && !selectedRef) {
        deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
        return
      }
      const resolved = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(resolved.workflow)
      if (!value) {
        deps.flashFooter(
          `workflow ${resolved.workflow.id} flush-context: ${(resolved.workflow.flush_agent_context_before_run ?? true) ? "true" : "false"}`,
          "info",
        )
        return
      }
      if (value !== "true" && value !== "false") {
        deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
        return
      }
      if (!deps.setWorkflowFlushContext) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.setWorkflowFlushContext(
        resolved.workflow.id,
        value === "true",
      )
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.flashFooter(
        `workflow ${payload.workflow.id} flush-context set to ${payload.workflow.flush_agent_context_before_run ? "true" : "false"}`,
        "info",
      )
      return
    }

    if (subcommand === "run-output-schema") {
      const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[1]) ? args[1] : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const value = explicitWorkflowRef ? args[2] : args[1]
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow run-output-schema [workflow-ref] [schema-ref|none]", "error")
        return
      }
      const resolved = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(resolved.workflow)
      if (value === undefined) {
        deps.flashFooter(
          `workflow ${resolved.workflow.id} run-output-schema: ${resolved.workflow.run_output_schema_ref ?? "none"}`,
          "info",
        )
        return
      }
      if (!deps.setWorkflowRunOutputSchema) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const schemaRef = value.trim().toLowerCase() === "none" ? null : value
      const payload = await deps.setWorkflowRunOutputSchema(resolved.workflow.id, schemaRef)
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.flashFooter(
        `workflow ${payload.workflow.id} run-output-schema set to ${payload.workflow.run_output_schema_ref ?? "none"}`,
        "info",
      )
      return
    }

    if (subcommand === "intermediate-output-schema") {
      const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[1]) ? args[1] : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const value = explicitWorkflowRef ? args[2] : args[1]
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow intermediate-output-schema [workflow-ref] [schema-ref|none]", "error")
        return
      }
      const resolved = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(resolved.workflow)
      if (value === undefined) {
        deps.flashFooter(
          `workflow ${resolved.workflow.id} intermediate-output-schema: ${resolved.workflow.intermediate_output_schema_ref ?? "none"}`,
          "info",
        )
        return
      }
      if (!deps.setWorkflowIntermediateOutputSchema) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const schemaRef = value.trim().toLowerCase() === "none" ? null : value
      const payload = await deps.setWorkflowIntermediateOutputSchema(resolved.workflow.id, schemaRef)
      deps.applySessionState(payload.session)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.flashFooter(
        `workflow ${payload.workflow.id} intermediate-output-schema set to ${payload.workflow.intermediate_output_schema_ref ?? "none"}`,
        "info",
      )
      return
    }

    if (subcommand === "queue") {
      const action = args[1]?.trim().toLowerCase() ?? "list"
      if (action === "list") {
        if (!deps.listQueuedWorkflowLaunches) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const queuedLaunches = await deps.listQueuedWorkflowLaunches()
        deps.flashFooter(
          queuedLaunches.length === 0
            ? "workflow queue is empty"
            : `workflow queue: ${queuedLaunches.map(formatQueuedWorkflowLaunch).join(", ")}`,
          "info",
        )
        return
      }
      if (action === "flush") {
        if (!deps.clearQueuedWorkflowLaunches) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.clearQueuedWorkflowLaunches()
        deps.applySessionState(payload.session)
        deps.flashFooter(
          payload.queued_launches.length === 0
            ? "workflow queue already empty"
            : `cleared ${payload.queued_launches.length} queued workflow launch${payload.queued_launches.length === 1 ? "" : "es"}`,
          "info",
        )
        return
      }
      if (action === "remove") {
        const queueItemRef = args[2]
        if (!queueItemRef) {
          deps.flashFooter("usage: /workflow queue remove <queue-item-ref>", "error")
          return
        }
        if (!deps.removeQueuedWorkflowLaunch) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.removeQueuedWorkflowLaunch(queueItemRef)
        deps.applySessionState(payload.session)
        deps.flashFooter(`removed queued workflow launch ${payload.queued_launch.id}`, "info")
        return
      }
      deps.flashFooter("usage: /workflow queue [list|flush|remove <queue-item-ref>]", "error")
      return
    }

    if (subcommand === "max-turns") {
      const value = args[1]
      if (!value) {
        const current = deps.sessionState().config_state?.values?.[WORKFLOW_MAX_TURNS_CONFIG_KEY]
        const label = current && current.trim() !== "" ? current : "unset"
        deps.flashFooter(`workflow max turns: ${label}`, "info")
        return
      }
      if (!deps.attachmentState()) {
        deps.flashFooter("must be attached to set workflow max turns", "error")
        return
      }
      const normalized = value.trim().toLowerCase()
      const nextValue =
        normalized === "off" || normalized === "0"
          ? "0"
          : Number.isFinite(Number(normalized))
            ? String(Math.max(1, Math.floor(Number(normalized))))
            : null
      if (!nextValue) {
        deps.flashFooter("usage: /workflow max-turns <count|off>", "error")
        return
      }
      const payload = await deps.updateSessionConfig(
        deps.sessionState().id,
        deps.attachmentState()!.id,
        { [WORKFLOW_MAX_TURNS_CONFIG_KEY]: nextValue },
        false,
      )
      deps.applySessionState(payload.session)
      deps.flashFooter(
        nextValue === "0"
          ? "workflow max turns disabled"
          : `workflow max turns set to ${nextValue}`,
        "info",
      )
      return
    }

    if (subcommand === "runs") {
      if (!deps.listWorkflowRuns) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const workflowRef = args[1] ?? null
      const workflowRuns = await deps.listWorkflowRuns(workflowRef)
      deps.flashFooter(
        workflowRuns.length === 0
          ? (workflowRef ? `no workflow runs for ${workflowRef}` : "no workflow runs in session")
          : `workflow runs: ${workflowRuns.map(formatWorkflowRunSummary).join(", ")}`,
        "info",
      )
      return
    }

    if (subcommand === "cancel") {
      const workflowRunRef = args[1]
      if (!workflowRunRef) {
        deps.flashFooter("usage: /workflow cancel <run-ref>", "error")
        return
      }
      if (!deps.cancelWorkflowRun) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.cancelWorkflowRun(workflowRunRef)
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `cancelled workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
        "info",
      )
      return
    }

    if (subcommand === "resume") {
      const workflowRunRef = args[1]
      if (!workflowRunRef) {
        deps.flashFooter("usage: /workflow resume <run-ref>", "error")
        return
      }
      if (!deps.resumeWorkflowRun) {
        deps.flashFooter("workflow runtime commands unavailable", "error")
        return
      }
      const payload = await deps.resumeWorkflowRun(workflowRunRef)
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `resumed workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
        "info",
      )
      return
    }

    if (subcommand === "terminal") {
      const workflowRef = workflowRefOrSelected(args[1]) ?? deps.sessionState().workflows?.[0]?.id ?? null
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow terminal [workflow-ref]", "error")
        return
      }
      const payload = await deps.resolveWorkflow(workflowRef)
      deps.upsertWorkflowDefinition(payload.workflow)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.openWorkflowTerminalPanel?.(payload.workflow.id)
      deps.flashFooter(`opened workflow terminal for ${payload.workflow.id}`, "info")
      return
    }

    if (subcommand === "add" && args[1] === "node") {
      const explicitWorkflowRef = args.length >= 4 ? args[2] : null
      const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
      const target = explicitWorkflowRef ? args[3] : args[2]
      if (!workflowRef || target !== "all") {
        deps.flashFooter("usage: /workflow add node [workflow-ref] all", "error")
        return
      }
      await addAllRemainingWorkflowNodes(workflowRef)
      return
    }

    if (subcommand === "node") {
      const action = args[1]
      if (action === "add") {
        const explicitWorkflowRef = args.length >= 4 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const agentRef = explicitWorkflowRef ? args[3] : args[2]
        if (!workflowRef || !agentRef) {
          deps.flashFooter("usage: /workflow node add [workflow-ref] <agent-id|all>", "error")
          return
        }
        if (agentRef === "all") {
          await addAllRemainingWorkflowNodes(workflowRef)
          return
        }
        const resolvedAgent = deps.resolveSessionAgent(agentRef)
        if (!resolvedAgent.agent || resolvedAgent.error) {
          deps.flashFooter(resolvedAgent.error ?? `agent '${agentRef}' not found`, "error")
          return
        }
        const payload = await deps.addWorkflowNode(workflowRef, resolvedAgent.agent.id)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`added workflow node ${payload.node.id} for agent ${deps.formatAgentLabel(resolvedAgent.agent)}`, "info")
        return
      }
      if (action === "remove") {
        const explicitWorkflowRef = args.length >= 4 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        if (!workflowRef || !nodeId) {
          deps.flashFooter("usage: /workflow node remove [workflow-ref] <node-id>", "error")
          return
        }
        const payload = await deps.removeWorkflowNode(workflowRef, nodeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`removed workflow node ${payload.node.id}`, "info")
        return
      }
      if (action === "instructions") {
        const instructionsAction = args[2]
        if (!instructionsAction) {
          deps.flashFooter(
            "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
            "error",
          )
          return
        }
        if (instructionsAction === "close") {
          deps.closeWorkflowNodeInstructionsEditor?.()
          deps.flashFooter("closed node instructions editor", "info")
          return
        }
        if (instructionsAction === "save") {
          const context = deps.getWorkflowNodeInstructionsContext?.()
          if (!context || !deps.updateWorkflowNodeInstructions || !deps.getWorkflowNodeInstructionsDraft) {
            deps.flashFooter("no workflow node instructions editor is open", "error")
            return
          }
          const payload = await deps.updateWorkflowNodeInstructions(
            context.workflowId,
            context.nodeId,
            deps.getWorkflowNodeInstructionsDraft(),
          )
          deps.applySessionState(payload.session)
          deps.upsertWorkflowDefinition(payload.workflow)
          deps.closeWorkflowNodeInstructionsEditor?.()
          deps.flashFooter(`saved node instructions for ${payload.node.id}`, "info")
          return
        }
        const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[3]) ? args[3] : null
        const instructionsWorkflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[4] : args[3]
        const fileRef = explicitWorkflowRef ? args[5] : args[4]
        if (!instructionsWorkflowRef || !nodeId) {
          deps.flashFooter(
            "usage: /workflow node instructions show|set [workflow-ref] <node-id> [file]",
            "error",
          )
          return
        }
        const resolved = await deps.resolveWorkflow(instructionsWorkflowRef)
        deps.upsertWorkflowDefinition(resolved.workflow)
        const node = resolved.workflow.nodes?.find((entry) => entry.id === nodeId)
        if (!node) {
          deps.flashFooter(`workflow node ${nodeId} not found`, "error")
          return
        }
        if (instructionsAction === "show") {
          deps.openWorkflowNodeInstructionsEditor?.(
            resolved.workflow.id,
            node.id,
            node.instructions ?? "",
          )
          deps.selectWorkflowCanvas(resolved.workflow.id)
          deps.flashFooter(`opened node ${node.id} instructions in the I/O panel`, "info")
          return
        }
        if (instructionsAction !== "set") {
          deps.flashFooter(
            "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
            "error",
          )
          return
        }
        if (fileRef) {
          if (!deps.updateWorkflowNodeInstructions) {
            deps.flashFooter("workflow instructions unavailable", "error")
            return
          }
          const content = await readFile(resolvePath(deps.workspace, fileRef), "utf8")
          const payload = await deps.updateWorkflowNodeInstructions(resolved.workflow.id, node.id, content)
          deps.applySessionState(payload.session)
          deps.upsertWorkflowDefinition(payload.workflow)
          deps.flashFooter(`updated node instructions for ${payload.node.id}`, "info")
          return
        }
        deps.openWorkflowNodeInstructionsEditor?.(resolved.workflow.id, node.id, node.instructions ?? "")
        deps.selectWorkflowCanvas(resolved.workflow.id)
        deps.flashFooter("editing node instructions in the I/O panel; submit text then /workflow node instructions save", "info")
        return
      }
      if (action === "can-complete-run") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
        if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
          deps.flashFooter("usage: /workflow node can-complete-run [workflow-ref] <node-id> <true|false>", "error")
          return
        }
        if (!deps.setWorkflowNodeCanCompleteRun) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.setWorkflowNodeCanCompleteRun(workflowRef, nodeId, value === "true")
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} can-complete-run set to ${payload.node.can_complete_workflow_run ? "true" : "false"}`,
          "info",
        )
        return
      }
      if (action === "can-emit-intermediate-output") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
        if (!workflowRef || !nodeId || (value !== "true" && value !== "false")) {
          deps.flashFooter("usage: /workflow node can-emit-intermediate-output [workflow-ref] <node-id> <true|false>", "error")
          return
        }
        if (!deps.setWorkflowNodeCanEmitIntermediateOutput) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const payload = await deps.setWorkflowNodeCanEmitIntermediateOutput(workflowRef, nodeId, value === "true")
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} can-emit-intermediate-output set to ${payload.node.can_emit_intermediate_run_output ? "true" : "false"}`,
          "info",
        )
        return
      }
      if (action === "intermediate-output-schema") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !nodeId || value === undefined) {
          deps.flashFooter("usage: /workflow node intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none>", "error")
          return
        }
        if (!deps.setWorkflowNodeIntermediateOutputSchema) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        const schemaRef = value.trim().toLowerCase() === "none" ? null : value
        const payload = await deps.setWorkflowNodeIntermediateOutputSchema(workflowRef, nodeId, schemaRef)
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} intermediate-output-schema set to ${payload.node.intermediate_output_schema_ref ?? "none"}`,
          "info",
        )
        return
      }
      if (action === "max-turns") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const nodeId = explicitWorkflowRef ? args[3] : args[2]
        const value = (explicitWorkflowRef ? args[4] : args[3])?.trim().toLowerCase()
        if (!workflowRef || !nodeId || !value) {
          deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
          return
        }
        if (!deps.setWorkflowNodeMaxTurns) {
          deps.flashFooter("workflow runtime commands unavailable", "error")
          return
        }
        let maxTurns: number | null
        if (value === "none") {
          maxTurns = null
        } else {
          const parsed = Number.parseInt(value, 10)
          if (!Number.isFinite(parsed) || parsed <= 0) {
            deps.flashFooter("usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>", "error")
            return
          }
          maxTurns = parsed
        }
        const payload = await deps.setWorkflowNodeMaxTurns(workflowRef, nodeId, maxTurns)
        deps.applySessionState(payload.session)
        deps.upsertWorkflowDefinition(payload.workflow)
        deps.flashFooter(
          `workflow node ${payload.node.id} max-turns set to ${payload.node.max_turns ?? "none"}`,
          "info",
        )
        return
      }
      deps.flashFooter(
        "usage: /workflow node add [workflow-ref] <agent-id|all> | remove [workflow-ref] <node-id> | instructions ... | can-complete-run [workflow-ref] <node-id> <true|false> | can-emit-intermediate-output [workflow-ref] <node-id> <true|false> | intermediate-output-schema [workflow-ref] <node-id> <schema-ref|none> | max-turns [workflow-ref] <node-id> <count|none>",
        "error",
      )
      return
    }

    if (subcommand === "edge") {
      const action = args[1]
      if (action === "add") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = explicitWorkflowRef ?? selectedWorkflowRef()
        const fromRef = explicitWorkflowRef ? args[3] : args[2]
        const toRef = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !fromRef || !toRef) {
          deps.flashFooter(workflowEdgeAddUsage, "error")
          return
        }
        if (!explicitWorkflowRef && isKnownWorkflowReference(fromRef)) {
          deps.flashFooter(workflowEdgeAddUsage, "error")
          return
        }
        const resolvedWorkflow = await deps.resolveWorkflow(workflowRef)
        deps.upsertWorkflowDefinition(resolvedWorkflow.workflow)
        const fromNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, workflowRef, fromRef)
        if ("error" in fromNode) {
          deps.flashFooter(fromNode.error, "error")
          return
        }
        const toNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, workflowRef, toRef)
        if ("error" in toNode) {
          deps.flashFooter(toNode.error, "error")
          return
        }
        if (fromNode.nodeId === toNode.nodeId) {
          deps.flashFooter("workflow edges must connect two different nodes", "error")
          return
        }
        if (hasDuplicateWorkflowEdge(resolvedWorkflow.workflow, fromNode.nodeId, toNode.nodeId)) {
          deps.flashFooter("workflow edge already exists between those nodes", "error")
          return
        }
        const payload = await deps.addWorkflowEdge(workflowRef, fromNode.nodeId, toNode.nodeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`added workflow edge ${payload.edge.id}`, "info")
        return
      }
      if (action === "remove") {
        const explicitWorkflowRef = args.length >= 4 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const edgeId = explicitWorkflowRef ? args[3] : args[2]
        if (!workflowRef || !edgeId) {
          deps.flashFooter("usage: /workflow edge remove [workflow-ref] <edge-id>", "error")
          return
        }
        const payload = await deps.removeWorkflowEdge(workflowRef, edgeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`removed workflow edge ${payload.edge.id}`, "info")
        return
      }
      deps.flashFooter(
        `${workflowEdgeAddUsage} | remove [workflow-ref] <edge-id>`,
        "error",
      )
      return
    }

    if (subcommand === "endpoint") {
      const action = args[1]
      if (action === "new") {
        const explicitWorkflowRef = firstWorkflowArgIsExplicit(args[2]) ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const entryNodeId = explicitWorkflowRef ? args[3] : args[2]
        const alias = (explicitWorkflowRef ? args[4] : args[3]) ?? null
        if (!workflowRef || !entryNodeId) {
          deps.flashFooter(
            "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias]",
            "error",
          )
          return
        }
        const payload = await deps.createWorkflowEndpoint(workflowRef, entryNodeId, alias)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(`created workflow endpoint ${payload.endpoint.id}`, "info")
        return
      }
      if (action === "alias") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const endpointRef = explicitWorkflowRef ? args[3] : args[2]
        const alias = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !endpointRef || !alias) {
          deps.flashFooter(
            "usage: /workflow endpoint alias [workflow-ref] <endpoint-ref> <alias>",
            "error",
          )
          return
        }
        const payload = await deps.assignWorkflowEndpointAlias(workflowRef, endpointRef, alias)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(
          `workflow endpoint ${payload.endpoint.id} aliased as ${payload.endpoint.alias}`,
          "info",
        )
        return
      }
      if (action === "bind") {
        const explicitWorkflowRef = args.length >= 5 ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const endpointRef = explicitWorkflowRef ? args[3] : args[2]
        const entryNodeId = explicitWorkflowRef ? args[4] : args[3]
        if (!workflowRef || !endpointRef || !entryNodeId) {
          deps.flashFooter(
            "usage: /workflow endpoint bind [workflow-ref] <endpoint-ref> <entry-node-id>",
            "error",
          )
          return
        }
        const payload = await deps.bindWorkflowEndpoint(workflowRef, endpointRef, entryNodeId)
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow.id)
        deps.flashFooter(
          `workflow endpoint ${payload.endpoint.id} bound to node ${payload.endpoint.entry_node_id}`,
          "info",
        )
        return
      }
      deps.flashFooter(
        "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind [workflow-ref] <endpoint-ref> <entry-node-id>",
        "error",
      )
      return
    }

    if (subcommand === "watchdog") {
      const action = args[1]
      if (action === "add") {
        if (!deps.createWorkflowWatchdog) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const explicitWorkflowRef = args[4] === "every" ? args[2] : null
        const workflowRef = workflowRefOrSelected(explicitWorkflowRef)
        const endpointRef = explicitWorkflowRef ? args[3] : args[2]
        const everyLiteral = explicitWorkflowRef ? args[4] : args[3]
        const intervalLiteral = explicitWorkflowRef ? args[5] : args[4]
        const optionStartIndex = explicitWorkflowRef ? 6 : 5
        const hasPolicyArg = args[optionStartIndex] === "skip" || args[optionStartIndex] === "queue"
        const policy = (hasPolicyArg ? args[optionStartIndex] : "skip") as "skip" | "queue"
        const maxWakeupsKeyword = args[optionStartIndex + (hasPolicyArg ? 1 : 0)]
        const hasMaxWakeupsArg = maxWakeupsKeyword === "max-wakeups"
        const maxWakeupsLiteral = hasMaxWakeupsArg ? args[optionStartIndex + (hasPolicyArg ? 2 : 1)] : undefined
        const maxWakeups = hasMaxWakeupsArg ? parseWatchdogMaxWakeups(maxWakeupsLiteral) : undefined
        const prompt = args
          .slice(optionStartIndex + (hasPolicyArg ? 1 : 0) + (hasMaxWakeupsArg ? 2 : 0))
          .join(" ")
          .trim() || "Run the workflow exactly as instructed."
        if (!workflowRef || !endpointRef || everyLiteral !== "every") {
          deps.flashFooter(
            "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt]",
            "error",
          )
          return
        }
        const intervalSeconds = parseWatchdogIntervalSeconds(intervalLiteral)
        if (!intervalSeconds) {
          deps.flashFooter("watchdog interval must be like 30s, 5m, 1h, or 1d", "error")
          return
        }
        if (hasMaxWakeupsArg && maxWakeups === undefined) {
          deps.flashFooter("max-wakeups must be a positive integer or `null`", "error")
          return
        }
        const payload = await deps.createWorkflowWatchdog(
          workflowRef,
          endpointRef,
          intervalSeconds,
          prompt,
          policy,
          maxWakeups,
        )
        deps.applySessionState(payload.session)
        deps.selectWorkflowCanvas(payload.workflow?.id ?? workflowRef)
        deps.flashFooter(`created workflow watchdog ${payload.watchdog.id}`, "info")
        return
      }
      if (action === "list") {
        if (!deps.listWorkflowWatchdogs) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const workflowRef = args[2] ?? null
        const payload = await deps.listWorkflowWatchdogs(workflowRef)
        if (payload.watchdogs.length === 0) {
          deps.flashFooter("no workflow watchdogs configured", "info")
          return
        }
        deps.appendNotice(payload.watchdogs.map((watchdog) =>
          `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"} next=${new Date(watchdog.next_run_at_ms).toISOString()}${watchdog.pending_run ? " pending=true" : ""}`
        ).join("\n"))
        deps.flashFooter(`listed ${payload.watchdogs.length} workflow watchdog(s)`, "info")
        return
      }
      if (action === "enable" || action === "disable") {
        if (!deps.setWorkflowWatchdogEnabled) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const watchdogRef = args[2]
        if (!watchdogRef) {
          deps.flashFooter(`usage: /workflow watchdog ${action} <watchdog-ref>`, "error")
          return
        }
        const payload = await deps.setWorkflowWatchdogEnabled(watchdogRef, action === "enable")
        deps.applySessionState(payload.session)
        deps.flashFooter(
          `${action === "enable" ? "enabled" : "disabled"} workflow watchdog ${payload.watchdog.id}`,
          "info",
        )
        return
      }
      if (action === "remove") {
        if (!deps.removeWorkflowWatchdog) {
          deps.flashFooter("workflow watchdogs are unavailable in this build", "error")
          return
        }
        const watchdogRef = args[2]
        if (!watchdogRef) {
          deps.flashFooter("usage: /workflow watchdog remove <watchdog-ref>", "error")
          return
        }
        const payload = await deps.removeWorkflowWatchdog(watchdogRef)
        deps.applySessionState(payload.session)
        deps.flashFooter(`removed workflow watchdog ${payload.watchdog.id}`, "info")
        return
      }
      deps.flashFooter(
        "usage: /workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [max-wakeups <n|null>] [prompt] | list [workflow-ref] | enable <watchdog-ref> | disable <watchdog-ref> | remove <watchdog-ref>",
        "error",
      )
      return
    }

    const edgeFromRef = args[1]
    const edgeToRef = args[2]
    if (edgeFromRef && edgeToRef) {
      const resolvedWorkflow = await deps.resolveWorkflow(subcommand)
      deps.upsertWorkflowDefinition(resolvedWorkflow.workflow)
      const fromNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, subcommand, edgeFromRef)
      if ("error" in fromNode) {
        deps.flashFooter(fromNode.error, "error")
        return
      }
      const toNode = resolveWorkflowNodeReference(resolvedWorkflow.workflow, subcommand, edgeToRef)
      if ("error" in toNode) {
        deps.flashFooter(toNode.error, "error")
        return
      }
      if (fromNode.nodeId === toNode.nodeId) {
        deps.flashFooter("workflow edges must connect two different nodes", "error")
        return
      }
      if (hasDuplicateWorkflowEdge(resolvedWorkflow.workflow, fromNode.nodeId, toNode.nodeId)) {
        deps.flashFooter("workflow edge already exists between those nodes", "error")
        return
      }
      const payload = await deps.addWorkflowEdge(subcommand, fromNode.nodeId, toNode.nodeId)
      deps.applySessionState(payload.session)
      deps.selectWorkflowCanvas(payload.workflow.id)
      deps.showWorkflowScreen()
      deps.flashFooter(`added workflow edge ${payload.edge.id}`, "info")
      return
    }

    const alias = args[1]
    if (!alias) {
      deps.flashFooter(
        "usage: /workflow | /workflow list | /workflow show [workflow-ref] | /workflow new [alias] | /workflow run|start [workflow-ref] <endpoint-ref> [prompt] | /workflow max-turns <count|off> | /workflow run-output-schema [workflow-ref] [schema-ref|none] | /workflow intermediate-output-schema [workflow-ref] [schema-ref|none] | /workflow runs [workflow-ref] | /workflow cancel <run-ref> | /workflow resume <run-ref> | /workflow terminal [workflow-ref] | /workflow <workflow-ref> <alias> | /workflow <workflow-ref> <from-node-or-agent-ref> <to-node-or-agent-ref> | /workflow node ... | /workflow edge ... | /workflow endpoint ...",
        "error",
      )
      return
    }

    const workflow = await deps.assignWorkflowAlias(subcommand, alias)
    if (!workflow) {
      deps.flashFooter(`unknown workflow: ${subcommand}`, "error")
      return
    }
    deps.upsertWorkflowDefinition(workflow)
    deps.showWorkflowScreen()
    deps.flashFooter(`workflow ${workflow.id} aliased as ${workflow.alias}`, "info")
  }

  return {
    handleSessionCommand,
    handleProviderCommand,
    handleModelCommand,
    handleVariantCommand,
    handleViewCommand,
    handleCycleAgentFocus,
    handleAgentCommand,
    handleMachineCommand,
    handleRelayCommand,
    handleWorkflowCommand,
    handleMcpCommand,
    handleSkillCommand,
  }
}

export function parseRequestedViewLayout(
  value: string,
  currentLayout: MultiAgentResponseLayout,
):
  | { kind: "summary" }
  | { kind: "invalid" }
  | { kind: "set"; layout: MultiAgentResponseLayout } {
  const normalized = value.trim().toLowerCase()
  if (!normalized) {
    return { kind: "summary" }
  }
  if (normalized !== "split" && normalized !== "individual") {
    return { kind: "invalid" }
  }
  if (normalized === currentLayout) {
    return { kind: "set", layout: currentLayout }
  }
  return { kind: "set", layout: normalized }
}

export function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]`)
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}
