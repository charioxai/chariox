import type {
  RuntimeSession,
  WorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowQueueCommandDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  applySessionState: (session: RuntimeSession) => void
  listWorkflowPromptQueues?: () => Promise<WorkflowPromptQueueDefinition[]>
  createWorkflowPromptQueue?: (alias: string, priority: number) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  updateWorkflowPromptQueue?: (
    queueRef: string,
    patch: { alias?: string | null; priority?: number | null; enabled?: boolean | null },
  ) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  removeWorkflowPromptQueue?: (queueRef: string) => Promise<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>
  listQueuedWorkflowPrompts?: () => Promise<WorkflowQueuedPrompt[]>
  updateQueuedWorkflowPrompt?: (
    queueItemRef: string,
    patch: { prompt?: string | null; queueRef?: string | null },
  ) => Promise<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>
  removeQueuedWorkflowPrompt?: (queueItemRef: string) => Promise<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>
  clearWorkflowPromptQueue?: (queueRef: string) => Promise<{ queued_prompts: WorkflowQueuedPrompt[]; session: RuntimeSession }>
}

export async function handleWorkflowQueueCommand(
  deps: WorkflowQueueCommandDeps,
  args: readonly string[],
): Promise<void> {
  const action = args[1]?.trim().toLowerCase() ?? "list"
  if (action === "list") {
    await listQueuesAndPrompts(deps)
    return
  }
  if (action === "create") {
    const alias = args[2]
    const priority = Number(args[3] ?? "0")
    if (!alias || !Number.isFinite(priority)) {
      deps.flashFooter("usage: /workflow queue create <alias> [priority]", "error")
      return
    }
    if (!deps.createWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.createWorkflowPromptQueue(alias, priority)
    deps.applySessionState(payload.session)
    deps.flashFooter(`created workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "priority") {
    const queueRef = args[2]
    const priority = Number(args[3])
    if (!queueRef || !Number.isFinite(priority)) {
      deps.flashFooter("usage: /workflow queue priority <queue-ref> <priority>", "error")
      return
    }
    if (!deps.updateWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateWorkflowPromptQueue(queueRef, { priority })
    deps.applySessionState(payload.session)
    deps.flashFooter(`updated workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "rename") {
    const queueRef = args[2]
    const alias = args[3]
    if (!queueRef || !alias) {
      deps.flashFooter("usage: /workflow queue rename <queue-ref> <alias>", "error")
      return
    }
    if (!deps.updateWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.updateWorkflowPromptQueue(queueRef, { alias })
    deps.applySessionState(payload.session)
    deps.flashFooter(`renamed workflow queue ${formatWorkflowPromptQueue(payload.queue)}`, "info")
    return
  }
  if (action === "remove") {
    const queueItemRef = args[2]
    if (!queueItemRef) {
      deps.flashFooter("usage: /workflow queue remove <queue-item-ref>", "error")
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
    const queueItemRef = args[2]
    const prompt = args.slice(3).join(" ").trim()
    if (!queueItemRef || !prompt) {
      deps.flashFooter("usage: /workflow queue edit <queue-item-ref> <prompt>", "error")
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
    const queueItemRef = args[2]
    const queueRef = args[3]
    if (!queueItemRef || !queueRef) {
      deps.flashFooter("usage: /workflow queue move <queue-item-ref> <queue-ref>", "error")
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
    const queueRef = args[2] ?? "default"
    if (!deps.clearWorkflowPromptQueue) {
      deps.flashFooter("workflow queue commands unavailable", "error")
      return
    }
    const payload = await deps.clearWorkflowPromptQueue(queueRef)
    deps.applySessionState(payload.session)
    deps.flashFooter(
      payload.queued_prompts.length === 0
        ? `workflow queue ${queueRef} already empty`
        : `cleared ${payload.queued_prompts.length} queued workflow prompt${payload.queued_prompts.length === 1 ? "" : "s"} from ${queueRef}`,
      "info",
    )
    return
  }
  deps.flashFooter("usage: /workflow queue [list|create|rename|priority|edit|move|remove|clear]", "error")
}

async function listQueuesAndPrompts(deps: WorkflowQueueCommandDeps): Promise<void> {
  if (!deps.listWorkflowPromptQueues || !deps.listQueuedWorkflowPrompts) {
    deps.flashFooter("workflow queue commands unavailable", "error")
    return
  }
  const [queues, prompts] = await Promise.all([
    deps.listWorkflowPromptQueues(),
    deps.listQueuedWorkflowPrompts(),
  ])
  const queueSummary = queues.map((queue) => {
    const depth = prompts.filter((prompt) => prompt.queue_id === queue.id).length
    return `${formatWorkflowPromptQueue(queue)} depth=${depth}`
  })
  deps.flashFooter(
    queueSummary.length === 0
      ? "workflow queues unavailable"
      : `workflow queues: ${queueSummary.join(", ")}${prompts.length ? `; prompts: ${prompts.map(formatWorkflowQueuedPrompt).join(", ")}` : ""}`,
    "info",
  )
}

export function formatWorkflowPromptQueue(queue: WorkflowPromptQueueDefinition): string {
  return `${queue.id}(${queue.alias}) priority=${queue.priority}${queue.enabled ? "" : " disabled"}`
}

export function formatWorkflowQueuedPrompt(prompt: WorkflowQueuedPrompt): string {
  const text = prompt.prompt && prompt.prompt.trim() !== ""
    ? ` prompt=${JSON.stringify(prompt.prompt.length > 50 ? `${prompt.prompt.slice(0, 50)}...` : prompt.prompt)}`
    : ""
  return `${prompt.id} [${prompt.source}] queue=${prompt.queue_id} endpoint=${prompt.endpoint_id} status=${prompt.status}${text}`
}
