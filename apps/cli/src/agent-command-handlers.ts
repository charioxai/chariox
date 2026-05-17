import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import {
  parseAgentSpawnOptions,
  resolveLocalPlacement,
  type LocalGitWorktreeOptions,
  type RemoteGitWorktreePlacement,
} from "./command-worktree-placement.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
import {
  handleAgentSubstituteCommand,
  type AgentSubstituteCommandHandlerDeps,
} from "./agent-substitute-command-handlers.js"

const SESSION_AGENT_MODE_CONFIG_KEY = "agents.mode"
const SESSION_AGENT_PERMISSION_CONFIG_KEY = "agents.permissions"

type FooterTone = "info" | "error"

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

type AgentConfigUpdatePayload = {
  agent: AgentInstance
  session: RuntimeSession
}

export type AgentCommandHandlerDeps = AgentSubstituteCommandHandlerDeps & {
  currentWorktreeTarget: () => string
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  currentModelId: () => string
  currentVariantId: () => string
  currentProviderId: () => string
  focusedAgentId: () => string | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  maxAgentsPerScreen: () => number
  flashFooter: (message: string, tone: FooterTone) => void
  formatError: (error: unknown) => string
  prepareLocalGitWorktree?: (options: LocalGitWorktreeOptions) => Promise<string>
  updateAgentConfig?: (
    sessionId: string,
    agentId: string,
    options: {
      executionMode?: "build" | "plan" | null
      clearExecutionMode?: boolean
      permissionLevel?: "required" | "yolo" | null
      clearPermissionLevel?: boolean
    },
  ) => Promise<AgentConfigUpdatePayload>
  updateAgentProfile?: (
    sessionId: string,
    agentId: string,
    options: {
      provider?: string | null
      model?: string | null
      effort?: string | null
      clearEffort?: boolean
    },
  ) => Promise<AgentConfigUpdatePayload>
  aliasAgent?: (
    sessionId: string,
    agentId: string,
    alias: string,
  ) => Promise<AgentConfigUpdatePayload>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  rebuildTranscript: () => void
  cycleAgentFocus: () => Promise<AgentCyclePayload>
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  spawnAgent: (
    provider?: string | null,
    alias?: string,
    model?: string | null,
    effort?: string | null,
    worktreeId?: string,
    machineRef?: string,
    worktreePlacement?: RemoteGitWorktreePlacement | undefined,
    sliceRef?: string,
  ) => Promise<AgentSpawnPayload>
  destroyAgent: (agentId: string) => Promise<RuntimeSession>
  focusAgent: (agentId: string) => Promise<AgentFocusPayload>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
}

export async function handleCycleAgentFocus(
  deps: AgentCommandHandlerDeps,
): Promise<void> {
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

export async function handleAgentSlashCommand(
  deps: AgentCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "agent" }>,
): Promise<void> {
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
        const parsed = parseAgentSpawnOptions(spawnArgs)
        if (parsed.error) {
          deps.flashFooter(parsed.error, "error")
          return
        }
        const count = parseSpawnCount(parsed.positional[0])
        if (
          count !== null
          && (parsed.positional.length > 1 || parsed.directory || parsed.machineRef || parsed.sliceRef || parsed.gitWorktree || parsed.branch || parsed.fromRef)
        ) {
          deps.flashFooter("usage: /agent spawn <count>", "error")
          return
        }
        if (count !== null && parsed.positional.length === 1) {
          for (let index = 0; index < count; index += 1) {
            await spawnAndLaunchAgent(deps, {})
          }
          deps.flashFooter(
            `spawned ${count} agent${count === 1 ? "" : "s"} from session defaults`,
            "info",
          )
          return
        }

        const alias = parsed.positional[0]
        const model = parsed.positional[1]
        const provider = model ? deps.currentProviderId() : null
        const effort = model ? deps.currentVariantId() : null
        const remoteGitPlacement = parsed.machineRef && (parsed.gitWorktree || parsed.branch || parsed.fromRef)
          ? {
              target_directory: parsed.gitWorktree ?? null,
              branch: parsed.branch ?? null,
              from_ref: parsed.fromRef ?? null,
            }
          : undefined
        const worktreeId = await resolveLocalPlacement({
          directory: parsed.directory,
          gitWorktree: parsed.gitWorktree,
          branch: parsed.branch,
          fromRef: parsed.fromRef,
          machineRef: parsed.machineRef,
          label: "agent working directory",
        }, {
          baseDirectory: deps.currentWorktreeTarget(),
          prepareLocalGitWorktree: deps.prepareLocalGitWorktree,
        })
        const payload = await spawnAndLaunchAgent(deps, {
          provider,
          alias,
          model: model ?? null,
          effort,
          worktreeId,
          machineRef: parsed.machineRef,
          worktreePlacement: remoteGitPlacement,
          sliceRef: parsed.sliceRef,
        })
        const placement = parsed.sliceRef
          ? ` in slice:${parsed.sliceRef}`
          : parsed.machineRef
          ? ` on ${parsed.machineRef}${worktreeId ? ` in ${worktreeId}` : ""}`
          : worktreeId
            ? ` in ${worktreeId}`
            : ""
        deps.flashFooter(`spawned agent ${payload.agent.agent_ref}${alias ? ` (${alias})` : ""}${placement}`, "info")
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
    case "alias":
    case "name": {
      if (!deps.aliasAgent) {
        deps.flashFooter("agent aliases are unavailable in this build", "error")
        return
      }
      const reference = args[1]
      const explicitAliasArgs = args.length > 2 ? args.slice(2) : args.slice(1)
      const resolved = deps.resolveSessionAgent(args.length > 2 ? reference : deps.focusedAgentId() ?? undefined)
      if (!resolved.agent) {
        deps.flashFooter(resolved.error ?? "usage: /agent alias [agent-ref] <alias|clear>", "error")
        return
      }
      const rawAlias = explicitAliasArgs.join(" ").trim()
      if (!rawAlias) {
        deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} alias: ${resolved.agent.alias ?? "<none>"}`, "info")
        return
      }
      const shouldClearAgentAlias = rawAlias === "clear" || rawAlias === "none" || rawAlias === "-"
      try {
        const payload = await deps.aliasAgent(deps.sessionState().id, resolved.agent.id, shouldClearAgentAlias ? "" : rawAlias)
        deps.applySessionState(payload.session)
        await deps.refreshAgentPanes(payload.session)
        deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} alias: ${payload.agent.alias ?? "<none>"}`, "info")
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
      await handleCycleAgentFocus(deps)
      return
    }
    case "mode": {
      if (!deps.updateAgentConfig) {
        deps.flashFooter("agent config updates are unavailable in this build", "error")
        return
      }
      const reference = args[1]
      const rawValue = args[2] ?? (reference && (parseExecutionMode(reference) || reference === "inherit") ? reference : undefined)
      const resolved = deps.resolveSessionAgent(rawValue ? reference : deps.focusedAgentId() ?? undefined)
      if (!resolved.agent) {
        deps.flashFooter(resolved.error ?? "usage: /agent mode [agent-ref] <build|plan|inherit>", "error")
        return
      }
      if (!rawValue) {
        const effective = effectiveAgentExecutionMode(deps.sessionState(), resolved.agent)
        const source = resolved.agent.execution_mode_override ? "agent" : "session"
        deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} mode: ${effective} (${source})`, "info")
        return
      }
      if (rawValue !== "inherit" && !parseExecutionMode(rawValue)) {
        deps.flashFooter("usage: /agent mode [agent-ref] <build|plan|inherit>", "error")
        return
      }
      const payload = await deps.updateAgentConfig(deps.sessionState().id, resolved.agent.id, {
        executionMode: rawValue === "inherit" ? null : parseExecutionMode(rawValue),
        clearExecutionMode: rawValue === "inherit",
      })
      deps.applySessionState(payload.session)
      await deps.refreshAgentPanes(payload.session)
      const effective = effectiveAgentExecutionMode(payload.session, payload.agent)
      deps.flashFooter(
        `${deps.formatAgentLabel(payload.agent)} mode: ${effective}${rawValue === "inherit" ? " (session)" : " (agent)"}`,
        "info",
      )
      return
    }
    case "provider":
    case "model":
    case "variant": {
      if (!deps.updateAgentProfile) {
        deps.flashFooter("agent profile updates are unavailable in this build", "error")
        return
      }
      const reference = args[1]
      const rawValue = args.length > 2 ? args.slice(2).join(" ").trim() : args.slice(1).join(" ").trim()
      const resolved = deps.resolveSessionAgent(args.length > 2 ? reference : deps.focusedAgentId() ?? undefined)
      if (!resolved.agent) {
        deps.flashFooter(resolved.error ?? `usage: /agent ${subcommand} [agent-ref] <value>`, "error")
        return
      }
      if (!rawValue) {
        const value = subcommand === "provider"
          ? resolved.agent.provider
          : subcommand === "model"
            ? resolved.agent.model ?? "<none>"
            : resolved.agent.effort ?? "<none>"
        deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} ${subcommand}: ${value}`, "info")
        return
      }
      const shouldClearEffort = subcommand === "variant" && ["clear", "none", "-", "default"].includes(rawValue)
      const payload = await deps.updateAgentProfile(deps.sessionState().id, resolved.agent.id, {
        ...(subcommand === "provider" ? { provider: rawValue } : {}),
        ...(subcommand === "model" ? { model: rawValue } : {}),
        ...(subcommand === "variant" && !shouldClearEffort ? { effort: rawValue } : {}),
        ...(shouldClearEffort ? { clearEffort: true } : {}),
      })
      deps.applySessionState(payload.session)
      await deps.refreshAgentPanes(payload.session)
      const value = subcommand === "provider"
        ? payload.agent.provider
        : subcommand === "model"
          ? payload.agent.model ?? "<none>"
          : payload.agent.effort ?? "<none>"
      deps.flashFooter(`${deps.formatAgentLabel(payload.agent)} ${subcommand}: ${value}`, "info")
      return
    }
    case "permissions": {
      if (!deps.updateAgentConfig) {
        deps.flashFooter("agent config updates are unavailable in this build", "error")
        return
      }
      const reference = args[1]
      const rawValue = args[2] ?? (reference && (parsePermissionLevel(reference) || reference === "inherit") ? reference : undefined)
      const resolved = deps.resolveSessionAgent(rawValue ? reference : deps.focusedAgentId() ?? undefined)
      if (!resolved.agent) {
        deps.flashFooter(resolved.error ?? "usage: /agent permissions [agent-ref] <required|yolo|inherit>", "error")
        return
      }
      if (!rawValue) {
        const effective = effectiveAgentPermissionLevel(deps.sessionState(), resolved.agent)
        const source = resolved.agent.permission_level_override ? "agent" : "session"
        deps.flashFooter(`${deps.formatAgentLabel(resolved.agent)} permissions: ${effective} (${source})`, "info")
        return
      }
      if (rawValue !== "inherit" && !parsePermissionLevel(rawValue)) {
        deps.flashFooter("usage: /agent permissions [agent-ref] <required|yolo|inherit>", "error")
        return
      }
      const payload = await deps.updateAgentConfig(deps.sessionState().id, resolved.agent.id, {
        permissionLevel: rawValue === "inherit" ? null : parsePermissionLevel(rawValue),
        clearPermissionLevel: rawValue === "inherit",
      })
      deps.applySessionState(payload.session)
      await deps.refreshAgentPanes(payload.session)
      const effective = effectiveAgentPermissionLevel(payload.session, payload.agent)
      deps.flashFooter(
        `${deps.formatAgentLabel(payload.agent)} permissions: ${effective}${rawValue === "inherit" ? " (session)" : " (agent)"}`,
        "info",
      )
      return
    }
    case "substitute":
    case "subs": {
      await handleAgentSubstituteCommand(deps, args)
      return
    }
    default:
      deps.flashFooter(
        "usage: /agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--slice <slice-ref>] | /agent spawn <count> | delete [agent-name|agent-alias] | focus <agent-id> | alias [agent-ref] <alias|clear> | provider/model/variant [agent-ref] <value> | list | cycle | mode [agent-ref] <build|plan|inherit> | permissions [agent-ref] <required|yolo|inherit> | substitute ...",
        "error",
      )
  }
}

function parseExecutionMode(value: string | null | undefined): "build" | "plan" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "build" || normalized === "plan" ? normalized : null
}

function parsePermissionLevel(value: string | null | undefined): "required" | "yolo" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "required" || normalized === "yolo" ? normalized : null
}

function effectiveAgentExecutionMode(session: RuntimeSession, agent: AgentInstance | null | undefined): "build" | "plan" {
  return agent?.execution_mode_override
    ?? parseExecutionMode(session.config_state?.values?.[SESSION_AGENT_MODE_CONFIG_KEY])
    ?? parseExecutionMode(session.agent_defaults?.execution_mode)
    ?? "build"
}

function effectiveAgentPermissionLevel(session: RuntimeSession, agent: AgentInstance | null | undefined): "required" | "yolo" {
  return agent?.permission_level_override
    ?? parsePermissionLevel(session.config_state?.values?.[SESSION_AGENT_PERMISSION_CONFIG_KEY])
    ?? parsePermissionLevel(session.agent_defaults?.permission_level)
    ?? "yolo"
}

function parseSpawnCount(value: string | undefined): number | null {
  if (!value || !/^\d+$/.test(value)) {
    return null
  }
  const count = Number(value)
  return Number.isInteger(count) && count > 0 ? count : null
}

async function spawnAndLaunchAgent(
  deps: AgentCommandHandlerDeps,
  options: {
    provider?: string | null
    alias?: string | undefined
    model?: string | null
    effort?: string | null
    worktreeId?: string | undefined
    machineRef?: string | undefined
    worktreePlacement?: RemoteGitWorktreePlacement | undefined
    sliceRef?: string | undefined
  },
): Promise<AgentSpawnPayload> {
  const payload = await deps.spawnAgent(
    options.provider,
    options.alias,
    options.model,
    options.effort,
    options.worktreeId,
    options.machineRef,
    options.worktreePlacement,
    options.sliceRef,
  )
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  if (options.machineRef || options.sliceRef || payload.agent.remote_execution) {
    deps.setProviderRunState(null)
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
  const launchProvider = payload.agent.provider || options.provider || deps.currentProviderId()
  const launchModel = payload.agent.model || options.model || deps.currentModelId()
  const launchEffort = payload.agent.effort || options.effort || deps.currentVariantId()
  const run = await deps.launchAgentProviderRun(
    launchProvider,
    launchModel,
    launchEffort,
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

export function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]`)
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}
