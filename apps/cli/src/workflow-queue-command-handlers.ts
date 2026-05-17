import type {
  QueuedWorkflowLaunch,
  RuntimeSession,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type QueuedWorkflowLaunchPayload = {
  queued_launch: QueuedWorkflowLaunch
  session: RuntimeSession
}

export type WorkflowQueueCommandDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  applySessionState: (session: RuntimeSession) => void
  listQueuedWorkflowLaunches?: () => Promise<QueuedWorkflowLaunch[]>
  removeQueuedWorkflowLaunch?: (queueItemRef: string) => Promise<QueuedWorkflowLaunchPayload>
  clearQueuedWorkflowLaunches?: () => Promise<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>
}

export async function handleWorkflowQueueCommand(
  deps: WorkflowQueueCommandDeps,
  args: readonly string[],
): Promise<void> {
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
}

export function formatQueuedWorkflowLaunch(queuedLaunch: QueuedWorkflowLaunch): string {
  return [
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
}
