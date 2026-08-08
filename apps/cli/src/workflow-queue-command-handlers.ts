import type {
  RuntimeSession,
  WorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowQueueCommandDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  applySessionState: (session: RuntimeSession) => void
  selectedWorkflowId?: () => string | null
  listWorkflowPromptQueues?: (workflowRef?: string | null) => Promise<WorkflowPromptQueueDefinition[]>
  createWorkflowPromptQueue?: (workflowRef: string | null, alias: string, priority: number) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  updateWorkflowPromptQueue?: (
    workflowRef: string | null,
    queueRef: string,
    patch: { alias?: string | null; priority?: number | null; enabled?: boolean | null },
  ) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  removeWorkflowPromptQueue?: (workflowRef: string | null, queueRef: string) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  listQueuedWorkflowPrompts?: () => Promise<WorkflowQueuedPrompt[]>
  updateQueuedWorkflowPrompt?: (
    queueItemRef: string,
    patch: { prompt?: string | null; queueRef?: string | null },
  ) => Promise<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>
  removeQueuedWorkflowPrompt?: (queueItemRef: string) => Promise<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>
  clearWorkflowPromptQueue?: (workflowRef: string | null, queueRef: string) => Promise<{ queued_prompts: WorkflowQueuedPrompt[]; session: RuntimeSession }>
}

export async function handleWorkflowQueueCommand(
  deps: WorkflowQueueCommandDeps,
  args: readonly string[],
): Promise<void> {
  const parsed = parseWorkflowQueueArgs(args)
  if (parsed.error) {
    deps.flashFooter("usage: /workflow queue [--workflow <workflow-ref>] [list|create|rename|priority|enable|disable|delete|edit|move|remove|clear|flush]", "error")
    return
  }
  const workflowRef = parsed.workflowRef ?? currentWorkflowRef(deps)
  const action = parsed.args[1]?.trim().toLowerCase() ?? "list"
  if (action === "list") {
    await listQueuesAndPrompts(deps, workflowRef)
    return
  }
  if (action === "create") {
    const alias = parsed.args[2]
    const priority = Number(parsed.args[3] ?? "0")
    if (!alias || !Number.isFinite(priority)) {
      deps.flashFooter("usage: /workflow queue create [--workflow <workflow-ref>] <alias> [priority]", "error")
      return
    }
    if (!deps.createWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.createWorkflowPromptQueue(workflowRef, alias, priority)
    deps.applySessionState(payload.session)
    deps.flashFooter(`created workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "priority") {
    const queueRef = parsed.args[2]
    const priority = Number(parsed.args[3])
    if (!queueRef || !Number.isFinite(priority)) {
      deps.flashFooter("usage: /workflow queue priority [--workflow <workflow-ref>] <queue-ref> <priority>", "error")
      return
    }
    if (!deps.updateWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateWorkflowPromptQueue(workflowRef, queueRef, { priority })
    deps.applySessionState(payload.session)
    deps.flashFooter(`updated workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "rename") {
    const queueRef = parsed.args[2]
    const alias = parsed.args[3]
    if (!queueRef || !alias) {
      deps.flashFooter("usage: /workflow queue rename [--workflow <workflow-ref>] <queue-ref> <alias>", "error")
      return
    }
    if (!deps.updateWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateWorkflowPromptQueue(workflowRef, queueRef, { alias })
    deps.applySessionState(payload.session)
    deps.flashFooter(`renamed workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "enable" || action === "disable") {
    const queueRef = parsed.args[2]
    if (!queueRef) {
      deps.flashFooter(`usage: /workflow queue ${action} [--workflow <workflow-ref>] <queue-ref>`, "error")
      return
    }
    if (!deps.updateWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateWorkflowPromptQueue(workflowRef, queueRef, { enabled: action === "enable" })
    deps.applySessionState(payload.session)
    deps.flashFooter(`${action === "enable" ? "enabled" : "disabled"} workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "delete") {
    const queueRef = parsed.args[2]
    if (!queueRef) {
      deps.flashFooter("usage: /workflow queue delete [--workflow <workflow-ref>] <queue-ref>", "error")
      return
    }
    if (!deps.removeWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.removeWorkflowPromptQueue(workflowRef, queueRef)
    deps.applySessionState(payload.session)
    deps.flashFooter(`deleted workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "remove") {
    const queueItemRef = parsed.args[2]
    if (!queueItemRef) {
      deps.flashFooter("usage: /workflow queue remove [--workflow <workflow-ref>] <queue-item-ref>", "error")
      return
    }
    if (!deps.removeQueuedWorkflowPrompt) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.removeQueuedWorkflowPrompt(queueItemRef)
    deps.applySessionState(payload.session)
    deps.flashFooter(`removed queued workflow prompt ${payload.queued_prompt.id}`, "info")
    return
  }
  if (action === "edit") {
    const queueItemRef = parsed.args[2]
    const prompt = parsed.args.slice(3).join(" ").trim()
    if (!queueItemRef || !prompt) {
      deps.flashFooter("usage: /workflow queue edit [--workflow <workflow-ref>] <queue-item-ref> <prompt>", "error")
      return
    }
    if (!deps.updateQueuedWorkflowPrompt) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateQueuedWorkflowPrompt(queueItemRef, { prompt })
    deps.applySessionState(payload.session)
    deps.flashFooter(`updated queued workflow prompt ${payload.queued_prompt.id}`, "info")
    return
  }
  if (action === "move") {
    const queueItemRef = parsed.args[2]
    const queueRef = parsed.args[3]
    if (!queueItemRef || !queueRef) {
      deps.flashFooter("usage: /workflow queue move [--workflow <workflow-ref>] <queue-item-ref> <queue-ref>", "error")
      return
    }
    if (!deps.updateQueuedWorkflowPrompt) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateQueuedWorkflowPrompt(queueItemRef, { queueRef })
    deps.applySessionState(payload.session)
    deps.flashFooter(`moved queued workflow prompt ${payload.queued_prompt.id}`, "info")
    return
  }
  if (action === "flush" || action === "clear") {
    const queueRef = parsed.args[2] ?? "default"
    if (!deps.clearWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.clearWorkflowPromptQueue(workflowRef, queueRef)
    deps.applySessionState(payload.session)
    deps.flashFooter(
      payload.queued_prompts.length === 0
        ? `workflow queue ${queueRef} already empty`
        : `cleared ${payload.queued_prompts.length} queued workflow prompt${payload.queued_prompts.length === 1 ? "" : "s"} from ${queueRef}`,
      "info",
    )
    return
  }
  deps.flashFooter("usage: /workflow queue [list|create|rename|priority|enable|disable|delete|edit|move|remove|clear|flush]", "error")
}

async function listQueuesAndPrompts(deps: WorkflowQueueCommandDeps, workflowRef: string | null): Promise<void> {
  if (!deps.listWorkflowPromptQueues || !deps.listQueuedWorkflowPrompts) {
    deps.flashFooter("workflow queue commands unavailable", "error")
    return
  }
  const [queues, prompts] = await Promise.all([
    deps.listWorkflowPromptQueues(workflowRef),
    deps.listQueuedWorkflowPrompts(),
  ])
  const workflowIds = new Set(queues.map((queue) => queue.workflow_id))
  const workflowPrompts = prompts.filter((prompt) => workflowIds.has(prompt.workflow_id))
  const queueSummary = queues.map((queue) => {
    const depth = workflowPrompts.filter((prompt) => prompt.workflow_id === queue.workflow_id && prompt.queue_id === queue.id).length
    return `${formatWorkflowPromptQueue(queue)} depth=${depth}`
  })
  deps.flashFooter(
    queueSummary.length === 0
      ? "workflow queues unavailable"
      : `workflow queues: ${queueSummary.join(", ")}${workflowPrompts.length ? `; prompts: ${workflowPrompts.map(formatWorkflowQueuedPrompt).join(", ")}` : ""}`,
    "info",
  )
}

function currentWorkflowRef(deps: WorkflowQueueCommandDeps): string | null {
  return deps.selectedWorkflowId?.() ?? null
}

export function formatWorkflowPromptQueue(queue: WorkflowPromptQueueDefinition): string {
  return `${queue.id}(${queue.alias}) priority=${queue.priority}${queue.enabled ? "" : " disabled"}`
}

export function formatWorkflowQueuedPrompt(prompt: WorkflowQueuedPrompt): string {
  const text = prompt.prompt && prompt.prompt.trim() !== ""
    ? ` prompt=${JSON.stringify(prompt.prompt.length > 50 ? `${prompt.prompt.slice(0, 50)}...` : prompt.prompt)}`
    : ""
  return `${prompt.id} [${prompt.source}] workflow=${prompt.workflow_id} queue=${prompt.queue_id} endpoint=${prompt.endpoint_id} status=${prompt.status}${text}`
}

function parseWorkflowQueueArgs(args: readonly string[]): { args: string[]; workflowRef?: string; error?: boolean } {
  const normalized: string[] = []
  let workflowRef: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]
    if (value === undefined) {
      continue
    }
    if (value === "--workflow") {
      const next = args[index + 1]?.trim()
      if (!next) {
        return { args: normalized, error: true }
      }
      workflowRef = next
      index += 1
      continue
    }
    normalized.push(value)
  }
  return { args: normalized, ...(workflowRef ? { workflowRef } : {}) }
}
