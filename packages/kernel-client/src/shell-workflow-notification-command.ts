/** MP-08 / MP-10: one shared attach surface for TUI and shell clients. */
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import type { WorkflowNotificationsResponse, WorkflowNotificationAttachedResponse, WorkflowNotificationSourceRegisteredResponse, WorkflowNotificationSourceSummary, WorkflowPublicationDefinition } from "./kernel-types.js"
import { attachWorkflowNotificationRequest, detachWorkflowNotificationRequest, listWorkflowNotificationsRequest, registerWorkflowNotificationSourceRequest } from "./ipc-workflow-notification-requests.js"
import { listWorkflowPublicationsRequest } from "./ipc-requests.js"
import { notificationKernelLabels } from "./notification-kernel-labels.js"
type Deps = { client: { send(request: Record<string, unknown>): Promise<Record<string, unknown>> } }

/** `/workflow notifications on|off <workflow> <exit>`: one exit of a workflow is one source. */
export async function executeWorkflowNotificationSettings(args: string[], context: ShellContext, deps: Deps): Promise<ShellCommandResult> {
  const usage = "usage: workflow notifications on|off <workflow> <exit>  (an exit node id, or exit-N)"
  const [state, workflow, exit, ...extra] = args
  if ((state !== "on" && state !== "off") || !workflow || !exit || extra.length > 0) return { ok: false, message: usage }
  if (!context.sessionId) return { ok: false, message: "select a session first" }
  const response = await deps.client.send(registerWorkflowNotificationSourceRequest(context.sessionId, workflow, exit, state === "on"))
  const payload = response as WorkflowNotificationSourceRegisteredResponse
  if (!payload.WorkflowNotificationSourceRegistered) throw new Error("unexpected notification source response")
  const source = payload.WorkflowNotificationSourceRegistered.source
  return { ok: true, message: `Notify when ${source.display_name} finishes: ${state}`, data: payload.WorkflowNotificationSourceRegistered }
}

/** A source is named by its display name, `kernel/display name` or its id. */
export function matchNotificationSource(sources: WorkflowNotificationSourceSummary[], ref: string): WorkflowNotificationSourceSummary[] {
  return sources.filter(s => s.source_id === ref || s.display_name === ref || `${s.kernel_id}/${s.display_name}` === ref)
}

export async function executeWorkflowNotificationTrigger(args: string[], context: ShellContext, deps: Deps): Promise<ShellCommandResult> {
  if (!context.sessionId) return { ok: false, message: "select a session first" }
  const [action = "list", sourceRef, ...extra] = args
  const usage = "usage: workflow trigger notification list | attach <source> | detach <subscription-id>"
  if (extra.length > 0) return { ok: false, message: usage }
  if (action === "detach") {
    if (!sourceRef) return { ok: false, message: usage }
    await deps.client.send(detachWorkflowNotificationRequest(context.sessionId, sourceRef))
    return { ok: true, message: `detached ${sourceRef}` }
  }
  if (action !== "list" && action !== "attach") return { ok: false, message: usage }
  const response = await deps.client.send(listWorkflowNotificationsRequest(context.sessionId)) as WorkflowNotificationsResponse
  const inventory = response.WorkflowNotifications
  if (!inventory) throw new Error("unexpected notification inventory response")
  if (action === "list") {
    const labels = await notificationKernelLabels(deps.client, new Set(inventory.sources.map(s => s.kernel_id)))
    return { ok: true, message: [
      "Notification sources:",
      ...inventory.sources.map(s => `${s.source_id} ${s.display_name} (${labels.get(s.kernel_id) ?? s.kernel_id})${s.available ? "" : " — kernel offline/source not available"}`),
      "Subscribed to (detach <subscription-id>):",
      ...inventory.subscriptions.map(s => `${s.subscription_id}: ${s.source_id} → ${s.target_kind === "agent" ? `agent ${s.agent_id ?? s.endpoint_id}` : s.publication_id}${s.source_available ? "" : " — source not available"}`),
      ...inventory.diagnostics.map(d => `diagnostic ${d.source_id} ${d.occurrence_id}: ${d.code}`),
    ].join("\n"), data: inventory }
  }
  if (!sourceRef) return { ok: false, message: usage }
  const sources = matchNotificationSource(inventory.sources, sourceRef)
  if (sources.length !== 1) return { ok: false, message: "source missing or ambiguous; use an id from notification list" }
  const source = sources[0]!
  if (!source.available) return { ok: false, message: "source not available: kernel offline" }
  const publicationsResponse = await deps.client.send(listWorkflowPublicationsRequest(context.sessionId))
  const publications = (publicationsResponse.WorkflowPublicationsListed as { publications: WorkflowPublicationDefinition[] }).publications.filter(p => p.enabled && p.kind === "event_based" && p.workflow_id === context.workflowId)
  if (publications.length !== 1) return { ok: false, message: "select a workflow with one enabled notification trigger" }
  const attached = await deps.client.send(attachWorkflowNotificationRequest(context.sessionId, source.source_id, publications[0]!.id)) as WorkflowNotificationAttachedResponse
  if (!attached.WorkflowNotificationAttached) throw new Error("unexpected notification attach response")
  return { ok: true, message: `attached ${source.display_name} as ${attached.WorkflowNotificationAttached.subscription.subscription_id}`, data: attached.WorkflowNotificationAttached }
}
