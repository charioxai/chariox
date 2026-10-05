/** MP-08 / MP-10: shared TUI attach projection and offline/filter denial. */
import test from "node:test"
import assert from "node:assert/strict"
import { executeWorkflowNotificationTrigger, executeWorkflowNotificationSettings } from "./shell-workflow-notification-command.js"
import type { ShellContext } from "./shell-core.js"
const context: ShellContext = { sessionId: "target", workflowId: "consumer", workspace: "/tmp", worktree: "/tmp", provider: "dev-stub", model: "", effort: "", variables: {} }
const source = { source_id: "source", kernel_id: "peer", session_id: "source-session", workflow_id: "reviewer", name: "Reviewer", events: "both", available: true, fields: ["repo", "pr", "verdict"] }
test("MP-08 / MP-10: attach uses discovered source and ordinary event trigger with filters", async () => {
  const calls: Record<string, unknown>[] = []
  const deps = { client: { async send(request: Record<string, unknown>) {
    calls.push(request)
    if ("ListWorkflowNotifications" in request) return { WorkflowNotifications: { sources: [source], subscriptions: [], diagnostics: [] } }
    if ("ListWorkflowPublications" in request) return { WorkflowPublicationsListed: { publications: [{ id: "publication", workflow_id: "consumer", enabled: true, kind: "event_based" }] } }
    return { WorkflowNotificationAttached: { subscription: {} } }
  } } }
  const result = await executeWorkflowNotificationTrigger(["attach", "Reviewer", "success", "--filter", "repo=fixture/repo", "--filter", "pr=[873,874]", "--ttl", "30", "--delivery", "inject"], context, deps)
  assert.equal(result.ok, true)
  assert.deepEqual(calls[2], { AttachWorkflowNotification: { session_id: "target", source_id: "source", publication_ref: "publication", queue_ref: null, ttl_days: 30, events: "success", filters: { repo: "fixture/repo", pr: [873, 874] }, delivery_mode: "inject" } })
})
test("MP-08 / MP-10: opaque dotted fields are allowed, offline attach is refused", async () => {
  for (const available of [false, true]) {
    let mutations = 0
    const deps = { client: { async send(request: Record<string, unknown>) {
      if ("AttachWorkflowNotification" in request) { mutations++; return { WorkflowNotificationAttached: { subscription: {} } } }
      return { WorkflowNotifications: { sources: [{ ...source, available }], subscriptions: [], diagnostics: [] } }
    } } }
    const result = await executeWorkflowNotificationTrigger(["attach", "source", "--publication", "p", "--filter", "opaque.value=42"], context, deps)
    assert.equal(result.ok, available); assert.equal(mutations, available ? 1 : 0)
  }
})
test("MP-08 / MP-10: settings switch registers output projection, never emits", async () => {
  const calls: Record<string, unknown>[] = []
  const deps = { client: { async send(request: Record<string, unknown>) { calls.push(request); return { WorkflowNotificationSourceRegistered: { source: {} } } } } }
  assert.equal((await executeWorkflowNotificationSettings(["on", "--fields", "verdict,review_url"], context, deps)).ok, true)
  assert.deepEqual(calls, [{ RegisterWorkflowNotificationSource: { session_id: "target", workflow_ref: "consumer", enabled: true, output_fields: ["verdict", "review_url"] } }])
})
