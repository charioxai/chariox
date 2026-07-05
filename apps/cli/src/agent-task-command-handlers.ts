import type {
  AgentInstance,
  MetaagentTask,
  RuntimeSession,
} from "./cli-types.js"
import type { ResolvedAgentReference } from "@arroba/kernel-client/session-agent-resolver"

type FooterTone = "info" | "error"

export type AgentTaskCommandHandlerDeps = {
  sessionState: () => RuntimeSession
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  formatError: (error: unknown) => string
  applySessionState: (session: RuntimeSession) => void
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
  updateMetaagentTask?: (
    sessionId: string,
    metaagentId: string,
    updates: { taskMarkdown?: string | null; planMarkdown?: string | null },
  ) => Promise<RuntimeSession>
  pauseMetaagentTask?: (sessionId: string, metaagentId: string) => Promise<RuntimeSession>
  resumeMetaagentTask?: (sessionId: string, metaagentId: string) => Promise<RuntimeSession>
  abortMetaagentTask?: (
    sessionId: string,
    metaagentId: string,
    reason?: string | null,
  ) => Promise<RuntimeSession>
}

export async function handleAgentTaskCommand(
  deps: AgentTaskCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const subcommand = args[1] ?? "show"
  switch (subcommand) {
    case "show":
    case "status": {
      const target = resolveMetaagentTarget(deps, args[2])
      if (!target.agent) {
        deps.flashFooter(target.error ?? "usage: /agent task show [agent-ref]", "error")
        return
      }
      deps.appendNotice(formatMetaagentTaskNotice(target.agent, findMetaagentTask(deps.sessionState(), target.agent.id)))
      deps.flashFooter(`showing task for ${deps.formatAgentLabel(target.agent)}`, "info")
      return
    }
    case "edit":
    case "set": {
      await updateTaskMarkdown(deps, args.slice(2), "task")
      return
    }
    case "plan": {
      await updateTaskMarkdown(deps, args.slice(2), "plan")
      return
    }
    case "pause": {
      await setTaskLifecycle(deps, args[2], "pause")
      return
    }
    case "resume": {
      await setTaskLifecycle(deps, args[2], "resume")
      return
    }
    case "abort": {
      await abortTask(deps, args.slice(2))
      return
    }
    default:
      deps.flashFooter(
        "usage: /agent task [show|edit|plan|pause|resume|abort] [agent-ref] [text]",
        "error",
      )
  }
}

async function updateTaskMarkdown(
  deps: AgentTaskCommandHandlerDeps,
  args: string[],
  field: "task" | "plan",
) {
  const { agent, rest, error } = resolveOptionalTargetWithText(deps, args)
  if (!agent) {
    deps.flashFooter(error ?? `usage: /agent task ${field === "task" ? "edit" : "plan"} [agent-ref] <text>`, "error")
    return
  }
  if (!deps.updateMetaagentTask) {
    deps.flashFooter("agent task runtime is unavailable", "error")
    return
  }
  const markdown = rest.join(" ").trim()
  if (!markdown) {
    deps.flashFooter(`usage: /agent task ${field === "task" ? "edit" : "plan"} [agent-ref] <text>`, "error")
    return
  }

  try {
    const session = await deps.updateMetaagentTask(
      deps.sessionState().id,
      agent.id,
      field === "task" ? { taskMarkdown: markdown } : { planMarkdown: markdown },
    )
    deps.applySessionState(session)
    deps.refreshSplitPaneFocusRepaint()
    deps.flashFooter(`updated ${field} for ${deps.formatAgentLabel(agent)}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

async function setTaskLifecycle(
  deps: AgentTaskCommandHandlerDeps,
  reference: string | undefined,
  action: "pause" | "resume",
) {
  const target = resolveMetaagentTarget(deps, reference)
  if (!target.agent) {
    deps.flashFooter(target.error ?? `usage: /agent task ${action} [agent-ref]`, "error")
    return
  }
  const lifecycleRequest = action === "pause" ? deps.pauseMetaagentTask : deps.resumeMetaagentTask
  if (!lifecycleRequest) {
    deps.flashFooter("agent task runtime is unavailable", "error")
    return
  }
  if (!findMetaagentTask(deps.sessionState(), target.agent.id)) {
    deps.flashFooter(`no task exists for ${deps.formatAgentLabel(target.agent)}`, "error")
    return
  }
  try {
    const session = await lifecycleRequest(deps.sessionState().id, target.agent.id)
    deps.applySessionState(session)
    deps.refreshSplitPaneFocusRepaint()
    deps.flashFooter(`${action === "pause" ? "paused" : "resumed"} task for ${deps.formatAgentLabel(target.agent)}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

async function abortTask(
  deps: AgentTaskCommandHandlerDeps,
  args: string[],
) {
  const { agent, rest, error } = resolveOptionalTargetWithText(deps, args)
  if (!agent) {
    deps.flashFooter(error ?? "usage: /agent task abort [agent-ref] [reason]", "error")
    return
  }
  if (!deps.abortMetaagentTask) {
    deps.flashFooter("agent task runtime is unavailable", "error")
    return
  }
  if (!findMetaagentTask(deps.sessionState(), agent.id)) {
    deps.flashFooter(`no task exists for ${deps.formatAgentLabel(agent)}`, "error")
    return
  }
  try {
    const session = await deps.abortMetaagentTask(deps.sessionState().id, agent.id, rest.join(" ").trim() || null)
    deps.applySessionState(session)
    deps.refreshSplitPaneFocusRepaint()
    deps.flashFooter(`aborted task for ${deps.formatAgentLabel(agent)}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

function resolveOptionalTargetWithText(
  deps: AgentTaskCommandHandlerDeps,
  args: string[],
): { agent: AgentInstance | null; rest: string[]; error?: string | null | undefined } {
  if (args.length === 0) {
    const target = resolveMetaagentTarget(deps)
    return { agent: target.agent, rest: [], error: target.error }
  }
  const first = args[0]
  if (first) {
    const resolved = deps.resolveSessionAgent(first)
    if (resolved.agent && isMetaModeAgent(resolved.agent)) {
      return { agent: resolved.agent, rest: args.slice(1) }
    }
    if (resolved.agent) {
      return {
        agent: null,
        rest: [],
        error: `${deps.formatAgentLabel(resolved.agent)} is not in meta mode`,
      }
    }
  }
  const target = resolveMetaagentTarget(deps)
  return { agent: target.agent, rest: args, error: target.error }
}

function resolveMetaagentTarget(
  deps: AgentTaskCommandHandlerDeps,
  reference?: string | null,
): { agent: AgentInstance | null; error?: string | null } {
  const resolved = deps.resolveSessionAgent(reference ?? deps.sessionState().focused_agent_id)
  if (resolved.error || !resolved.agent) {
    return { agent: null, error: resolved.error ?? "no agent in meta mode available" }
  }
  if (!isMetaModeAgent(resolved.agent)) {
    return {
      agent: null,
      error: `${deps.formatAgentLabel(resolved.agent)} is not in meta mode`,
    }
  }
  return { agent: resolved.agent }
}

function isMetaModeAgent(agent: AgentInstance): boolean {
  return Boolean(agent.meta_mode)
}

function findMetaagentTask(session: RuntimeSession, metaagentId: string): MetaagentTask | null {
  return (session.metaagent_tasks ?? []).find((task) => task.metaagent_id === metaagentId) ?? null
}

function formatMetaagentTaskNotice(agent: AgentInstance, task: MetaagentTask | null): string {
  if (!task) {
    return `Meta mode task for ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}: none`
  }
  const lines = [
    `Meta mode task for ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
    `status: ${task.status}`,
    `revision: ${task.revision}`,
    "",
    "task:",
    task.task_markdown.trim() || "(empty)",
    "",
    "plan:",
    task.plan_markdown.trim() || "(empty)",
  ]
  if (task.blocked_reason) {
    lines.push("", `blocked: ${task.blocked_reason}`)
  }
  if (task.aborted_reason) {
    lines.push("", `aborted: ${task.aborted_reason}`)
  }
  if (task.completion_summary) {
    lines.push("", `completed: ${task.completion_summary}`)
  }
  return lines.join("\n")
}
