/** MP-08 / MP-10: one shared attach surface for TUI and shell clients. */
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import type { WorkflowNotificationEvents, WorkflowNotificationsResponse, WorkflowNotificationAttachedResponse, WorkflowNotificationSourceRegisteredResponse, WorkflowPublicationDefinition } from "./kernel-types.js"
import { attachWorkflowNotificationRequest, detachWorkflowNotificationRequest, listWorkflowNotificationsRequest, registerWorkflowNotificationSourceRequest } from "./ipc-workflow-notification-requests.js"
import { listWorkflowPublicationsRequest } from "./ipc-requests.js"
type Deps = { client: { send(request: Record<string, unknown>): Promise<Record<string, unknown>> } }

export async function executeWorkflowNotificationSettings(args: string[], context: ShellContext, deps: Deps): Promise<ShellCommandResult> {
  const [state, ...rest] = args
  if (state !== "on" && state !== "off") return { ok: false, message: "usage: workflow notifications on|off [workflow-ref] [--fields verdict,review_url]" }
  const fieldIndex = rest.indexOf("--fields")
  const fieldList = fieldIndex < 0 ? undefined : rest[fieldIndex + 1]
  if (fieldIndex >= 0 && (!fieldList || fieldList.startsWith("--"))) return { ok: false, message: "usage: --fields needs a comma-separated list, e.g. --fields verdict,review_url" }
  const fields = fieldList === undefined ? null : fieldList.split(",").filter(Boolean)
  const workflow = rest[0]?.startsWith("--") ? context.workflowId : rest[0] ?? context.workflowId
  if (!workflow || !context.sessionId) return { ok: false, message: "select a workflow first" }
  const response = await deps.client.send(registerWorkflowNotificationSourceRequest(context.sessionId, workflow, state === "on", fields))
  const payload = response as WorkflowNotificationSourceRegisteredResponse
  if (!payload.WorkflowNotificationSourceRegistered) throw new Error("unexpected notification source response")
  return { ok: true, message: `Send notifications when runs finish: ${state}`, data: payload.WorkflowNotificationSourceRegistered }
}

export async function executeWorkflowNotificationTrigger(args: string[], context: ShellContext, deps: Deps): Promise<ShellCommandResult> {
  if (!context.sessionId) return { ok: false, message: "select a session first" }
  const [action = "list", sourceRef, ...options] = args
  if (action === "detach") {
    if (!sourceRef) return { ok: false, message: "usage: workflow trigger notification detach <subscription-id>" }
    await deps.client.send(detachWorkflowNotificationRequest(context.sessionId, sourceRef))
    return { ok: true, message: `detached ${sourceRef}` }
  }
  if (action !== "list" && action !== "attach") return { ok: false, message: "usage: workflow trigger notification list|attach|detach" }
  let events: WorkflowNotificationEvents = "success"
  let deliveryMode: "queue" | "inject" = "queue"
  let publication: string | undefined
  let ttl = 7
  const filters: Record<string, unknown> = {}
  for (let i = 0; i < options.length; i++) {
    const option = options[i]
    if (i === 0 && (option === "success" || option === "failure" || option === "both")) { events = option; continue }
    const value = options[++i]
    if (!value) return { ok: false, message: `missing value for ${option}` }
    if (option === "--publication") publication = value
    else if (option === "--ttl") {
      ttl = Number(value)
      if (!Number.isInteger(ttl) || ttl < 1 || ttl > 30) return { ok: false, message: "TTL must be 1–30 days" }
    } else if (option === "--delivery") {
      if (value !== "queue" && value !== "inject") return { ok: false, message: "delivery must be queue or inject" }
      deliveryMode = value
    } else if (option === "--filter") {
      const eq = value.indexOf("=")
      if (eq <= 0) return { ok: false, message: "filter must be field=value (JSON arrays select any-of)" }
      const field = value.slice(0, eq), raw = value.slice(eq + 1)
      try { filters[field] = JSON.parse(raw) as unknown } catch { filters[field] = raw }
    } else return { ok: false, message: `unknown notification option ${option}` }
  }
  const response = await deps.client.send(listWorkflowNotificationsRequest(context.sessionId)) as WorkflowNotificationsResponse
  const inventory = response.WorkflowNotifications
  if (!inventory) throw new Error("unexpected notification inventory response")
  if (action === "list") return { ok: true, message: [
    "My workflows:",
    ...inventory.sources.map(s => `${s.source_id} ${s.name} (${s.kernel_id})${s.available ? "" : " — kernel offline/source not available"}; fields: ${s.fields.join(", ")}`),
    "Subscriptions (detach <subscription-id>):",
    ...inventory.subscriptions.map(s => `${s.subscription_id}: ${s.source_id} → ${s.publication_id} (${s.events}, ${s.delivery_mode}, ${s.ttl_days}d)${s.source_available ? "" : " — source not available"}`),
    ...inventory.diagnostics.map(d => `diagnostic ${d.source_id} ${d.occurrence_id}: ${d.code}`),
  ].join("\n"), data: inventory }
  const sources = inventory.sources.filter(s => s.source_id === sourceRef || s.name === sourceRef || `${s.kernel_id}/${s.name}` === sourceRef)
  if (sources.length !== 1) return { ok: false, message: "source missing or ambiguous; use an id from notification list" }
  const source = sources[0]!
  if (!source.available) return { ok: false, message: "source not available: kernel offline" }
  if (!publication) {
    const publicationsResponse = await deps.client.send(listWorkflowPublicationsRequest(context.sessionId))
    const publications = (publicationsResponse.WorkflowPublicationsListed as { publications: WorkflowPublicationDefinition[] }).publications.filter(p => p.enabled && p.kind === "event_based" && p.workflow_id === context.workflowId)
    if (publications.length !== 1) return { ok: false, message: "select a workflow with one enabled notification trigger, or pass --publication <ref>" }
    publication = publications[0]!.id
  }
  const attached = await deps.client.send(attachWorkflowNotificationRequest(context.sessionId, source.source_id, publication, null, ttl, events, filters, deliveryMode)) as WorkflowNotificationAttachedResponse
  if (!attached.WorkflowNotificationAttached) throw new Error("unexpected notification attach response")
  return { ok: true, message: `attached ${source.name} (${events}, ${deliveryMode}) as ${attached.WorkflowNotificationAttached.subscription.subscription_id}`, data: attached.WorkflowNotificationAttached }
}
