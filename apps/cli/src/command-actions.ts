import type {
  AgentInstance,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionConfigState,
  WorkflowEdgeDefinition,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { SessionListEntry } from "./sessions.js"

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
  transitionToNoSession: (message: string) => void
  applyModelSelection: (value: string) => Promise<void>
  applyVariantSelection: (value: string) => Promise<void>
  logViewCommand?: (fields: Record<string, unknown>) => void
  setMultiAgentResponseLayout: (layout: MultiAgentResponseLayout) => void
  applyResponseLayout: () => void
  updateSessionResponseLayout: (
    sessionId: string,
    attachmentId: string,
    layout: MultiAgentResponseLayout,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  saveUiPreferences: (prefs: { multiAgentResponseLayout: MultiAgentResponseLayout }) => Promise<void>
  rebuildTranscript: () => void
  requestRender: () => void
  afterViewRender?: (layout: MultiAgentResponseLayout) => void
  cycleAgentFocus: () => Promise<AgentCyclePayload>
  launchAgentProviderRun: (
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  spawnAgent: (provider: string, alias?: string, model?: string) => Promise<AgentSpawnPayload>
  destroyAgent: (agentId: string) => Promise<RuntimeSession>
  focusAgent: (agentId: string) => Promise<AgentFocusPayload>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  workflowScreenActive: () => boolean
  showWorkflowScreen: () => void
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
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
  formatSessionList: (sessions: SessionListEntry[], currentSessionId?: string) => string
}

export function createCommandActionHandlers(deps: CommandActionDeps) {
  const handleSessionCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "session" }>,
  ): Promise<boolean> => {
    const { action, value } = command

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
      default:
        return false
    }
  }

  const handleProviderCommand = async (
    command: Extract<ParsedSlashCommand, { kind: "provider" }>,
  ): Promise<void> => {
    const { value } = command
    if (!value) {
      deps.flashFooter("usage: /provider opencode", "error")
      return
    }
    if (value !== "opencode") {
      deps.flashFooter(`unknown provider: ${value}`, "error")
      return
    }
    deps.flashFooter("OpenCode selected", "info")
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
        const alias = args[1]
        const model = args[2]
        const provider = deps.providerRunState()?.provider ?? "opencode"
        try {
          const payload = await deps.spawnAgent(provider, alias, model)
          deps.applySessionState(payload.session)
          await deps.refreshAgentPanes(payload.session)
          const run = await deps.launchAgentProviderRun(
            model ?? deps.currentModelId(),
            deps.currentVariantId(),
            payload.agent.id,
          )
          deps.setProviderRunState(run)
          const refreshedSession = await deps.refreshSessionState(payload.session.id)
          deps.applySessionState(refreshedSession)
          await deps.refreshAgentPanes(refreshedSession)
          deps.rebuildTranscript()
          deps.refreshSplitPaneFocusRepaint()
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
          "usage: /agent spawn [alias] [model] | delete [agent-name|agent-alias] | focus <agent-id> | list | cycle",
          "error",
        )
    }
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
      if (!deps.workflowScreenActive()) {
        deps.showWorkflowScreen()
      }
      return
    }

    if (subcommand === "list") {
      const workflows = await deps.listWorkflows()
      deps.flashFooter(
        workflows.length === 0
          ? "no workflows in workspace"
          : `workflows: ${workflows.map((workflow) => workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id).join(", ")}`,
        "info",
      )
      return
    }

    if (subcommand === "show") {
      const workflowRef = args[1]
      if (!workflowRef) {
        deps.flashFooter("usage: /workflow show <workflow-ref>", "error")
        return
      }
      const payload = await deps.resolveWorkflow(workflowRef)
      deps.flashFooter(
        `workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
        "info",
      )
      return
    }

    if (subcommand === "new") {
      const payload = await deps.createWorkflow(args[1] ?? null)
      deps.showWorkflowScreen()
      deps.applySessionState(payload.session)
      deps.flashFooter(
        `created workflow ${payload.workflow.id}${payload.workflow.alias ? ` (${payload.workflow.alias})` : ""}`,
        "info",
      )
      return
    }

    if (subcommand === "node") {
      const action = args[1]
      const workflowRef = args[2]
      if (action === "add") {
        const agentId = args[3]
        if (!workflowRef || !agentId) {
          deps.flashFooter("usage: /workflow node add <workflow-ref> <agent-id>", "error")
          return
        }
        const payload = await deps.addWorkflowNode(workflowRef, agentId)
        deps.applySessionState(payload.session)
        deps.flashFooter(`added workflow node ${payload.node.id} for agent ${agentId}`, "info")
        return
      }
      if (action === "remove") {
        const nodeId = args[3]
        if (!workflowRef || !nodeId) {
          deps.flashFooter("usage: /workflow node remove <workflow-ref> <node-id>", "error")
          return
        }
        const payload = await deps.removeWorkflowNode(workflowRef, nodeId)
        deps.applySessionState(payload.session)
        deps.flashFooter(`removed workflow node ${payload.node.id}`, "info")
        return
      }
      deps.flashFooter(
        "usage: /workflow node add <workflow-ref> <agent-id> | remove <workflow-ref> <node-id>",
        "error",
      )
      return
    }

    if (subcommand === "edge") {
      const action = args[1]
      const workflowRef = args[2]
      if (action === "add") {
        const fromNodeId = args[3]
        const toNodeId = args[4]
        if (!workflowRef || !fromNodeId || !toNodeId) {
          deps.flashFooter(
            "usage: /workflow edge add <workflow-ref> <from-node-id> <to-node-id>",
            "error",
          )
          return
        }
        const payload = await deps.addWorkflowEdge(workflowRef, fromNodeId, toNodeId)
        deps.applySessionState(payload.session)
        deps.flashFooter(`added workflow edge ${payload.edge.id}`, "info")
        return
      }
      if (action === "remove") {
        const edgeId = args[3]
        if (!workflowRef || !edgeId) {
          deps.flashFooter("usage: /workflow edge remove <workflow-ref> <edge-id>", "error")
          return
        }
        const payload = await deps.removeWorkflowEdge(workflowRef, edgeId)
        deps.applySessionState(payload.session)
        deps.flashFooter(`removed workflow edge ${payload.edge.id}`, "info")
        return
      }
      deps.flashFooter(
        "usage: /workflow edge add <workflow-ref> <from-node-id> <to-node-id> | remove <workflow-ref> <edge-id>",
        "error",
      )
      return
    }

    if (subcommand === "endpoint") {
      const action = args[1]
      const workflowRef = args[2]
      if (action === "new") {
        const entryNodeId = args[3]
        const alias = args[4] ?? null
        if (!workflowRef || !entryNodeId) {
          deps.flashFooter(
            "usage: /workflow endpoint new <workflow-ref> <entry-node-id> [alias]",
            "error",
          )
          return
        }
        const payload = await deps.createWorkflowEndpoint(workflowRef, entryNodeId, alias)
        deps.applySessionState(payload.session)
        deps.flashFooter(`created workflow endpoint ${payload.endpoint.id}`, "info")
        return
      }
      if (action === "alias") {
        const endpointRef = args[3]
        const alias = args[4]
        if (!workflowRef || !endpointRef || !alias) {
          deps.flashFooter(
            "usage: /workflow endpoint alias <workflow-ref> <endpoint-ref> <alias>",
            "error",
          )
          return
        }
        const payload = await deps.assignWorkflowEndpointAlias(workflowRef, endpointRef, alias)
        deps.applySessionState(payload.session)
        deps.flashFooter(
          `workflow endpoint ${payload.endpoint.id} aliased as ${payload.endpoint.alias}`,
          "info",
        )
        return
      }
      if (action === "bind") {
        const endpointRef = args[3]
        const entryNodeId = args[4]
        if (!workflowRef || !endpointRef || !entryNodeId) {
          deps.flashFooter(
            "usage: /workflow endpoint bind <workflow-ref> <endpoint-ref> <entry-node-id>",
            "error",
          )
          return
        }
        const payload = await deps.bindWorkflowEndpoint(workflowRef, endpointRef, entryNodeId)
        deps.applySessionState(payload.session)
        deps.flashFooter(
          `workflow endpoint ${payload.endpoint.id} bound to node ${payload.endpoint.entry_node_id}`,
          "info",
        )
        return
      }
      deps.flashFooter(
        "usage: /workflow endpoint new <workflow-ref> <entry-node-id> [alias] | alias <workflow-ref> <endpoint-ref> <alias> | bind <workflow-ref> <endpoint-ref> <entry-node-id>",
        "error",
      )
      return
    }

    const alias = args[1]
    if (!alias) {
      deps.flashFooter(
        "usage: /workflow | /workflow list | /workflow show <workflow-ref> | /workflow new [alias] | /workflow <workflow-ref> <alias> | /workflow node ... | /workflow edge ... | /workflow endpoint ...",
        "error",
      )
      return
    }

    const workflow = await deps.assignWorkflowAlias(subcommand, alias)
    if (!workflow) {
      deps.flashFooter(`unknown workflow: ${subcommand}`, "error")
      return
    }
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
    handleWorkflowCommand,
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
